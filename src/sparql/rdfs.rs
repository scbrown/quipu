//! RDFS type-hierarchy helpers — subclass inference for rdf:type queries.

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

use super::Bindings;
use super::TemporalContext;
use super::pattern::bind_var;

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

/// One type constant in a query that subclass expansion widened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedType {
    /// The IRI written in the query.
    pub type_iri: String,
    /// The subclasses folded in, transitively. Never empty — a type with no
    /// subclasses is not reported, because nothing about its count is inferred.
    pub subclasses: Vec<String>,
}

/// Which type constants in `query` will be widened by subclass inference?
///
/// # Why this exists
///
/// `?s a <T>` and `?s a ?t . FILTER(?t = <T>)` express the same constraint and
/// return DIFFERENT counts: the constant form is expanded over `rdfs:subClassOf`
/// (see [`eval_type_pattern_with_subclasses`]), the variable form is matched
/// literally. Both answers are legitimate and both are needed — asserted-only is
/// the right basis for a vocabulary census or governance gating, inferred is the
/// right basis for blast radius — so neither semantics is a bug and neither may
/// be silently removed.
///
/// What WAS a bug is that the response could not tell you which question it had
/// answered. Both return HTTP 200, both numbers are individually plausible, and
/// nothing distinguishes them. That silence produced FOUR independent
/// measurement errors by four people; one shipped into a governance argument and
/// sized a decision several times too large, and it also CONCEALED a real
/// finding, because an ungoverned subclass was swallowed by its inflated parent
/// count. A footgun that catches four careful readers is a product defect, and
/// the fix is a flag rather than a semantics change.
///
/// # Accuracy
///
/// Gated on exactly the conditions the evaluator uses, so the marker cannot
/// disagree with the numbers it annotates: default-graph only (inside a named
/// GRAPH a type pattern is matched literally), rdf:type predicate, constant IRI
/// object. A type whose subclass set is just itself is omitted — its count is
/// asserted-only and saying otherwise would train readers to ignore the field.
///
/// It reports that expansion WAS APPLIED, which is a fact about the query. It
/// does not promise the extra classes contributed rows; that is a second
/// question, and answering it would require running the query twice.
pub fn expanded_types(store: &Store, query: &str, ctx: &TemporalContext) -> Vec<ExpandedType> {
    use spargebra::SparqlParser;

    if !ctx.graph.is_root_default() {
        return Vec::new();
    }
    let Ok(parsed) = SparqlParser::new().parse_query(query) else {
        return Vec::new();
    };

    let mut out: Vec<ExpandedType> = Vec::new();
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
            out.push(ExpandedType {
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

/// Evaluate a `?x a <Class>` pattern with subclass expansion.
pub fn eval_type_pattern_with_subclasses(
    store: &Store,
    tp: &TriplePattern,
    bindings: &Bindings,
    class_ids: &[i64],
    ctx: &TemporalContext,
) -> Result<Vec<Bindings>> {
    let Some(type_pred_id) = store.lookup(RDF_TYPE)? else {
        return Ok(vec![]);
    };

    // Subject filter (if bound).
    let subject_id =
        if let Some(iri) = super::pattern::resolve_subject_pattern(&tp.subject, bindings) {
            match store.lookup(&iri)? {
                Some(id) => Some(id),
                None => return Ok(vec![]),
            }
        } else {
            None
        };

    let mut results = Vec::new();

    // For each class in the hierarchy, find instances.
    for class_id in class_ids {
        let v_bytes = Value::Ref(*class_id).to_bytes();

        // Build SQL and params dynamically.
        let mut conditions = vec!["a = ?1".to_string(), "v = ?2".to_string()];
        let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params_vec.push(Box::new(type_pred_id));
        params_vec.push(Box::new(v_bytes.clone()));

        if let Some(sid) = subject_id {
            conditions.push(format!("e = ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(sid));
        }
        conditions.push("op = 1".to_string());
        conditions.push("g = 0".to_string()); // committed reads are ROOT-scoped (#36)
        if let Some(vt) = &ctx.valid_at {
            conditions.push(format!("valid_from <= ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(vt.clone()));
            conditions.push(format!(
                "(valid_to IS NULL OR valid_to > ?{})",
                params_vec.len()
            ));
        } else {
            conditions.push("valid_to IS NULL".to_string());
        }
        if let Some(tx) = ctx.as_of_tx {
            conditions.push(format!("tx <= ?{}", params_vec.len() + 1));
            params_vec.push(Box::new(tx));
        }

        // DISTINCT: dedup duplicate current (e,v) rows from re-asserted facts so
        // `?x a Class` type matches don't multiply under joins/COUNT (GH#13).
        let sql = format!(
            "SELECT DISTINCT e, v FROM facts WHERE {}",
            conditions.join(" AND ")
        );
        let mut stmt = store.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(std::convert::AsRef::as_ref).collect();
        let mut rows = stmt.query(param_refs.as_slice())?;

        while let Some(row) = rows.next()? {
            let e_id: i64 = row.get(0)?;
            let v_bytes_row: Vec<u8> = row.get(1)?;
            let v = Value::from_bytes(&v_bytes_row)?;

            let mut new_bindings = bindings.clone();
            let mut compatible = true;

            // Bind subject variable.
            if let TermPattern::Variable(var) = &tp.subject {
                let e_iri = store.resolve(e_id)?;
                let e_val = if let Some(term_id) = store.lookup(&e_iri)? {
                    Value::Ref(term_id)
                } else {
                    Value::Str(e_iri)
                };
                if !bind_var(&mut new_bindings, var.as_str(), e_val, &mut compatible) {
                    continue;
                }
            }

            // Bind object variable (always the matched class).
            if let TermPattern::Variable(var) = &tp.object
                && !bind_var(&mut new_bindings, var.as_str(), v, &mut compatible)
            {
                continue;
            }

            if compatible {
                results.push(new_bindings);
            }
        }
    }

    Ok(results)
}
