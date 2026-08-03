//! Attribution-tree tests. Size-exempt (`*_tests.rs`).

use super::*;

/// A record attributed to `chain`.
fn rec(chain: &[&str]) -> TraceRecord {
    TraceRecord {
        kind: Some("guard".into()),
        principal_chain: chain.iter().map(|s| (*s).to_string()).collect(),
        ..TraceRecord::default()
    }
}

fn node<'a>(forest: &'a Forest, principal: &str) -> &'a Node {
    forest
        .nodes()
        .into_iter()
        .find(|n| n.principal == principal)
        .unwrap_or_else(|| panic!("no node for {principal}"))
}

#[test]
fn a_worker_subtree_attaches_to_its_dispatch_node() {
    // The whole point: a flat record cannot say which link was answerable, and
    // this is what turns the sequence back into the shape SARC §9.5 wants.
    let trace = [rec(&["orchestrator"]), rec(&["orchestrator", "worker"])];
    let forest = build(&trace);
    assert_eq!(forest.roots.len(), 1);
    let root = &forest.roots[0];
    assert_eq!(root.principal, "orchestrator");
    assert_eq!(root.children.len(), 1);
    assert_eq!(root.children[0].principal, "worker");
    assert_eq!(root.subtree_records(), 2);
    assert_eq!(
        root.records.len(),
        1,
        "the worker's record is NOT the root's"
    );
}

#[test]
fn depth_is_preserved_however_deep() {
    let trace = [rec(&["a", "b", "c", "d"])];
    let forest = build(&trace);
    assert_eq!(node(&forest, "d").path, vec!["a", "b", "c", "d"]);
    assert_eq!(forest.nodes().len(), 4);
}

#[test]
fn independent_chains_are_separate_roots() {
    let trace = [rec(&["alpha", "w"]), rec(&["beta", "w"])];
    let forest = build(&trace);
    assert_eq!(forest.roots.len(), 2);
    // Same worker id under two orchestrators must NOT merge — that would
    // attribute one dispatch's actions to the other's caller.
    assert_eq!(node(&forest, "alpha").children[0].subtree_records(), 1);
    assert_eq!(node(&forest, "beta").children[0].subtree_records(), 1);
}

#[test]
fn an_unattributed_record_is_not_placed_at_the_root() {
    // Attaching it to whichever root happened to be first would invent an answer
    // to the exact question the tree exists to answer.
    let trace = [rec(&["orchestrator"]), rec(&[])];
    let forest = build(&trace);
    assert_eq!(forest.unattributed, vec![1]);
    assert_eq!(forest.roots[0].subtree_records(), 1);
}

#[test]
fn a_dispatch_node_with_no_records_of_its_own_is_marked_implied() {
    // "This agent did nothing" and "this agent's actions are not in this window"
    // are different facts, and only one of them is good news.
    let trace = [rec(&["orchestrator", "worker"])];
    let forest = build(&trace);
    assert!(node(&forest, "orchestrator").implied);
    assert!(!node(&forest, "worker").implied);
    assert_eq!(forest.implied().len(), 1);
}

#[test]
fn the_control_an_observed_dispatch_node_is_not_implied() {
    let trace = [rec(&["orchestrator"]), rec(&["orchestrator", "worker"])];
    assert!(build(&trace).implied().is_empty());
}

#[test]
fn nodes_where_sibling_dispatches_would_collapse_are_reported() {
    // The honest statement of what reconstruction cannot do: two separate
    // dispatches of the same worker by the same caller produce the same chain,
    // so they land on one node. Not an error — one agent legitimately does many
    // things — but the reader must not be told the tree is unambiguous.
    let trace = [
        rec(&["orchestrator", "worker"]),
        rec(&["orchestrator", "worker"]),
    ];
    let forest = build(&trace);
    let collapsed = forest.collapsed();
    assert_eq!(collapsed.len(), 1);
    assert_eq!(collapsed[0].principal, "worker");
    assert_eq!(collapsed[0].records.len(), 2);
}

#[test]
fn the_control_a_single_record_node_is_not_reported_as_collapsed() {
    let trace = [rec(&["orchestrator", "worker"])];
    assert!(build(&trace).collapsed().is_empty());
}

#[test]
fn the_summary_says_the_trace_is_a_sequence() {
    // A tree presented without that sentence reads as a structural guarantee
    // the record does not make.
    let summary = build(&[rec(&["a", "b"])]).summary();
    assert!(summary.contains("sequence, not a tree"), "{summary}");
    assert!(summary.contains("NOT placed"), "{summary}");
    assert!(
        summary.contains("implied rather than observed"),
        "{summary}"
    );
}

#[test]
fn the_rendering_indents_by_depth_and_flags_implied_nodes() {
    let trace = [rec(&["a", "b"])];
    let lines = build(&trace).render();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("a:"), "{lines:?}");
    assert!(lines[0].contains("implied"), "{lines:?}");
    assert!(lines[1].starts_with("  b:"), "{lines:?}");
}

#[test]
fn an_empty_trace_is_an_empty_forest_not_an_error() {
    let forest = build(&[]);
    assert!(forest.roots.is_empty());
    assert!(forest.unattributed.is_empty());
    assert_eq!(forest.records, 0);
}

#[test]
fn build_jsonl_reads_a_real_spool_and_counts_what_it_could_not() {
    let jsonl = concat!(
        r#"{"kind":"guard","principal_chain":["orchestrator","worker"]}"#,
        "\n",
        r#"{"kind":"guard"}"#,
        "\n",
        "not json\n"
    );
    let (forest, unreadable) = build_jsonl(jsonl);
    assert_eq!(unreadable, 1, "the garbage line is counted, not dropped");
    assert_eq!(forest.records, 2);
    assert_eq!(forest.unattributed, vec![1]);
    assert_eq!(node(&forest, "worker").records, vec![0]);
}
