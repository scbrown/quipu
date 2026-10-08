//! aegis-11rwfs: a constant template is instantiated once, not per solution.

use oxigraph::{
    model::Term,
    sparql::{QueryResults, SparqlEvaluator},
    store::Store as OxStore,
};
use spargebra::{GraphUpdateOperation, Query, SparqlParser};

use super::{Instantiated, instantiate};

const G: &str = "http://ex.org/board";

fn run(ox: &OxStore, update: &str) -> Instantiated {
    let parsed = quipu::sparql_structure::parse_update(SparqlParser::new(), update)
        .unwrap()
        .unwrap();
    let base_iri = parsed.base_iri.clone();
    let Some(GraphUpdateOperation::DeleteInsert {
        delete,
        insert,
        using,
        pattern,
    }) = parsed.operations.into_iter().next()
    else {
        panic!("expected one DELETE/INSERT");
    };
    let select = Query::Select {
        dataset: using,
        pattern: *pattern,
        base_iri,
    };
    let QueryResults::Solutions(solutions) = SparqlEvaluator::new()
        .for_query(select)
        .on_store(ox)
        .execute()
        .unwrap()
    else {
        panic!("WHERE did not evaluate to solutions");
    };
    instantiate(&delete, &insert, solutions).unwrap()
}

/// The shape seeds sends for an update of N seeds (seeds src/native/remote.rs):
/// DELETE every fact of every seed, INSERT the new facts as constants, WHERE a
/// UNION over every fact of every seed. One solution per existing fact.
fn seeds_cas(n: usize, f: usize) -> (OxStore, String) {
    let ox = OxStore::new().unwrap();
    let mut data = String::new();
    let (mut delete, mut insert, mut unions) = (String::new(), String::new(), Vec::new());
    for i in 0..n {
        for j in 0..f {
            data.push_str(&format!(
                "<http://ex.org/s{i}> <http://ex.org/p{j}> \"old\" . "
            ));
            insert.push_str(&format!(
                "<http://ex.org/s{i}> <http://ex.org/p{j}> \"new\" . "
            ));
        }
        delete.push_str(&format!("<http://ex.org/s{i}> ?p{i} ?o{i} . "));
        unions.push(format!("{{ <http://ex.org/s{i}> ?p{i} ?o{i} }}"));
    }
    SparqlEvaluator::new()
        .parse_update(&format!("INSERT DATA {{ GRAPH <{G}> {{ {data} }} }}"))
        .unwrap()
        .on_store(&ox)
        .execute()
        .unwrap();
    let update = format!(
        "DELETE {{ GRAPH <{G}> {{ {delete} }} }} INSERT {{ GRAPH <{G}> {{ {insert} }} }} \
         WHERE {{ GRAPH <{G}> {{ {} }} }}",
        unions.join(" UNION ")
    );
    (ox, update)
}

#[test]
fn a_constant_insert_template_is_built_once_not_per_solution() {
    let (n, f) = (50, 4);
    let (ox, update) = seeds_cas(n, f);
    let out = run(&ox, &update);
    assert_eq!(out.deletes.len(), n * f, "every old fact is deleted");
    assert_eq!(out.inserts.len(), n * f, "every new fact is inserted");
    // n*f solutions. Per-solution instantiation built (n*f)^2 = 40_000 insert
    // quads here; at n=537 that was the production OOM.
    assert_eq!(
        out.built,
        2 * n * f,
        "deletes once per solution, inserts once"
    );
}

#[test]
fn no_solution_inserts_nothing_even_from_a_constant_template() {
    let ox = OxStore::new().unwrap();
    let out = run(
        &ox,
        "INSERT { <http://ex.org/a> <http://ex.org/b> \"c\" } WHERE { <http://ex.org/x> ?p ?o }",
    );
    assert!(out.inserts.is_empty() && out.deletes.is_empty());
}

#[test]
fn template_blank_nodes_stay_fresh_per_solution() {
    let ox = OxStore::new().unwrap();
    let out = run(
        &ox,
        "INSERT { _:b <http://ex.org/v> ?x } WHERE { VALUES ?x { 1 2 3 } }",
    );
    assert_eq!(out.inserts.len(), 3);
    let subjects: std::collections::HashSet<_> =
        out.inserts.iter().map(|q| q.subject.clone()).collect();
    assert_eq!(subjects.len(), 3, "one fresh blank node per solution");
    assert!(
        out.inserts
            .iter()
            .all(|q| matches!(q.object, Term::Literal(_)))
    );
}
