use super::*;
use crate::namespace::{
    BOBBIN, BOBBIN_DEFINED_IN, BOBBIN_FILE_PATH, BOBBIN_IMPORTS, BOBBIN_LANGUAGE, BOBBIN_NAME,
};
use crate::store::Store;
use crate::types::{Op, Value};

/// Seed a store with two repos of code entities and some unresolved imports.
fn seeded_store() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-04-04T00:00:00Z";

    // --- Repo A: a Rust project ---

    // CodeModule: repo-a/src/utils.rs
    let mod_a = store
        .intern(&format!("{BOBBIN}code/repo-a/src/utils.rs"))
        .unwrap();
    let rdf_type = store.intern(crate::namespace::RDF_TYPE).unwrap();
    let code_module_type = store.intern(&format!("{BOBBIN}CodeModule")).unwrap();
    let name_attr = store.intern(BOBBIN_NAME).unwrap();
    let lang_attr = store.intern(BOBBIN_LANGUAGE).unwrap();
    let filepath_attr = store.intern(BOBBIN_FILE_PATH).unwrap();
    let defined_in_attr = store.intern(BOBBIN_DEFINED_IN).unwrap();
    let imports_attr = store.intern(BOBBIN_IMPORTS).unwrap();

    store
        .transact(
            &[
                Datum {
                    entity: mod_a,
                    attribute: rdf_type,
                    value: Value::Ref(code_module_type),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: mod_a,
                    attribute: filepath_attr,
                    value: Value::Str("src/utils.rs".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: mod_a,
                    attribute: lang_attr,
                    value: Value::Str("rust".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            ts,
            None,
            Some("seed"),
        )
        .unwrap();

    // CodeSymbol: repo-a/src/utils.rs::helper_fn
    let sym_helper = store
        .intern(&format!("{BOBBIN}code/repo-a/src/utils.rs::helper_fn"))
        .unwrap();
    let code_symbol_type = store.intern(&format!("{BOBBIN}CodeSymbol")).unwrap();

    store
        .transact(
            &[
                Datum {
                    entity: sym_helper,
                    attribute: rdf_type,
                    value: Value::Ref(code_symbol_type),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_helper,
                    attribute: name_attr,
                    value: Value::Str("helper_fn".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_helper,
                    attribute: defined_in_attr,
                    value: Value::Ref(mod_a),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            ts,
            None,
            Some("seed"),
        )
        .unwrap();

    // --- Repo B: another Rust project that imports from repo-a ---

    let mod_b = store
        .intern(&format!("{BOBBIN}code/repo-b/src/main.rs"))
        .unwrap();

    store
        .transact(
            &[
                Datum {
                    entity: mod_b,
                    attribute: rdf_type,
                    value: Value::Ref(code_module_type),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: mod_b,
                    attribute: filepath_attr,
                    value: Value::Str("src/main.rs".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: mod_b,
                    attribute: lang_attr,
                    value: Value::Str("rust".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            ts,
            None,
            Some("seed"),
        )
        .unwrap();

    // CodeSymbol: repo-b/src/main.rs::run — has an unresolved import
    let sym_run = store
        .intern(&format!("{BOBBIN}code/repo-b/src/main.rs::run"))
        .unwrap();

    store
        .transact(
            &[
                Datum {
                    entity: sym_run,
                    attribute: rdf_type,
                    value: Value::Ref(code_symbol_type),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_run,
                    attribute: name_attr,
                    value: Value::Str("run".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_run,
                    attribute: defined_in_attr,
                    value: Value::Ref(mod_b),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                // Unresolved import: "utils::helper_fn" (should resolve to repo-a's helper_fn)
                Datum {
                    entity: sym_run,
                    attribute: imports_attr,
                    value: Value::Str("utils::helper_fn".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            ts,
            None,
            Some("seed"),
        )
        .unwrap();

    store
}

#[test]
fn resolves_single_match() {
    let mut store = seeded_store();
    let resolvers = default_resolvers();
    let report = reconcile(&mut store, &resolvers, "2026-04-04T01:00:00Z").unwrap();

    assert_eq!(report.resolved, 1);
    assert_eq!(report.dangling, 0);
    assert_eq!(report.ambiguous, 0);

    // Verify the import edge is now a Ref, not a Str.
    let imports_id = store.lookup(BOBBIN_IMPORTS).unwrap().unwrap();
    let sym_run_id = store
        .lookup(&format!("{BOBBIN}code/repo-b/src/main.rs::run"))
        .unwrap()
        .unwrap();
    let facts = store.entity_facts(sym_run_id).unwrap();
    let import_fact = facts.iter().find(|f| f.attribute == imports_id).unwrap();
    assert!(
        matches!(import_fact.value, Value::Ref(_)),
        "expected Ref, got {import_fact:?}"
    );

    // Verify it points to the right entity.
    let target_id = store
        .lookup(&format!("{BOBBIN}code/repo-a/src/utils.rs::helper_fn"))
        .unwrap()
        .unwrap();
    assert_eq!(import_fact.value, Value::Ref(target_id));
}

#[test]
fn dangling_when_target_not_indexed() {
    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-04-04T00:00:00Z";

    // Create a symbol with an import that can't be resolved.
    let mod_id = store
        .intern(&format!("{BOBBIN}code/repo-x/src/lib.rs"))
        .unwrap();
    let sym_id = store
        .intern(&format!("{BOBBIN}code/repo-x/src/lib.rs::foo"))
        .unwrap();
    let name_attr = store.intern(BOBBIN_NAME).unwrap();
    let lang_attr = store.intern(BOBBIN_LANGUAGE).unwrap();
    let defined_in_attr = store.intern(BOBBIN_DEFINED_IN).unwrap();
    let imports_attr = store.intern(BOBBIN_IMPORTS).unwrap();

    store
        .transact(
            &[
                Datum {
                    entity: mod_id,
                    attribute: lang_attr,
                    value: Value::Str("rust".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_id,
                    attribute: name_attr,
                    value: Value::Str("foo".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_id,
                    attribute: defined_in_attr,
                    value: Value::Ref(mod_id),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                // Import target doesn't exist in the store.
                Datum {
                    entity: sym_id,
                    attribute: imports_attr,
                    value: Value::Str("nonexistent::Widget".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            ts,
            None,
            Some("seed"),
        )
        .unwrap();

    let resolvers = default_resolvers();
    let report = reconcile(&mut store, &resolvers, "2026-04-04T01:00:00Z").unwrap();

    assert_eq!(report.resolved, 0);
    assert_eq!(report.dangling, 1);
    assert_eq!(report.ambiguous, 0);

    // Import should still be a Str (unchanged).
    let facts = store.entity_facts(sym_id).unwrap();
    let import_fact = facts.iter().find(|f| f.attribute == imports_attr).unwrap();
    assert_eq!(
        import_fact.value,
        Value::Str("nonexistent::Widget".to_string())
    );
}

#[test]
fn ambiguous_when_multiple_matches() {
    let mut store = Store::open_in_memory().unwrap();
    let ts = "2026-04-04T00:00:00Z";

    let name_attr = store.intern(BOBBIN_NAME).unwrap();
    let lang_attr = store.intern(BOBBIN_LANGUAGE).unwrap();
    let defined_in_attr = store.intern(BOBBIN_DEFINED_IN).unwrap();
    let imports_attr = store.intern(BOBBIN_IMPORTS).unwrap();

    // Two symbols with the same name in different repos.
    let mod_1 = store
        .intern(&format!("{BOBBIN}code/repo-1/src/lib.rs"))
        .unwrap();
    let mod_2 = store
        .intern(&format!("{BOBBIN}code/repo-2/src/lib.rs"))
        .unwrap();
    let sym_1 = store
        .intern(&format!("{BOBBIN}code/repo-1/src/lib.rs::Widget"))
        .unwrap();
    let sym_2 = store
        .intern(&format!("{BOBBIN}code/repo-2/src/lib.rs::Widget"))
        .unwrap();

    let mod_src = store
        .intern(&format!("{BOBBIN}code/repo-src/src/main.rs"))
        .unwrap();
    let sym_src = store
        .intern(&format!("{BOBBIN}code/repo-src/src/main.rs::caller"))
        .unwrap();

    store
        .transact(
            &[
                // Module 1
                Datum {
                    entity: mod_1,
                    attribute: lang_attr,
                    value: Value::Str("rust".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                // Symbol 1 named "Widget"
                Datum {
                    entity: sym_1,
                    attribute: name_attr,
                    value: Value::Str("Widget".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_1,
                    attribute: defined_in_attr,
                    value: Value::Ref(mod_1),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                // Module 2
                Datum {
                    entity: mod_2,
                    attribute: lang_attr,
                    value: Value::Str("rust".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                // Symbol 2 named "Widget"
                Datum {
                    entity: sym_2,
                    attribute: name_attr,
                    value: Value::Str("Widget".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_2,
                    attribute: defined_in_attr,
                    value: Value::Ref(mod_2),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                // Source module + symbol with unresolved import
                Datum {
                    entity: mod_src,
                    attribute: lang_attr,
                    value: Value::Str("rust".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_src,
                    attribute: name_attr,
                    value: Value::Str("caller".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                Datum {
                    entity: sym_src,
                    attribute: defined_in_attr,
                    value: Value::Ref(mod_src),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
                // Ambiguous import: "Widget" matches both repos.
                Datum {
                    entity: sym_src,
                    attribute: imports_attr,
                    value: Value::Str("Widget".to_string()),
                    valid_from: ts.to_string(),
                    valid_to: None,
                    op: Op::Assert,
                },
            ],
            ts,
            None,
            Some("seed"),
        )
        .unwrap();

    let resolvers = default_resolvers();
    let report = reconcile(&mut store, &resolvers, "2026-04-04T01:00:00Z").unwrap();

    assert_eq!(report.resolved, 0);
    assert_eq!(report.ambiguous, 1);

    // Import should still be a Str (unchanged).
    let facts = store.entity_facts(sym_src).unwrap();
    let import_fact = facts.iter().find(|f| f.attribute == imports_attr).unwrap();
    assert_eq!(import_fact.value, Value::Str("Widget".to_string()));
}

#[test]
fn idempotent_rerun() {
    let mut store = seeded_store();
    let resolvers = default_resolvers();

    // First pass resolves the import.
    let r1 = reconcile(&mut store, &resolvers, "2026-04-04T01:00:00Z").unwrap();
    assert_eq!(r1.resolved, 1);

    // Second pass: already resolved edges (Ref) are skipped.
    let r2 = reconcile(&mut store, &resolvers, "2026-04-04T02:00:00Z").unwrap();
    assert_eq!(r2.resolved, 0);
    assert_eq!(r2.dangling, 0);
    assert_eq!(r2.ambiguous, 0);
}

#[test]
fn empty_store_returns_empty_report() {
    let mut store = Store::open_in_memory().unwrap();
    let resolvers = default_resolvers();
    let report = reconcile(&mut store, &resolvers, "2026-04-04T00:00:00Z").unwrap();

    assert_eq!(report.resolved, 0);
    assert_eq!(report.dangling, 0);
    assert_eq!(report.ambiguous, 0);
    assert!(report.details.is_empty());
}

#[test]
fn python_resolver_parses_dotted_path() {
    let resolver = PythonResolver;
    let candidates = resolver.parse("os.path.join");

    assert_eq!(candidates.len(), 2);
    // First candidate: symbol=join, module_hint=os/path
    assert_eq!(candidates[0].symbol_name.as_deref(), Some("join"));
    assert_eq!(candidates[0].module_hint.as_deref(), Some("os/path"));
    // Second candidate: module-only match
    assert!(candidates[1].symbol_name.is_none());
    assert_eq!(candidates[1].module_hint.as_deref(), Some("os/path/join"));
}

#[test]
fn rust_resolver_strips_crate_prefix() {
    let resolver = RustResolver;
    let candidates = resolver.parse("crate::store::Store");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].symbol_name.as_deref(), Some("Store"));
    assert_eq!(candidates[0].module_hint.as_deref(), Some("store"));
}

#[test]
fn go_resolver_handles_full_path() {
    let resolver = GoResolver;
    let candidates = resolver.parse("github.com/user/repo/pkg");

    assert_eq!(candidates.len(), 2);
    // Module-level match
    assert!(candidates[0].symbol_name.is_none());
    assert_eq!(
        candidates[0].module_hint.as_deref(),
        Some("github.com/user/repo/pkg")
    );
    // Package name match
    assert_eq!(candidates[1].symbol_name.as_deref(), Some("pkg"));
}

#[test]
fn iri_path_matching() {
    let iri = &format!("{BOBBIN}code/repo-a/src/utils.rs::helper_fn");
    assert!(iri_contains_path(iri, "utils"));
    assert!(iri_contains_path(iri, "src/utils"));
    assert!(!iri_contains_path(iri, "nonexistent"));

    // Non-bobbin IRIs don't match.
    assert!(!iri_contains_path("http://example.com/foo", "foo"));
}
