//! Tests for named datasets (quipu #69) — one per acceptance criterion.

use super::*;
use crate::types::Op;

const TS: &str = "2026-08-06T00:00:00Z";

fn store_with_graphs(iris: &[&str]) -> Store {
    let store = Store::open_in_memory().unwrap();
    for iri in iris {
        store.overlay_create(iri, 0).unwrap();
    }
    store
}

// ---------------------------------------------------------------------------
// Acceptance 3: a declared ordering refuses duplicate ranks
// ---------------------------------------------------------------------------

#[test]
fn a_declared_ordering_refuses_duplicate_ranks() {
    let mut store = store_with_graphs(&["urn:g:a", "urn:g:b"]);
    let err = store
        .dataset_create(
            "urn:ds:clash",
            &[
                DatasetMember::ranked("urn:g:a", 10),
                DatasetMember::ranked("urn:g:b", 10),
            ],
            TS,
            None,
        )
        .expect_err("two members at one rank is an ambiguous ordering");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:g:a") && msg.contains("urn:g:b"),
        "names both: {msg}"
    );
    assert!(msg.contains("10"), "names the rank: {msg}");
}

#[test]
fn a_refused_ordering_creates_nothing() {
    let mut store = store_with_graphs(&["urn:g:a", "urn:g:b"]);
    let _ = store.dataset_create(
        "urn:ds:clash",
        &[
            DatasetMember::ranked("urn:g:a", 1),
            DatasetMember::ranked("urn:g:b", 1),
        ],
        TS,
        None,
    );
    assert!(!store.is_dataset("urn:ds:clash").unwrap());
    assert_eq!(store.dataset_list().unwrap(), Vec::<String>::new());
}

#[test]
fn an_unordered_dataset_allows_many_members_without_ranks() {
    // NULL ord is exempt from the unique index — an unordered dataset is the
    // normal case and must not be mistaken for a rank collision.
    let mut store = store_with_graphs(&["urn:g:a", "urn:g:b", "urn:g:c"]);
    store
        .dataset_create(
            "urn:ds:plain",
            &[
                DatasetMember::new("urn:g:a"),
                DatasetMember::new("urn:g:b"),
                DatasetMember::new("urn:g:c"),
            ],
            TS,
            None,
        )
        .unwrap();
    assert_eq!(store.dataset_members("urn:ds:plain").unwrap().len(), 3);
}

#[test]
fn distinct_ranks_are_returned_in_declared_order() {
    let mut store = store_with_graphs(&["urn:g:lo", "urn:g:hi"]);
    store
        .dataset_create(
            "urn:ds:ordered",
            &[
                DatasetMember::ranked("urn:g:hi", 2),
                DatasetMember::ranked("urn:g:lo", 1),
            ],
            TS,
            None,
        )
        .unwrap();
    let m = store.dataset_members("urn:ds:ordered").unwrap();
    assert_eq!(m[0].graph_iri, "urn:g:lo", "rank 1 first");
    assert_eq!(m[1].graph_iri, "urn:g:hi");
}

// ---------------------------------------------------------------------------
// Acceptance 2: datasets overlap freely
// ---------------------------------------------------------------------------

#[test]
fn datasets_overlap_and_membership_implies_nothing_about_others() {
    // Alexander's semilattice: the city, not the tree. Neither contains the
    // other and they share a member.
    let mut store = store_with_graphs(&["urn:g:smac", "urn:g:thinker", "urn:g:doctrine"]);
    store
        .dataset_create(
            "urn:ds:play",
            &[
                DatasetMember::new("urn:g:smac"),
                DatasetMember::new("urn:g:thinker"),
            ],
            TS,
            None,
        )
        .unwrap();
    store
        .dataset_create(
            "urn:ds:audit",
            &[
                DatasetMember::new("urn:g:smac"),
                DatasetMember::new("urn:g:doctrine"),
            ],
            TS,
            None,
        )
        .unwrap();

    let play: Vec<String> = store
        .dataset_members("urn:ds:play")
        .unwrap()
        .into_iter()
        .map(|m| m.graph_iri)
        .collect();
    let audit: Vec<String> = store
        .dataset_members("urn:ds:audit")
        .unwrap()
        .into_iter()
        .map(|m| m.graph_iri)
        .collect();

    assert!(play.contains(&"urn:g:smac".to_string()));
    assert!(audit.contains(&"urn:g:smac".to_string()));
    assert!(
        !play.contains(&"urn:g:doctrine".to_string()),
        "membership in one implies nothing about the other"
    );
    assert!(!audit.contains(&"urn:g:thinker".to_string()));
}

// ---------------------------------------------------------------------------
// parent_branch is not touched
// ---------------------------------------------------------------------------

