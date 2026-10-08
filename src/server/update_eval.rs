//! SPARQL 1.1 Update evaluation with the specification's DELETE/INSERT order
//! (aegis-odm5yt).
//!
//! Oxigraph 0.5 (spareval 0.2) applies a `DELETE … INSERT … WHERE` one
//! solution at a time: that solution's deletes, then its inserts. A quad one
//! solution inserts is then removed again by a later solution's DELETE, so the
//! result depends on solution order, which is hash order. SPARQL 1.1 Update
//! §3.1.3 defines the result as `(before − deletes) ∪ inserts` over ALL
//! solutions. A seeds-style rewrite (`DELETE { <s> ?p ?o } INSERT { <s> … }
//! WHERE { <s> ?p ?o }`) loses every field it re-inserts unchanged whenever
//! that field's solution comes after another.
//!
//! So `DELETE/INSERT` is evaluated here: the WHERE pattern runs as a SELECT on
//! the same store, both templates are instantiated over every solution, and
//! every delete is applied before any insert. Every other operation (`INSERT
//! DATA`, `DELETE DATA`, `LOAD`, `CLEAR`, …) has no solution sequence and goes
//! to Oxigraph unchanged, one operation at a time, in request order.

use std::collections::{HashMap, HashSet};

use oxigraph::{
    model::{BlankNode, GraphName, NamedNode, NamedOrBlankNode, Quad, Term, Triple},
    sparql::{QueryResults, QuerySolution, SparqlEvaluator},
    store::Store as OxStore,
};
use spargebra::{
    GraphUpdateOperation, Query, SparqlParser, Update,
    term::{
        GraphNamePattern, GroundQuadPattern, GroundTermPattern, GroundTriplePattern,
        NamedNodePattern, QuadPattern, TermPattern, TriplePattern,
    },
};

use super::base::AppError;

fn update_error(e: impl std::fmt::Display) -> AppError {
    quipu::Error::InvalidValue(format!("SPARQL update error: {e}")).into()
}

/// Apply `update` to `ox` with SPARQL 1.1 DELETE/INSERT semantics.
pub(super) fn evaluate(ox: &OxStore, update: &str) -> Result<(), AppError> {
    let parsed = quipu::sparql_structure::parse_update(SparqlParser::new(), update)?
        .map_err(update_error)?;
    let base_iri = parsed.base_iri.clone();
    for operation in parsed.operations {
        match operation {
            GraphUpdateOperation::DeleteInsert {
                delete,
                insert,
                using,
                pattern,
            } => {
                let select = Query::Select {
                    dataset: using,
                    pattern: *pattern,
                    base_iri: base_iri.clone(),
                };
                let QueryResults::Solutions(solutions) = SparqlEvaluator::new()
                    .for_query(select)
                    .on_store(ox)
                    .execute()
                    .map_err(update_error)?
                else {
                    return Err(update_error("WHERE did not evaluate to solutions"));
                };
                let Instantiated {
                    deletes, inserts, ..
                } = instantiate(&delete, &insert, solutions)?;
                for q in &deletes {
                    ox.remove(q).map_err(update_error)?;
                }
                for q in &inserts {
                    ox.insert(q).map_err(update_error)?;
                }
            }
            other => {
                let single = Update {
                    base_iri: base_iri.clone(),
                    operations: vec![other],
                };
                SparqlEvaluator::new()
                    .for_update(single)
                    .on_store(ox)
                    .execute()
                    .map_err(update_error)?;
            }
        }
    }
    Ok(())
}

