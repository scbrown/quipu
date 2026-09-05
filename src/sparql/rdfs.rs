//! RDFS type-hierarchy evaluation and inference reporting helpers.

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

use super::pattern_util::bind_var;
use super::{Bindings, TemporalContext};

pub const RDF_TYPE: &str = namespace::RDF_TYPE;
const RDFS_SUBCLASS_OF: &str = namespace::RDFS_SUBCLASS_OF;

/// Check if a triple pattern has rdf:type as predicate and a concrete class as object.
pub fn is_rdf_type_pattern(tp: &TriplePattern) -> bool {
    matches!(&tp.predicate, NamedNodePattern::NamedNode(n) if n.as_str() == RDF_TYPE)
        && matches!(&tp.object, TermPattern::NamedNode(_))
}

/// Collect a class and all its subclasses (transitive) from the fact log.
///
/// Uses rdfs:subClassOf triples: `SubClass rdfs:subClassOf SuperClass`.
/// Returns the term IDs of the class and all subclasses.
pub fn collect_class_and_subclasses(store: &Store, class_iri: &str) -> Result<Vec<i64>> {
    let Some(class_id) = store.lookup(class_iri)? else {
        return Ok(vec![]);
    };

    let Some(subclass_pred) = store.lookup(RDFS_SUBCLASS_OF)? else {
        return Ok(vec![class_id]); // No subClassOf pred -> just the class itself
    };

    // BFS to find all subclasses.
    let mut result = vec![class_id];
    let mut frontier = vec![class_id];

    while !frontier.is_empty() {
        let mut next_frontier = Vec::new();
        for super_id in &frontier {
            // Find all X where X rdfs:subClassOf super_id (as a Ref value)
            let target_bytes = Value::Ref(*super_id).to_bytes();
            let mut stmt = store.prepare(
                "SELECT e FROM facts WHERE a = ?1 AND v = ?2 AND op = 1 AND g = 0 AND valid_to IS NULL",
            )?;
            let mut rows = stmt.query(rusqlite::params![subclass_pred, target_bytes])?;
            while let Some(row) = rows.next()? {
                let sub_id: i64 = row.get(0)?;
                if !result.contains(&sub_id) {
                    result.push(sub_id);
                    next_frontier.push(sub_id);
                }
            }
        }
        frontier = next_frontier;
    }

    Ok(result)
}

/// Evaluate a constant rdf:type pattern over the class and all subclasses.
pub fn eval_type_pattern_with_subclasses(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    class_ids: &[i64],
    ctx: &TemporalContext,
    limit: Option<usize>,
) -> Result<Vec<Bindings>> {
    let Some(type_pred_id) = store.lookup(RDF_TYPE)? else {
        return Ok(vec![]);
    };
    let subject_ids =
        if let Some(iri) = super::pattern_util::resolve_subject_pattern(&tp.subject, bindings) {
            let ids = store.lookup_all(&iri)?;
            if ids.is_empty() {
                return Ok(vec![]);
            }
            Some(ids)
        } else {
            None
        };
    let mut results = Vec::new();
    // An entity may assert both a subclass and its superclass. Entailment
    // produces one graph triple, not one row per proof path.
    let mut seen_entities = std::collections::HashSet::new();
    for class_id in class_ids {
        let mut conditions = vec![
            "a = ?1".to_string(),
            "v = ?2".to_string(),
            "op = 1".to_string(),
            "g = 0".to_string(),
        ];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
            Box::new(type_pred_id),
            Box::new(Value::Ref(*class_id).to_bytes()),
        ];
        if let Some(ids) = &subject_ids {
            conditions.push(super::sql_in::sql_id_in("e", ids, &mut params));
        }
        if let Some(vt) = &ctx.valid_at {
            conditions.push(format!("valid_from <= ?{}", params.len() + 1));
            params.push(Box::new(vt.clone()));
            conditions.push(format!(
                "(valid_to IS NULL OR valid_to > ?{})",
                params.len()
            ));
        } else if let Some(tx) = ctx.as_of_tx {
            conditions.push(format!(
                "(valid_to IS NULL OR retracted_tx > ?{})",
                params.len() + 1
            ));
            params.push(Box::new(tx));
        } else {
            conditions.push("valid_to IS NULL".to_string());
        }
        if let Some(tx) = ctx.as_of_tx {
            conditions.push(format!("tx <= ?{}", params.len() + 1));
            params.push(Box::new(tx));
        }
        let sql = format!(
            "SELECT DISTINCT e, v FROM facts WHERE {}",
            conditions.join(" AND ")
        );
        let refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(std::convert::AsRef::as_ref).collect();
        let mut stmt = store.prepare(&sql)?;
        let mut rows = stmt.query(refs.as_slice())?;
        while let Some(row) = rows.next()? {
            let mut next = bindings.clone();
            let mut compatible = true;
            let entity: i64 = row.get(0)?;
            let canonical = store.canonical_id(entity)?;
            if !seen_entities.insert(canonical) {
                continue;
            }
            if let TermPattern::Variable(var) = &tp.subject {
                bind_var(
                    &mut next,
                    var.as_str(),
                    Value::Ref(canonical),
                    &mut compatible,
                );
            }
            if compatible {
                results.push(next);
            }
            if limit.is_some_and(|cap| results.len() >= cap) {
                return Ok(results);
            }
        }
    }
    Ok(results)
}