#[test]
fn creating_a_dataset_does_not_touch_the_branch_tree() {
    // Datasets and the branch tree are different relations over the same node
    // set. The overlay's bind-once parent must be exactly what it was.
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:ov", 0).unwrap();
    let g = store.lookup("urn:g:ov").unwrap().unwrap();
    let before: Option<i64> = store
        .conn
        .query_row(
            "SELECT parent_branch FROM graphs WHERE g = ?1",
            params![g],
            |r| r.get(0),
        )
        .unwrap();

    store
        .dataset_create("urn:ds:x", &[DatasetMember::new("urn:g:ov")], TS, None)
        .unwrap();

    let after: Option<i64> = store
        .conn
        .query_row(
            "SELECT parent_branch FROM graphs WHERE g = ?1",
            params![g],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(before, after, "the branch tree is not a taxonomy");
}

// ---------------------------------------------------------------------------
// Meta-graph mirroring
// ---------------------------------------------------------------------------

#[test]
fn a_dataset_is_mirrored_into_the_meta_graph() {
    let mut store = store_with_graphs(&["urn:g:m1"]);
    store
        .dataset_create(
            "urn:ds:mirrored",
            &[DatasetMember::new("urn:g:m1")],
            TS,
            None,
        )
        .unwrap();

    let meta_g = store.meta_graph_id().unwrap();
    let subject = store.lookup("urn:ds:mirrored").unwrap().unwrap();
    let facts: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e = ?1 AND g = ?2 AND op = ?3",
            params![subject, meta_g, Op::Assert as i64],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(facts, 2, "rdf:type quipu:Dataset + one quipu:includesGraph");
}

#[test]
fn an_empty_dataset_is_refused() {
    let mut store = Store::open_in_memory().unwrap();
    let err = store
        .dataset_create("urn:ds:empty", &[], TS, None)
        .expect_err("an empty dataset names nothing");
    assert!(err.to_string().contains("no members"));
}

#[test]
fn a_dataset_can_be_replaced_and_members_do_not_accumulate() {
    let mut store = store_with_graphs(&["urn:g:a", "urn:g:b"]);
    store
        .dataset_create("urn:ds:r", &[DatasetMember::new("urn:g:a")], TS, None)
        .unwrap();
    store
        .dataset_create("urn:ds:r", &[DatasetMember::new("urn:g:b")], TS, None)
        .unwrap();
    let m = store.dataset_members("urn:ds:r").unwrap();
    assert_eq!(m.len(), 1, "replace, not append");
    assert_eq!(m[0].graph_iri, "urn:g:b");
}

#[test]
fn a_member_naming_a_nonexistent_graph_contributes_nothing() {
    // Same rule apply_dataset already applies to an unknown FROM IRI: match
    // nothing, never fall through to ROOT.
    let mut store = store_with_graphs(&["urn:g:real"]);
    store
        .dataset_create(
            "urn:ds:partial",
            &[
                DatasetMember::new("urn:g:real"),
                DatasetMember::new("urn:g:never-created"),
            ],
            TS,
            None,
        )
        .unwrap();
    assert_eq!(
        store.dataset_member_ids("urn:ds:partial").unwrap().len(),
        1,
        "the unknown member resolves to nothing, not to ROOT"
    );
}

#[test]
fn removing_a_dataset_clears_its_membership() {
    let mut store = store_with_graphs(&["urn:g:a"]);
    store
        .dataset_create("urn:ds:gone", &[DatasetMember::new("urn:g:a")], TS, None)
        .unwrap();
    assert!(store.dataset_remove("urn:ds:gone").unwrap());
    assert!(!store.is_dataset("urn:ds:gone").unwrap());
    assert!(store.dataset_members("urn:ds:gone").unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// The MCP tool surface
// ---------------------------------------------------------------------------

#[test]
fn the_tool_creates_lists_shows_and_removes() {
    let mut store = store_with_graphs(&["urn:g:a", "urn:g:b"]);
    let created = crate::tool_datasets(
        &mut store,
        &serde_json::json!({
            "action": "create",
            "name": "urn:ds:tool",
            "members": ["urn:g:a", {"graph": "urn:g:b", "ord": 1}],
            "timestamp": TS,
        }),
    )
    .unwrap();
    assert_eq!(created["members"], 2);

    let listed = crate::tool_datasets(&mut store, &serde_json::json!({"action": "list"})).unwrap();
    assert_eq!(listed["datasets"][0], "urn:ds:tool");

    let shown = crate::tool_datasets(
        &mut store,
        &serde_json::json!({"action": "show", "name": "urn:ds:tool"}),
    )
    .unwrap();
    assert_eq!(shown["members"].as_array().unwrap().len(), 2);

    let removed = crate::tool_datasets(
        &mut store,
        &serde_json::json!({"action": "remove", "name": "urn:ds:tool"}),
    )
    .unwrap();
    assert_eq!(removed["removed"], true);
}

#[test]
fn an_unknown_action_errors_rather_than_silently_listing() {
    // The recorded `tool_shapes` silent-fall-through lesson: a typo'd action
    // that quietly lists is indistinguishable from one that did what you asked.
    let mut store = Store::open_in_memory().unwrap();
    let err = crate::tool_datasets(&mut store, &serde_json::json!({"action": "creat"}))
        .expect_err("a typo must not fall through to list");
    assert!(err.to_string().contains("unknown datasets action"));
}

#[test]
fn the_tool_surfaces_the_duplicate_rank_refusal() {
    let mut store = store_with_graphs(&["urn:g:a", "urn:g:b"]);
    let err = crate::tool_datasets(
        &mut store,
        &serde_json::json!({
            "action": "create",
            "name": "urn:ds:bad",
            "members": [{"graph": "urn:g:a", "ord": 5}, {"graph": "urn:g:b", "ord": 5}],
        }),
    )
    .expect_err("duplicate ranks");
    assert!(err.to_string().contains("unambiguous"), "{err}");
}