/// Quads to delete and to insert, over every solution, each set
/// deduplicated (aegis-11rwfs).
///
/// A template quad with no variable and no blank node is the same quad for
/// every solution, so it is instantiated ONCE, when at least one solution
/// exists. Instantiating it per solution made memory O(solutions x template):
/// a seeds CAS update over N seeds has ~N*f solutions (its WHERE unions every
/// fact of every seed) and a constant insert template of ~N*f quads, and at
/// N=537 that OOM-killed the production server (8G + 6.8G swap in 17 s).
pub(super) struct Instantiated {
    pub(super) deletes: HashSet<Quad>,
    pub(super) inserts: HashSet<Quad>,
    /// Quads built before deduplication: the intermediate this bounds.
    /// Read by the regression test only.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) built: usize,
}

pub(super) fn instantiate(
    delete: &[GroundQuadPattern],
    insert: &[QuadPattern],
    solutions: impl IntoIterator<Item = Result<QuerySolution, impl std::fmt::Display>>,
) -> Result<Instantiated, AppError> {
    let (fixed_delete, varying_delete): (Vec<_>, Vec<_>) =
        delete.iter().partition(|q| constant_ground_quad(q));
    let (fixed_insert, varying_insert): (Vec<_>, Vec<_>) =
        insert.iter().partition(|q| constant_quad(q));
    let (mut deletes, mut inserts, mut built) = (HashSet::new(), HashSet::new(), 0);
    let mut first = None;
    for solution in solutions {
        let solution = solution.map_err(update_error)?;
        for q in varying_delete
            .iter()
            .filter_map(|q| ground_quad(q, &solution))
        {
            built += 1;
            deletes.insert(q);
        }
        // Template blank nodes are fresh per solution (§3.1.3).
        let mut bnodes = HashMap::new();
        for q in varying_insert
            .iter()
            .filter_map(|q| quad(q, &solution, &mut bnodes))
        {
            built += 1;
            inserts.insert(q);
        }
        if first.is_none() {
            first = Some(solution);
        }
    }
    if let Some(solution) = first {
        for q in fixed_delete
            .iter()
            .filter_map(|q| ground_quad(q, &solution))
        {
            built += 1;
            deletes.insert(q);
        }
        let mut bnodes = HashMap::new();
        for q in fixed_insert
            .iter()
            .filter_map(|q| quad(q, &solution, &mut bnodes))
        {
            built += 1;
            inserts.insert(q);
        }
    }
    Ok(Instantiated {
        deletes,
        inserts,
        built,
    })
}

fn constant_ground_term(pattern: &GroundTermPattern) -> bool {
    match pattern {
        GroundTermPattern::NamedNode(_) | GroundTermPattern::Literal(_) => true,
        GroundTermPattern::Triple(t) => {
            constant_ground_term(&t.subject)
                && matches!(t.predicate, NamedNodePattern::NamedNode(_))
                && constant_ground_term(&t.object)
        }
        GroundTermPattern::Variable(_) => false,
    }
}

fn constant_ground_quad(pattern: &GroundQuadPattern) -> bool {
    constant_ground_term(&pattern.subject)
        && matches!(pattern.predicate, NamedNodePattern::NamedNode(_))
        && constant_ground_term(&pattern.object)
        && !matches!(pattern.graph_name, GraphNamePattern::Variable(_))
}

/// No variable AND no blank node: a template blank node is fresh per
/// solution, so a quad holding one differs between solutions.
fn constant_term(pattern: &TermPattern) -> bool {
    match pattern {
        TermPattern::NamedNode(_) | TermPattern::Literal(_) => true,
        TermPattern::Triple(t) => {
            constant_term(&t.subject)
                && matches!(t.predicate, NamedNodePattern::NamedNode(_))
                && constant_term(&t.object)
        }
        TermPattern::BlankNode(_) | TermPattern::Variable(_) => false,
    }
}

fn constant_quad(pattern: &QuadPattern) -> bool {
    constant_term(&pattern.subject)
        && matches!(pattern.predicate, NamedNodePattern::NamedNode(_))
        && constant_term(&pattern.object)
        && !matches!(pattern.graph_name, GraphNamePattern::Variable(_))
}

