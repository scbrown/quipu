//! Tests for the stored named-query registry (quipu #79).

use super::*;

const TS: &str = "2026-08-06T00:00:00Z";

fn q(name: &str, template: &str, params: Vec<StoredParam>) -> StoredQuery {
    StoredQuery {
        name: name.into(),
        description: "d".into(),
        template: template.into(),
        dataset: None,
        params,
    }
}

fn p(name: &str, kind: &str, required: bool, default: Option<&str>) -> StoredParam {
    StoredParam {
        name: name.into(),
        kind: kind.into(),
        required,
        default: default.map(Into::into),
        description: "d".into(),
    }
}

// --- Acceptance 2: invalid definitions refuse AT LOAD ---

#[test]
fn a_template_that_does_not_parse_is_refused_at_load() {
    let store = Store::open_in_memory().unwrap();
    let bad = q("bad", "SELEKT ?s WHERE { ?s ?p ?o }", vec![]);
    let err = store.query_load(&bad, TS).expect_err("not SPARQL");
    assert!(err.to_string().contains("does not parse"), "{err}");
    assert!(store.query_get("bad").unwrap().is_none(), "nothing stored");
}

#[test]
fn a_placeholder_with_no_spec_is_refused() {
    let store = Store::open_in_memory().unwrap();
    let bad = q("bad", "SELECT ?s WHERE { <{entity}> ?p ?o }", vec![]);
    let err = store.query_load(&bad, TS).expect_err("unknown placeholder");
    assert!(err.to_string().contains("{entity}"), "names it: {err}");
}

#[test]
fn an_optional_param_without_a_default_is_refused() {
    // THE latent hole this closes. `render` skips an omitted optional with no
    // default, leaving `{limit}` VERBATIM in the SPARQL handed to the parser.
    let store = Store::open_in_memory().unwrap();
    let bad = q(
        "bad",
        "SELECT ?s WHERE { ?s ?p ?o } LIMIT {limit}",
        vec![p("limit", "int", false, None)],
    );
    let err = store
        .query_load(&bad, TS)
        .expect_err("optional, no default");
    let msg = err.to_string();
    assert!(msg.contains("no default"), "{msg}");
    assert!(msg.contains("verbatim"), "says WHY it matters: {msg}");
}

#[test]
fn a_spec_with_no_placeholder_is_refused() {
    let store = Store::open_in_memory().unwrap();
    let bad = q(
        "bad",
        "SELECT ?s WHERE { ?s ?p ?o }",
        vec![p("unused", "text", true, None)],
    );
    assert!(
        store.query_load(&bad, TS).is_err(),
        "a param nothing uses is a typo"
    );
}

#[test]
fn an_unknown_param_kind_is_refused() {
    let store = Store::open_in_memory().unwrap();
    let bad = q(
        "bad",
        "SELECT ?s WHERE { <{e}> ?p ?o }",
        vec![p("e", "uri", true, None)],
    );
    assert!(
        store.query_load(&bad, TS).is_err(),
        "'uri' is not a kind; 'iri' is"
    );
}

#[test]
fn a_duplicate_param_is_refused() {
    let store = Store::open_in_memory().unwrap();
    let bad = q(
        "bad",
        "SELECT ?s WHERE { <{e}> ?p ?o }",
        vec![p("e", "iri", true, None), p("e", "text", true, None)],
    );
    assert!(store.query_load(&bad, TS).is_err());
}

// --- Acceptance 1 & 3: round-trip, and close-don't-overwrite ---

#[test]
fn a_valid_query_round_trips_with_ordered_params() {
    let store = Store::open_in_memory().unwrap();
    let good = q(
        "facts",
        "SELECT ?p ?o WHERE { <{entity}> ?p ?o } LIMIT {limit}",
        vec![
            p("entity", "iri", true, None),
            p("limit", "int", false, Some("100")),
        ],
    );
    store.query_load(&good, TS).unwrap();

    let back = store.query_get("facts").unwrap().unwrap();
    assert_eq!(back, good, "round-trips exactly, params in order");
    assert_eq!(store.query_list().unwrap().len(), 1);
}

