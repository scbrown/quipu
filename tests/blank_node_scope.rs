//! Document-scoped identity across loads, graphs, and chunk boundaries.
use oxrdfio::RdfFormat;
use quipu::{Store, Value};
use std::collections::BTreeSet;

fn load(store: &mut Store, text: &str, graph: i64, scope: Option<&str>, chunk: usize) {
    quipu::rdf::ingest_rdf_chunked_with_scope(
        store,
        text.as_bytes(),
        RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
        graph,
        chunk,
        scope,
    )
    .unwrap();
}
fn nodes(store: &Store, graph: i64) -> BTreeSet<String> {
    let mut result = BTreeSet::new();
    for f in store.current_facts_in_graph(graph).unwrap() {
        for id in [
            Some(f.entity),
            if let Value::Ref(id) = f.value {
                Some(id)
            } else {
                None
            },
        ]
        .into_iter()
        .flatten()
        {
            let s = store.resolve(id).unwrap();
            if s.starts_with("_:") {
                result.insert(s);
            }
        }
    }
    result
}
const DOC: &str = "_:x <http://e/p> [ <http://e/p> [ <http://e/p> (\"a\" \"b\") ] ] .\n_:x <http://e/q> \"tail\" .\n";

#[test]
fn same_document_is_idempotent_including_anonymous_nodes_and_chunk_splits() {
    let mut store = Store::open_in_memory().unwrap();
    load(&mut store, DOC, 0, None, 1);
    let first = nodes(&store, 0);
    let exported =
        String::from_utf8(quipu::export_rdf(&store, RdfFormat::NTriples).unwrap()).unwrap();
    let expected = include_str!("fixtures/blank-node-scope-golden.nt");
    assert_eq!(
        exported.lines().collect::<BTreeSet<_>>(),
        expected.lines().collect::<BTreeSet<_>>()
    );
    assert_eq!(first.len(), 5);
    let count = store.current_facts().unwrap().len();
    assert_eq!(count, 8);
    load(&mut store, DOC, 0, None, 3);
    assert_eq!(nodes(&store, 0), first);
    assert_eq!(store.current_facts().unwrap().len(), count);
    // Stable encounter ordering is part of the persisted identity contract.
    let q = "SELECT ?s ?p ?o WHERE {?s ?p ?o}";
    assert_eq!(quipu::sparql::query(&store, q).unwrap().rows().len(), count);
}

#[test]
fn separate_documents_and_graphs_do_not_join_but_iris_still_do() {
    let mut store = Store::open_in_memory().unwrap();
    let a = "_:b0 <http://e/p> \"one\" . <http://e/s> <http://e/p> \"one\" .";
    let b = "_:b0 <http://e/p> \"two\" . <http://e/s> <http://e/p> \"two\" .";
    load(&mut store, a, 0, None, 1);
    load(&mut store, b, 0, None, 1);
    assert_eq!(nodes(&store, 0).len(), 2);
    let g1 = store.graph_create("http://e/g1").unwrap();
    let g2 = store.graph_create("http://e/g2").unwrap();
    load(&mut store, a, g1, None, 1);
    load(&mut store, a, g2, None, 1);
    assert!(nodes(&store, g1).is_disjoint(&nodes(&store, g2)));
    let rows=quipu::sparql::query(&store,"SELECT ?s WHERE { GRAPH <http://e/g1> {?s <http://e/p> ?o} GRAPH <http://e/g2> {?s <http://e/p> ?v} }").unwrap();
    assert_eq!(rows.rows().len(), 1, "only the named IRI joins");
}

#[test]
fn override_can_split_identical_loads_or_share_across_graphs() {
    let mut store = Store::open_in_memory().unwrap();
    load(&mut store, DOC, 0, Some("first"), 1);
    load(&mut store, DOC, 0, Some("second"), 1);
    assert_eq!(nodes(&store, 0).len(), 10);
    assert_eq!(store.current_facts().unwrap().len(), 16);
    let g = store.graph_create("http://e/g").unwrap();
    load(&mut store, DOC, g, Some("first"), 2);
    assert_eq!(nodes(&store, g).len(), 5);
    assert!(nodes(&store, g).is_subset(&nodes(&store, 0)));
}

#[test]
fn declared_ingest_keeps_one_map_and_reloads_without_duplicates() {
    use sha2::{Digest, Sha256};
    let mut store = Store::open_in_memory().unwrap();
    let g = store.graph_create("http://e/declared").unwrap();
    let declaration = quipu::LoadDeclaration {
        triples: 8,
        sha256: format!("{:x}", Sha256::digest(DOC.as_bytes())),
    };
    for chunk in [1, 3] {
        quipu::rdf::ingest_rdf_declared_with_scope(
            &mut store,
            DOC.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
            g,
            chunk,
            &declaration,
            Some("load"),
        )
        .unwrap();
        assert_eq!(nodes(&store, g).len(), 5);
        assert_eq!(store.current_facts_in_graph(g).unwrap().len(), 11);
    }
}

#[cfg(feature = "shacl")]
#[test]
fn public_cli_scope_override_separates_repeat_loads() {
    use std::process::Command;
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("input.ttl");
    std::fs::write(&file, DOC).unwrap();
    let db = temp.path().join("db.sqlite");
    for scope in ["a", "a", "b"] {
        let out = Command::new(env!("CARGO_BIN_EXE_quipu"))
            .args([
                "knot",
                file.to_str().unwrap(),
                "--db",
                db.to_str().unwrap(),
                "--blank-node-scope",
                scope,
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let store = Store::open(db.to_str().unwrap()).unwrap();
    assert_eq!(nodes(&store, 0).len(), 10);
    assert_eq!(store.current_facts().unwrap().len(), 16);
}

#[test]
fn knot_snapshot_and_append_share_the_same_scope_contract() {
    let mut store = Store::open_in_memory().unwrap();
    for replace in [false, true, true] {
        let result = quipu::mcp::tool_knot(
            &mut store,
            &serde_json::json!({
                "turtle":DOC,"blank_node_scope":"shared","snapshot":"test",
                "replace_snapshot":replace
            }),
        )
        .unwrap();
        assert_eq!(result["conforms"], true);
        assert_eq!(nodes(&store, 0).len(), 5);
        assert_eq!(store.current_facts().unwrap().len(), 8);
    }
}
