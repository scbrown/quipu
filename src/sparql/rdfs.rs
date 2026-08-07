//! RDFS type-hierarchy helpers and the asserted-only migration marker.

use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

use super::TemporalContext;

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

/// One type constant whose former subclass expansion is now withheld.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithheldType {
    /// The IRI written in the query.
    pub type_iri: String,
    /// The subclasses folded in, transitively. Never empty — a type with no
    /// subclasses is not reported, because nothing about its count is inferred.
    pub subclasses: Vec<String>,
}

/// Which type constants in `query` would have been widened before asserted-only?
///
/// # Why this exists
///
/// Before the migration, `?s a <T>` expanded over `rdfs:subClassOf`, while
/// `?s a ?t . FILTER(?t = <T>)` matched literally. The constant form is now
/// asserted-only too, per SPARQL simple entailment. The inferred question remains
/// available explicitly as `?s a/rdfs:subClassOf* <T>`.
///
/// The semantic flip would itself silently change counts, so the response names
/// only the constants for which expansion was withheld. The marker is omitted for
/// all unaffected traffic; presence remains the signal.
///
/// # Accuracy
///
/// Gated on exactly the former evaluator conditions: default graph only,
/// rdf:type predicate, constant IRI object. A leaf type is omitted because the
/// flip does not change its answer.
///
pub fn withheld_types(store: &Store, query: &str, ctx: &TemporalContext) -> Vec<WithheldType> {
    use spargebra::SparqlParser;

    if !ctx.graph.is_root_default() {
        return Vec::new();
    }
    let Ok(parsed) = SparqlParser::new().parse_query(query) else {
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