// The instantiation rules below match spareval's (src/update.rs): a quad with
// an unbound variable, a literal or triple subject, a non-IRI predicate, or a
// literal graph name is skipped, never an error.

fn subject(term: Term) -> Option<NamedOrBlankNode> {
    match term {
        Term::NamedNode(n) => Some(n.into()),
        Term::BlankNode(b) => Some(b.into()),
        _ => None,
    }
}

fn predicate(pattern: &NamedNodePattern, solution: &QuerySolution) -> Option<NamedNode> {
    match pattern {
        NamedNodePattern::NamedNode(n) => Some(n.clone()),
        NamedNodePattern::Variable(v) => match solution.get(v)? {
            Term::NamedNode(n) => Some(n.clone()),
            _ => None,
        },
    }
}

fn graph_name(pattern: &GraphNamePattern, solution: &QuerySolution) -> Option<GraphName> {
    match pattern {
        GraphNamePattern::NamedNode(n) => Some(n.clone().into()),
        GraphNamePattern::DefaultGraph => Some(GraphName::DefaultGraph),
        GraphNamePattern::Variable(v) => match solution.get(v)? {
            Term::NamedNode(n) => Some(n.clone().into()),
            Term::BlankNode(b) => Some(b.clone().into()),
            _ => None,
        },
    }
}

fn ground_term(pattern: &GroundTermPattern, solution: &QuerySolution) -> Option<Term> {
    Some(match pattern {
        GroundTermPattern::NamedNode(n) => n.clone().into(),
        GroundTermPattern::Literal(l) => l.clone().into(),
        GroundTermPattern::Triple(t) => ground_triple(t, solution)?.into(),
        GroundTermPattern::Variable(v) => solution.get(v)?.clone(),
    })
}

fn ground_triple(pattern: &GroundTriplePattern, solution: &QuerySolution) -> Option<Triple> {
    Some(Triple::new(
        subject(ground_term(&pattern.subject, solution)?)?,
        predicate(&pattern.predicate, solution)?,
        ground_term(&pattern.object, solution)?,
    ))
}

fn ground_quad(pattern: &GroundQuadPattern, solution: &QuerySolution) -> Option<Quad> {
    Some(Quad::new(
        subject(ground_term(&pattern.subject, solution)?)?,
        predicate(&pattern.predicate, solution)?,
        ground_term(&pattern.object, solution)?,
        graph_name(&pattern.graph_name, solution)?,
    ))
}

fn term(
    pattern: &TermPattern,
    solution: &QuerySolution,
    bnodes: &mut HashMap<BlankNode, BlankNode>,
) -> Option<Term> {
    Some(match pattern {
        TermPattern::NamedNode(n) => n.clone().into(),
        TermPattern::BlankNode(b) => bnodes.entry(b.clone()).or_default().clone().into(),
        TermPattern::Literal(l) => l.clone().into(),
        TermPattern::Triple(t) => triple(t, solution, bnodes)?.into(),
        TermPattern::Variable(v) => solution.get(v)?.clone(),
    })
}

fn triple(
    pattern: &TriplePattern,
    solution: &QuerySolution,
    bnodes: &mut HashMap<BlankNode, BlankNode>,
) -> Option<Triple> {
    Some(Triple::new(
        subject(term(&pattern.subject, solution, bnodes)?)?,
        predicate(&pattern.predicate, solution)?,
        term(&pattern.object, solution, bnodes)?,
    ))
}

fn quad(
    pattern: &QuadPattern,
    solution: &QuerySolution,
    bnodes: &mut HashMap<BlankNode, BlankNode>,
) -> Option<Quad> {
    Some(Quad::new(
        subject(term(&pattern.subject, solution, bnodes)?)?,
        predicate(&pattern.predicate, solution)?,
        term(&pattern.object, solution, bnodes)?,
        graph_name(&pattern.graph_name, solution)?,
    ))
}

#[cfg(test)]
#[path = "update_eval_tests.rs"]
mod tests;
