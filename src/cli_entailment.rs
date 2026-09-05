//! `--entailment <regime>` for `quipu query` (aegis-1gp76j).
//!
//! Composing the dataset here is what an entailment regime MEANS — a claim
//! about what the default graph ENTAILS — so the closure belongs in the
//! default graph for the duration of the answer. The alternative, rewriting
//! the query to add `FROM <…> FROM <…#inferred>`, would make the answer
//! describe a query the caller did not ask; for a conformance suite that is
//! the runner scoring itself.

use quipu::{GraphScope, Store};

/// Honour `--entailment` if present, returning the scope the query should run over.
///
/// Absent the flag this is the identity function, so the simple-entailment path
/// is unchanged and pays nothing.
pub fn apply(args: &[String], store: &mut Store, graph: GraphScope) -> GraphScope {
    let Some(regime) = crate::cli::flag_value(args, "--entailment") else {
        return graph;
    };
    if !regime.eq_ignore_ascii_case("rdfs") {
        eprintln!("error: unknown entailment regime {regime:?}; expected \"rdfs\"");
        std::process::exit(1);
    }

    let base = match &graph {
        GraphScope::Default(ids) if !ids.is_empty() => ids[0],
        _ => quipu::schema::ROOT_GRAPH,
    };

    let timestamp = crate::cli::chrono_now();
    if let Err(e) = quipu::sparql::rdfs_closure::materialise(store, base, &timestamp) {
        eprintln!("error materialising RDFS closure: {e}");
        std::process::exit(1);
    }

    match store.companion_inferred_iri(base) {
        Ok(iri) => match store.lookup(&iri) {
            Ok(Some(companion)) => GraphScope::Default(vec![base, companion]),
            _ => GraphScope::Default(vec![base]),
        },
        Err(e) => {
            eprintln!("error resolving companion graph: {e}");
            std::process::exit(1);
        }
    }
}
