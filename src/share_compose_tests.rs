use super::*;
use crate::share::{ShareDestination, ShareOptions};
use crate::types::Op;

const TS: &str = "2026-09-24T00:00:00Z";
const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
<urn:PersonShape> a sh:NodeShape; sh:targetClass <urn:Person>;
 sh:property [ sh:path <urn:name>; sh:minCount 1; sh:maxCount 1 ] .
"#;

fn pack(data: &str, shapes: &str) -> ShareImportRequest {
    let mut source = Store::open_in_memory().unwrap();
    crate::rdf::ingest_rdf(
        &mut source,
        data.as_bytes(),
        RdfFormat::Turtle,
        None,
        TS,
        None,
        None,
    )
    .unwrap();
    source.load_shapes("test", shapes, TS).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pack");
    crate::share::share(
        &source,
        path.to_str().unwrap(),
        &ShareOptions {
            destination: ShareDestination::Internal,
            ..Default::default()
        },
    )
    .unwrap();
    let mut request = crate::share_transport::read_local(path.to_str().unwrap()).unwrap();
    request.destination = ShareDestination::Internal;
    request
}

#[test]
fn validates_union_preserves_membership_and_never_promotes_root() {
    let a = pack("<urn:alice> a <urn:Person> .", SHAPES);
    let b = pack("<urn:alice> <urn:name> \"Alice\" .", SHAPES);
    let mut store = Store::open_in_memory().unwrap();
    let result = compose(&mut store, &[a, b], None, TS, None).unwrap();
    assert_eq!(result.outcome, "composed");
    assert!(store.current_facts().unwrap().is_empty());
    let e = store.lookup("urn:alice").unwrap().unwrap();
    for pack in &result.packs {
        let g = store.lookup(&pack.graph).unwrap().unwrap();
        assert_eq!(store.current_facts_in_graph(g).unwrap()[0].entity, e);
    }
    let query = format!(
        "SELECT ?name FROM <{}> WHERE {{ <urn:alice> a <urn:Person>; <urn:name> ?name }}",
        result.dataset
    );
    let rows = crate::sparql_query(&store, &query).unwrap();
    assert_eq!(rows.rows().len(), 1, "the join requires both packs");
}

#[test]
fn conflicting_shapes_refuse_without_mutation_and_explicit_authority_works() {
    let a = pack("<urn:alice> a <urn:Person> .", SHAPES);
    let b = pack("<urn:alice> <urn:name> \"Alice\" .", "");
    let mut store = Store::open_in_memory().unwrap();
    assert!(
        compose(&mut store, &[a.clone(), b.clone()], None, TS, None)
            .unwrap_err()
            .to_string()
            .contains("shape conflict")
    );
    assert!(store.dataset_list().unwrap().is_empty());
    let result = compose(&mut store, &[a, b], Some(0), TS, None).unwrap();
    assert_eq!(result.outcome, "composed");
    assert!(
        store.list_shapes().unwrap().is_empty(),
        "foreign policy never becomes global policy"
    );
}

#[test]
fn nonconforming_union_is_retained_only_in_explicit_dataset() {
    let a = pack("<urn:alice> a <urn:Person> .", SHAPES);
    let mut store = Store::open_in_memory().unwrap();
    let result = compose(&mut store, &[a], None, TS, None).unwrap();
    assert_eq!(result.outcome, "quarantined");
    assert_eq!(result.validation["violations"], 1);
    assert!(store.current_facts().unwrap().is_empty());
    assert_eq!(store.dataset_members(&result.dataset).unwrap().len(), 1);
}