#[test]
fn reloading_a_name_closes_the_prior_row_and_history_stays_queryable() {
    // #79 acceptance 3 — never INSERT OR REPLACE.
    let store = Store::open_in_memory().unwrap();
    store
        .query_load(
            &q("x", "SELECT ?s WHERE { ?s ?p ?o }", vec![]),
            "2026-08-01T00:00:00Z",
        )
        .unwrap();
    store
        .query_load(
            &q("x", "SELECT ?o WHERE { ?s ?p ?o }", vec![]),
            "2026-08-02T00:00:00Z",
        )
        .unwrap();

    assert_eq!(
        store.query_get("x").unwrap().unwrap().template,
        "SELECT ?o WHERE { ?s ?p ?o }",
        "current is the latest"
    );
    let rows: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM queries WHERE name = 'x'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 2, "the prior version is CLOSED, not discarded");
}

#[test]
fn a_reload_does_not_leak_the_prior_versions_params() {
    // The params join is on (name, valid_from); getting that wrong would show
    // BOTH versions' params on the current one, silently.
    let store = Store::open_in_memory().unwrap();
    store
        .query_load(
            &q(
                "x",
                "SELECT ?s WHERE { <{a}> ?p ?o }",
                vec![p("a", "iri", true, None)],
            ),
            "2026-08-01T00:00:00Z",
        )
        .unwrap();
    store
        .query_load(
            &q(
                "x",
                "SELECT ?s WHERE { <{b}> ?p ?o }",
                vec![p("b", "iri", true, None)],
            ),
            "2026-08-02T00:00:00Z",
        )
        .unwrap();

    let cur = store.query_get("x").unwrap().unwrap();
    assert_eq!(cur.params.len(), 1, "only the current version's params");
    assert_eq!(cur.params[0].name, "b");
}

