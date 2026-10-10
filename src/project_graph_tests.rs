use super::*;

const TS: &str = "2026-09-30T00:00:00Z";

#[test]
fn project_id_validation() {
    for good in ["demo", "quipu", "a.b-c_d", "X1"] {
        assert!(validate_project_id(good).is_ok(), "{good}");
    }
    for bad in ["", ".hidden", "a/b", "a b", "a:b", "../x", &"x".repeat(129)] {
        assert!(validate_project_id(bad).is_err(), "{bad:?}");
    }
    assert_eq!(project_iri("demo"), "urn:quipu:project:demo");
}

#[test]
fn scaffold_writes_the_ignore_file_and_settles_the_id() {
    let root = tempfile::tempdir().unwrap();
    let err = scaffold(root.path(), None).unwrap_err().to_string();
    assert!(err.contains("no project id"), "{err}");
    // The refusal still wrote the ignore file: a store created before the id
    // is named is protected from the first `git add`.
    assert_eq!(
        std::fs::read_to_string(root.path().join(".quipu/.gitignore")).unwrap(),
        GITIGNORE
    );
    assert_eq!(scaffold(root.path(), Some("demo")).unwrap(), "demo");
    assert_eq!(scaffold(root.path(), None).unwrap(), "demo");
    assert_eq!(scaffold(root.path(), Some("demo")).unwrap(), "demo");
    let err = scaffold(root.path(), Some("other"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("refusing to re-point"), "{err}");
    assert_eq!(
        read_project_id(root.path()).unwrap().as_deref(),
        Some("demo")
    );
}

fn staged(store: &mut Store, iri: &str, triples: &[(&str, &str, &str)]) {
    let g = store.graph_create(iri).unwrap();
    let datums: Vec<Datum> = triples
        .iter()
        .map(|(s, p, o)| Datum {
            entity: store.intern(s).unwrap(),
            attribute: store.intern(p).unwrap(),
            value: crate::types::Value::Str((*o).to_string()),
            valid_from: TS.into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    store.transact_to_graph(&datums, TS, None, None, g).unwrap();
}

fn count(store: &Store, iri: &str) -> usize {
    let g = store.lookup(iri).unwrap().unwrap();
    store.current_facts_in_graph(g).unwrap().len()
}

#[test]
fn promotion_diffs_into_the_project_graph_and_reload_is_a_no_op() {
    let mut store = Store::open_in_memory().unwrap();
    let target = project_iri("demo");
    staged(
        &mut store,
        "urn:test:staging:1",
        &[("ex:a", "ex:p", "1"), ("ex:b", "ex:p", "2")],
    );
    let first =
        promote_into_graph(&mut store, "s1", "urn:test:staging:1", &target, TS, None).unwrap();
    assert_eq!((first.added, first.removed, first.unchanged), (2, 0, 0));
    assert!(first.tx_id.is_some());
    assert_eq!(count(&store, &target), 2);
    // Nothing reached ROOT: the default graph is untouched.
    assert!(store.current_facts_in_graph(0).unwrap().is_empty());

    // Same bundle again (a re-load after a pull that changed nothing).
    let again =
        promote_into_graph(&mut store, "s1", "urn:test:staging:1", &target, TS, None).unwrap();
    assert_eq!((again.added, again.removed, again.unchanged), (0, 0, 2));
    assert!(
        again.tx_id.is_none(),
        "an unchanged load must not open a transaction"
    );

    // A new bundle: one fact kept, one dropped, one new.
    staged(
        &mut store,
        "urn:test:staging:2",
        &[("ex:a", "ex:p", "1"), ("ex:c", "ex:p", "3")],
    );
    let next =
        promote_into_graph(&mut store, "s2", "urn:test:staging:2", &target, TS, None).unwrap();
    assert_eq!((next.added, next.removed, next.unchanged), (1, 1, 1));
    assert_eq!(count(&store, &target), 2);
}

#[test]
fn a_missing_or_unregistered_staging_graph_is_refused() {
    let mut store = Store::open_in_memory().unwrap();
    let err = promote_into_graph(
        &mut store,
        "s",
        "urn:test:nope",
        "urn:quipu:project:x",
        TS,
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("no eligible staged import"), "{err}");
}