#[test]
fn reloading_snapshot_does_not_resurrect_and_validates_actual_local_union() {
    let a = pack("<urn:alice> a <urn:Person>; <urn:name> \"Alice\" .", SHAPES);
    let mut store = Store::open_in_memory().unwrap();
    let first = compose(&mut store, std::slice::from_ref(&a), None, TS, None).unwrap();
    let g = store.lookup(&first.packs[0].graph).unwrap().unwrap();
    let name = store.lookup("urn:name").unwrap().unwrap();
    let f = store
        .current_facts_in_graph(g)
        .unwrap()
        .into_iter()
        .find(|f| f.attribute == name)
        .unwrap();
    store
        .transact_to_graph(
            &[crate::store::Datum {
                entity: f.entity,
                attribute: f.attribute,
                value: f.value,
                valid_from: TS.into(),
                valid_to: None,
                op: Op::Retract,
            }],
            TS,
            None,
            None,
            g,
        )
        .unwrap();
    let second = compose(&mut store, &[a], None, TS, None).unwrap();
    assert_eq!(first.dataset, second.dataset);
    assert_eq!(
        second.outcome, "quarantined",
        "validate current facts, not the stale input bytes"
    );
    assert_eq!(store.current_facts_in_graph(g).unwrap().len(), 1);
}

#[test]
fn blank_nodes_are_pack_local_and_a_corrupt_second_pack_is_atomic() {
    let a = pack("_:a <urn:p> \"one\" .", "");
    let b = pack("_:a <urn:p> \"two\" .", "");
    let mut store = Store::open_in_memory().unwrap();
    let result = compose(&mut store, &[a.clone(), b.clone()], None, TS, None).unwrap();
    let entities: BTreeSet<_> = result
        .packs
        .iter()
        .map(|p| {
            let g = store.lookup(&p.graph).unwrap().unwrap();
            store.current_facts_in_graph(g).unwrap()[0].entity
        })
        .collect();
    assert_eq!(entities.len(), 2);
    let mut bad = b;
    bad.export_ntriples.push_str("broken");
    let mut fresh = Store::open_in_memory().unwrap();
    assert!(compose(&mut fresh, &[a, bad], None, TS, None).is_err());
    assert!(fresh.dataset_list().unwrap().is_empty());
    assert!(fresh.lookup(&result.packs[0].graph).unwrap().is_none());
}

#[test]
fn explicit_alias_links_resolve_across_packs_without_label_based_merging() {
    let a = pack(
        r#"<urn:canonical> a <urn:Person>; <urn:name> "Alice" .
        <urn:unrelated> <urn:name> "Alice" ."#,
        SHAPES,
    );
    let b = pack(
        r#"<urn:alias> <http://www.w3.org/2002/07/owl#sameAs> <urn:canonical>;
        <urn:email> "alice@example.org" ."#,
        SHAPES,
    );
    let mut store = Store::open_in_memory().unwrap();
    let result = compose(&mut store, &[a, b], None, TS, None).unwrap();
    let query = format!(
        r#"SELECT DISTINCT ?canonical ?email FROM <{}> WHERE {{
        ?alias <http://www.w3.org/2002/07/owl#sameAs>+ ?canonical; <urn:email> ?email .
        ?canonical a <urn:Person> . }}"#,
        result.dataset
    );
    assert_eq!(crate::sparql_query(&store, &query).unwrap().rows().len(), 1);
    assert_ne!(
        store.lookup("urn:canonical").unwrap(),
        store.lookup("urn:unrelated").unwrap()
    );
    let alias = store.lookup("urn:alias").unwrap().unwrap();
    let graph = store.lookup(&result.packs[1].graph).unwrap().unwrap();
    assert!(
        store
            .current_facts_in_graph(graph)
            .unwrap()
            .iter()
            .any(|f| f.entity == alias),
        "source identity survives, rather than being irreversibly rewritten"
    );
}

#[test]
fn late_graph_conflict_rolls_back_the_first_pack() {
    let a = pack("<urn:a> <urn:p> \"a\" .", "");
    let b = pack("<urn:b> <urn:p> \"b\" .", "");
    let mut store = Store::open_in_memory().unwrap();
    let first = format!("urn:quipu:composition:pack:{}", &a.manifest.share_id[7..]);
    let second = format!("urn:quipu:composition:pack:{}", &b.manifest.share_id[7..]);
    store.overlay_create(&second, 0).unwrap();
    assert!(compose(&mut store, &[a, b], None, TS, None).is_err());
    assert!(store.lookup(&first).unwrap().is_none());
    assert!(store.dataset_list().unwrap().is_empty());
    assert!(store.current_facts().unwrap().is_empty());
}