/// One type constant whose query expands over subclasses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithheldType {
    /// The IRI written in the query.
    pub type_iri: String,
    /// The subclasses folded in, transitively. Never empty — a type with no
    /// subclasses is not reported, because nothing about its count is inferred.
    pub subclasses: Vec<String>,
}

/// Which type constants in `query` are widened by subclass entailment?
///
/// # Why this exists
///
/// `?s a <T>` expands over `rdfs:subClassOf`, while `?s a ?t . FILTER(?t =
/// <T>)` remains a literal asserted-only census. The response marker names the
/// constants whose subclasses were included; it is omitted when no expansion
/// exists, so presence remains the signal.
///
/// # Accuracy
///
/// Gated on exactly the evaluator conditions: default graph only,
/// rdf:type predicate, constant IRI object. A leaf type is omitted because the
/// flip does not change its answer.
///
pub fn withheld_types(store: &Store, query: &str, ctx: &TemporalContext) -> Vec<WithheldType> {
    // Gated on exactly the evaluator's condition in `triple.rs`, INCLUDING the
    // entailment-regime case (aegis-g6bu6d). These two gates must agree: if the
    // evaluator expands and this does not, the answer is inferred and carries no
    // marker saying so — the silent direction this marker exists to end.
    if !(ctx.graph.is_root_default() || (ctx.entails_rdfs && ctx.graph.includes_root_default())) {
        return Vec::new();
    }
    let Ok(parsed) = super::sparql_parser().parse_query(query) else {
        return Vec::new();
    };

    let mut out: Vec<WithheldType> = Vec::new();
    for iri in type_constants(&parsed) {
        if out.iter().any(|e| e.type_iri == iri) {
            continue;
        }
        let Ok(ids) = collect_class_and_subclasses(store, &iri) else {
            continue;
        };
        // ids[0] is the class itself; anything beyond it is inference.
        if ids.len() <= 1 {
            continue;
        }
        let subclasses: Vec<String> = ids[1..]
            .iter()
            .filter_map(|id| store.resolve(*id).ok())
            .collect();
        if !subclasses.is_empty() {
            out.push(WithheldType {
                type_iri: iri,
                subclasses,
            });
        }
    }
    out
}

/// Every constant IRI standing in the object of an `rdf:type` pattern.
///
/// Walks the algebra rather than the surface syntax so a type constant is found
/// wherever it occurs — inside OPTIONAL, UNION, a subquery, a FILTER EXISTS.
/// Anything not recognised simply contributes nothing: a missed pattern costs a
/// marker, never a wrong one.
fn type_constants(query: &spargebra::Query) -> Vec<String> {
    use spargebra::algebra::GraphPattern;

    // The wildcard arm below is DELIBERATE and must stay a wildcard.
    //
    // `GraphPattern`'s variant set depends on which spargebra features are
    // enabled, so the set differs per build: under `--no-default-features` the
    // wildcard covers exactly one variant (`Values`) and clippy's
    // `match_wildcard_for_single_variants` fires, while richer feature
    // combinations leave several. Naming the variant explicitly to satisfy the
    // lint would make the match NON-EXHAUSTIVE on every other leg — trading a
    // lint on one build for a compile error on the rest.
    //
    // What the wildcard actually absorbs is patterns that carry no triple
    // patterns to walk (`VALUES` supplies inline data). The residual risk is
    // real and worth stating: a FUTURE spargebra variant that does contain type
    // constants would be silently skipped, and this function would under-report
    // rather than fail. It is allowed to under-report — see this function's
    // contract above: a missing marker, never a wrong one.
    #[allow(clippy::match_wildcard_for_single_variants)]
    fn walk(p: &GraphPattern, out: &mut Vec<String>) {
        match p {
            GraphPattern::Bgp { patterns } => {
                for tp in patterns {
                    if is_rdf_type_pattern(tp)
                        && let TermPattern::NamedNode(n) = &tp.object
                    {
                        out.push(n.as_str().to_string());
                    }
                }
            }
            GraphPattern::Path { .. } => {}
            GraphPattern::Join { left, right }
            | GraphPattern::LeftJoin { left, right, .. }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right } => {
                walk(left, out);
                walk(right, out);
            }
            GraphPattern::Filter { inner, .. }
            | GraphPattern::Extend { inner, .. }
            | GraphPattern::OrderBy { inner, .. }
            | GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::Group { inner, .. }
            | GraphPattern::Service { inner, .. } => walk(inner, out),
            // GRAPH <g> { … }: the evaluator matches type patterns LITERALLY in a
            // named graph, so nothing inside one is expanded and nothing inside
            // one may be reported.
            GraphPattern::Graph { .. } => {}
            _ => {}
        }
    }

    let mut out = Vec::new();
    match query {
        spargebra::Query::Select { pattern, .. }
        | spargebra::Query::Construct { pattern, .. }
        | spargebra::Query::Describe { pattern, .. }
        | spargebra::Query::Ask { pattern, .. } => walk(pattern, &mut out),
    }
    out
}