#[test]
fn remove_closes_rather_than_deletes() {
    let store = Store::open_in_memory().unwrap();
    store
        .query_load(&q("x", "SELECT ?s WHERE { ?s ?p ?o }", vec![]), TS)
        .unwrap();
    assert!(store.query_remove("x", "2026-08-03T00:00:00Z").unwrap());
    assert!(store.query_get("x").unwrap().is_none());
    let rows: i64 = store
        .conn
        .query_row("SELECT COUNT(*) FROM queries WHERE name = 'x'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(rows, 1, "closed, never deleted");
    assert!(!store.query_remove("x", TS).unwrap(), "already closed");
}

#[test]
fn a_dataset_scope_round_trips() {
    let store = Store::open_in_memory().unwrap();
    let mut scoped = q("s", "SELECT ?s WHERE { ?s ?p ?o }", vec![]);
    scoped.dataset = Some("urn:ds:pack".into());
    store.query_load(&scoped, TS).unwrap();
    assert_eq!(
        store.query_get("s").unwrap().unwrap().dataset.as_deref(),
        Some("urn:ds:pack")
    );
}

// --- Acceptance 1 & 4: dispatch, origin flags, dataset activation ---

#[test]
fn stored_and_builtin_coexist_and_are_origin_flagged() {
    let store = Store::open_in_memory().unwrap();
    store
        .query_load(&q("my_local", "SELECT ?s WHERE { ?s ?p ?o }", vec![]), TS)
        .unwrap();

    let listing = crate::tool_ask(&store, &serde_json::json!({})).unwrap();
    let qs = listing["queries"].as_array().unwrap();
    let builtin = qs
        .iter()
        .find(|e| e["name"] == "entity_facts")
        .expect("the compiled-in catalog is still listed");
    let stored = qs
        .iter()
        .find(|e| e["name"] == "my_local")
        .expect("the stored one is listed too");
    assert_eq!(builtin["source"], "builtin");
    assert_eq!(stored["source"], "stored");
}

#[test]
fn a_stored_query_is_invocable_by_name() {
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/s").unwrap();
    let a = store.intern("http://example.org/p").unwrap();
    store
        .transact(
            &[crate::store::Datum {
                entity: e,
                attribute: a,
                value: Value::Str("v".into()),
                valid_from: TS.into(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            None,
            None,
        )
        .unwrap();
    store
        .query_load(&q("all", "SELECT ?s ?o WHERE { ?s ?p ?o }", vec![]), TS)
        .unwrap();

    let out = crate::tool_ask(&store, &serde_json::json!({"name": "all"})).unwrap();
    assert_eq!(out["count"], 1, "the stored query ran: {out}");
}

#[test]
fn a_builtin_name_is_never_shadowed_by_a_stored_one() {
    // Dispatch is compiled-in FIRST. Loading a pack must not be able to
    // silently change what an existing name does.
    let store = Store::open_in_memory().unwrap();
    store
        .query_load(
            &q("entity_facts", "SELECT ?zzz WHERE { ?zzz ?p ?o }", vec![]),
            TS,
        )
        .unwrap();
    let out = crate::tool_ask(
        &store,
        &serde_json::json!({"name": "entity_facts", "params": {"entity": "http://example.org/x"}}),
    )
    .unwrap();
    // Assert on something ONLY the builtin has. An earlier version of this
    // checked for "?p ?o" — which appears in BOTH templates, so it passed even
    // when the stored one was dispatched. Sabotage caught it; the discriminator
    // is `?zzz`, which only the stored override contains.
    let sparql = out["sparql"].as_str().unwrap();
    assert!(
        !sparql.contains("?zzz"),
        "the stored override was dispatched — a pack must not shadow a builtin: {sparql}"
    );
    assert!(
        sparql.contains("LIMIT"),
        "the BUILTIN template ran (it has the LIMIT the override lacks): {sparql}"
    );
}

#[test]
fn a_dataset_scoped_query_activates_its_dataset_and_the_caller_can_override() {
    // #79 acceptance 4, both halves.
    use crate::store::datasets::DatasetMember;
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let e = store.intern("http://example.org/s").unwrap();
    let a = store.intern("http://example.org/p").unwrap();

    let g = store.overlay_create("urn:g:pack", 0).unwrap();
    store
        .overlay_write(g, Op::Assert, e, a, Value::Str("in-pack".into()), TS)
        .unwrap();
    store
        .dataset_create("urn:ds:pack", &[DatasetMember::new("urn:g:pack")], TS, None)
        .unwrap();

    let mut scoped = q("packq", "SELECT ?o WHERE { ?s ?p ?o }", vec![]);
    scoped.dataset = Some("urn:ds:pack".into());
    store.query_load(&scoped, TS).unwrap();

    // Scoped: reads the pack's graph, which ROOT-alone would not see.
    let scoped_out = crate::tool_ask(&store, &serde_json::json!({"name": "packq"})).unwrap();
    assert_eq!(scoped_out["count"], 1, "the dataset was activated");

    // Caller override wins: naming an unrelated graph reads that instead.
    let overridden = crate::tool_ask(
        &store,
        &serde_json::json!({"name": "packq", "graph": "urn:g:nonexistent"}),
    )
    .unwrap();
    assert_eq!(
        overridden["count"], 0,
        "the caller's graph wins; the scope is a default, not a lock"
    );
}

#[test]
fn the_tool_surfaces_load_validation_and_errors_on_a_typo_action() {
    let store = Store::open_in_memory().unwrap();
    let err = crate::tool_queries(
        &store,
        &serde_json::json!({
            "action": "load", "name": "bad",
            "template": "SELECT ?s WHERE { ?s ?p ?o } LIMIT {n}",
            "params": [{"name": "n", "type": "int", "required": false}]
        }),
    )
    .expect_err("optional without default");
    assert!(err.to_string().contains("no default"), "{err}");

    let typo = crate::tool_queries(&store, &serde_json::json!({"action": "lod"}))
        .expect_err("a typo must not fall through to list");
    assert!(typo.to_string().contains("unknown queries action"));
}
