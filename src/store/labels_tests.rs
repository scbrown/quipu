//! Tests for graph labels (quipu #65) — one per acceptance criterion, plus the
//! cases where a wrong answer would be silent.

use super::*;
use crate::lattice::DEFAULT_TRUST_CHAIN;

const TS: &str = "2026-08-06T00:00:00Z";

fn store_with_graph(iri: &str) -> Store {
    let store = Store::open_in_memory().unwrap();
    store.overlay_create(iri, 0).unwrap();
    store
}

fn trust(iri: &str, chain: &str, rank: i64) -> Trust {
    Trust::new(iri, chain, rank)
}

// ---------------------------------------------------------------------------
// Acceptance 1: unlabelled -> undeclared, coverage none, never fabricated
// ---------------------------------------------------------------------------

#[test]
fn unlabelled_graph_reads_undeclared_with_coverage_none() {
    let store = store_with_graph("urn:g:plain");
    let l = store.label_of("urn:g:plain").unwrap();

    assert_eq!(l.freshness.coverage, Coverage::None);
    assert_eq!(l.trust.coverage, Coverage::None);
    assert_eq!(l.policy.coverage, Coverage::None);
    assert!(l.freshness.value.is_none(), "never a fabricated freshness");
    assert!(l.trust.value.is_none(), "never a fabricated trust");
    assert!(l.policy.value.is_none(), "never a fabricated policy");
    assert!(l.labels_tx.is_none());
}

#[test]
fn a_graph_that_does_not_exist_is_undeclared_not_an_error() {
    let store = Store::open_in_memory().unwrap();
    let l = store.label_of("urn:g:never-created").unwrap();
    assert_eq!(l.freshness.coverage, Coverage::None);
    assert!(l.freshness.value.is_none());
}

#[test]
fn declaring_one_axis_leaves_the_others_undeclared() {
    // Silence on trust must not be read as anything about trust.
    let mut store = store_with_graph("urn:g:partial");
    store
        .set_graph_label(
            "urn:g:partial",
            &GraphLabel {
                freshness: Some(Freshness::Fresh),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();

    let l = store.label_of("urn:g:partial").unwrap();
    assert_eq!(l.freshness.value, Some(Freshness::Fresh));
    assert_eq!(l.freshness.coverage, Coverage::Full);
    assert_eq!(l.trust.coverage, Coverage::None, "trust stays undeclared");
    assert!(l.trust.value.is_none());
}

// ---------------------------------------------------------------------------
// Acceptance 2: facts + cache in one savepoint; zero drift afterwards
// ---------------------------------------------------------------------------

#[test]
fn set_graph_label_writes_facts_and_cache_with_zero_drift() {
    let mut store = store_with_graph("urn:g:full");
    let label = GraphLabel {
        freshness: Some(Freshness::Recomputing),
        trust: Some(trust("urn:t:observed", DEFAULT_TRUST_CHAIN, 20)),
        policy: Some(PolicyClass::new(["pii", "no-export"])),
    };
    let tx = store
        .set_graph_label("urn:g:full", &label, TS, None)
        .unwrap();
    assert!(tx > 0);

    // The cache round-trips.
    let l = store.label_of("urn:g:full").unwrap();
    assert_eq!(l.freshness.value, Some(Freshness::Recomputing));
    let t = l.trust.value.expect("trust declared");
    assert_eq!(t.rank, 20);
    assert_eq!(t.chain, DEFAULT_TRUST_CHAIN);
    assert_eq!(t.iri, "urn:t:observed");
    let p = l.policy.value.expect("policy declared");
    assert!(p.contains("pii") && p.contains("no-export"));
    assert_eq!(l.labels_tx, Some(tx));

    // And RDF agrees with it — this is the doctor check.
    assert_eq!(
        store.graph_label_drift().unwrap(),
        vec![],
        "cache must agree with the meta-graph facts"
    );
}

#[test]
fn the_facts_land_in_the_meta_graph_not_the_labelled_graph() {
    // The whole authority story rests on this: the write goes to the
    // meta-graph, so relabelling needs authority THERE.
    let mut store = store_with_graph("urn:g:sub");
    store
        .set_graph_label(
            "urn:g:sub",
            &GraphLabel {
                freshness: Some(Freshness::Stale),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();

    let meta_g = store.meta_graph_id().unwrap();
    let subject = store.lookup("urn:g:sub").unwrap().unwrap();
    let in_meta: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e = ?1 AND g = ?2",
            params![subject, meta_g],
            |r| r.get(0),
        )
        .unwrap();
    let in_subject_graph: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e = ?1 AND g = ?2",
            params![subject, subject],
            |r| r.get(0),
        )
        .unwrap();
    assert!(in_meta > 0, "label facts belong to the meta-graph");
    assert_eq!(
        in_subject_graph, 0,
        "and must not be written into the graph being labelled"
    );
}

#[test]
fn a_whitespace_policy_token_is_refused_before_anything_is_written() {
    // Pre-flight validation, deliberately BEFORE the savepoint opens. Note what
    // this does NOT prove: it never reaches the fact write, so it says nothing
    // about atomicity. The savepoint is tested by the next test instead.
    let mut store = store_with_graph("urn:g:tok");
    let bad = GraphLabel {
        freshness: Some(Freshness::Fresh),
        trust: None,
        policy: Some(PolicyClass::new(["two words"])),
    };
    let err = store
        .set_graph_label("urn:g:tok", &bad, TS, None)
        .expect_err("a token with whitespace would read back as two tokens");
    assert!(err.to_string().contains("whitespace"));
    assert_eq!(
        store.label_of("urn:g:tok").unwrap().freshness.coverage,
        Coverage::None
    );
}

#[test]
fn a_failure_after_the_fact_write_rolls_the_facts_back_too() {
    // THE atomicity test. Labelling an unregistered graph succeeds at
    // `transact_to_graph` — the meta-graph facts are really staged — and then
    // fails on the cache UPDATE matching no row. The savepoint must take the
    // facts back out; otherwise the store keeps facts with no cache, which is
    // permanent drift that nothing but `doctor labels` would ever reveal.
    //
    // This replaces an earlier version of this test that used the whitespace
    // token above. That one passed with the ROLLBACK deleted — it was vacuous
    // for atomicity, because the validation fired before the savepoint opened.
    let mut store = Store::open_in_memory().unwrap();
    let label = GraphLabel {
        freshness: Some(Freshness::Fresh),
        ..Default::default()
    };
    let err = store
        .set_graph_label("urn:g:unregistered", &label, TS, None)
        .expect_err("an unregistered graph has no cache row to write");
    assert!(
        err.to_string().contains("not a registered graph"),
        "says why: {err}"
    );

    // The facts must be gone, not merely uncached.
    let meta_g = store.meta_graph_id().unwrap();
    let subject = store.lookup("urn:g:unregistered").unwrap().unwrap();
    let leftover: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE e = ?1 AND g = ?2",
            params![subject, meta_g],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        leftover, 0,
        "the savepoint must roll the meta-graph facts back with the failed cache write"
    );
    assert_eq!(store.graph_label_drift().unwrap(), vec![]);
}

#[test]
fn an_empty_label_is_refused_rather_than_written_as_a_no_op() {
    let mut store = store_with_graph("urn:g:empty");
    let err = store
        .set_graph_label("urn:g:empty", &GraphLabel::default(), TS, None)
        .expect_err("an empty label declares nothing");
    assert!(err.to_string().contains("declares no axis"));
}

// ---------------------------------------------------------------------------
// Acceptance 3: labels are bitemporal — as_of_tx reconstructs a prior label
// ---------------------------------------------------------------------------

#[test]
fn a_prior_label_is_reconstructable_from_the_meta_graph() {
    let mut store = store_with_graph("urn:g:time");
    let tx1 = store
        .set_graph_label(
            "urn:g:time",
            &GraphLabel {
                freshness: Some(Freshness::Fresh),
                ..Default::default()
            },
            "2026-08-01T00:00:00Z",
            None,
        )
        .unwrap();
    let tx2 = store
        .set_graph_label(
            "urn:g:time",
            &GraphLabel {
                freshness: Some(Freshness::Stale),
                ..Default::default()
            },
            "2026-08-02T00:00:00Z",
            None,
        )
        .unwrap();
    assert!(tx2 > tx1);

    // Current label is the later one.
    assert_eq!(
        store.label_of("urn:g:time").unwrap().freshness.value,
        Some(Freshness::Stale)
    );

    // The earlier one is still in the store, reachable by transaction time.
    // This is the property that makes "was this graph fresh when we decided?"
    // answerable — the reason labels are RDF and not just columns.
    let subject = store.lookup("urn:g:time").unwrap().unwrap();
    let attr = store.lookup(QUIPU_FRESHNESS).unwrap().unwrap();
    let meta_g = store.meta_graph_id().unwrap();
    // `facts.v` is an opaque Value blob, never a SQL-typed column — decode it.
    let raw: Vec<u8> = store
        .conn
        .query_row(
            "SELECT v FROM facts WHERE e = ?1 AND a = ?2 AND g = ?3 AND tx <= ?4 \
             ORDER BY tx DESC LIMIT 1",
            params![subject, attr, meta_g, tx1],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        Value::from_bytes(&raw).unwrap(),
        Value::Str("fresh".into()),
        "as-of tx1 the graph was fresh"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 4: trust_chain mismatch -> refuse rather than lie
// ---------------------------------------------------------------------------

#[test]
fn a_chain_redefinition_makes_label_of_refuse_not_lie() {
    let mut store = store_with_graph("urn:g:chain");
    store
        .set_graph_label(
            "urn:g:chain",
            &GraphLabel {
                trust: Some(trust("urn:t:canonical", "urn:chain:original", 40)),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();
    assert!(
        store.label_of("urn:g:chain").is_ok(),
        "consistent to begin with"
    );

    // Redefine the chain that ranks this trust value, WITHOUT touching the
    // graph's cached row — exactly what a chain migration would do.
    let meta_g = store.meta_graph_id().unwrap();
    let term = store.lookup("urn:t:canonical").unwrap().unwrap();
    let attr = store.intern(QUIPU_IN_CHAIN).unwrap();
    let new_chain = store.intern("urn:chain:replacement").unwrap();
    store
        .transact_to_graph(
            &[Datum {
                entity: term,
                attribute: attr,
                value: Value::Ref(new_chain),
                valid_from: "2026-08-07T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-08-07T00:00:00Z",
            None,
            None,
            meta_g,
        )
        .unwrap();

    let err = store
        .label_of("urn:g:chain")
        .expect_err("a rank outside its declared chain means nothing");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:chain:original"),
        "names the cached chain: {msg}"
    );
    assert!(
        msg.contains("urn:chain:replacement"),
        "names the current chain: {msg}"
    );
}

#[test]
fn a_chain_redefinition_shows_up_as_drift() {
    let mut store = store_with_graph("urn:g:drift");
    store
        .set_graph_label(
            "urn:g:drift",
            &GraphLabel {
                trust: Some(trust("urn:t:v", "urn:chain:a", 10)),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();

    let meta_g = store.meta_graph_id().unwrap();
    let term = store.lookup("urn:t:v").unwrap().unwrap();
    let attr = store.intern(QUIPU_IN_CHAIN).unwrap();
    let b = store.intern("urn:chain:b").unwrap();
    store
        .transact_to_graph(
            &[Datum {
                entity: term,
                attribute: attr,
                value: Value::Ref(b),
                valid_from: "2026-08-07T00:00:00Z".into(),
                valid_to: None,
                op: Op::Assert,
            }],
            "2026-08-07T00:00:00Z",
            None,
            None,
            meta_g,
        )
        .unwrap();

    let drift = store.graph_label_drift().unwrap();
    assert!(
        drift.iter().any(|d| d.axis == "trust"),
        "doctor must see the chain move: {drift:?}"
    );
}

#[test]
fn drift_detects_a_cache_edited_behind_the_rdf() {
    // RDF is authoritative. Corrupt only the cache and the doctor must say so —
    // otherwise the cache could drift indefinitely and read as truth.
    let mut store = store_with_graph("urn:g:corrupt");
    store
        .set_graph_label(
            "urn:g:corrupt",
            &GraphLabel {
                freshness: Some(Freshness::Fresh),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();
    assert_eq!(store.graph_label_drift().unwrap(), vec![]);

    let g = store.lookup("urn:g:corrupt").unwrap().unwrap();
    store
        .conn
        .execute("UPDATE graphs SET fresh_rank = 0 WHERE g = ?1", params![g])
        .unwrap();

    let drift = store.graph_label_drift().unwrap();
    assert_eq!(drift.len(), 1, "exactly the freshness axis: {drift:?}");
    assert_eq!(drift[0].axis, "freshness");
    assert_eq!(drift[0].rdf, "fresh", "RDF is the authority");
    assert_eq!(drift[0].cached, "stale");
}

#[test]
fn policy_drift_is_compared_as_a_set_not_a_string() {
    // A reordered token list is not drift. Reporting it as drift would train
    // everyone to ignore the doctor.
    let mut store = store_with_graph("urn:g:pol");
    store
        .set_graph_label(
            "urn:g:pol",
            &GraphLabel {
                policy: Some(PolicyClass::new(["pii", "no-export"])),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();

    let g = store.lookup("urn:g:pol").unwrap().unwrap();
    store
        .conn
        .execute(
            "UPDATE graphs SET policy = 'no-export pii' WHERE g = ?1",
            params![g],
        )
        .unwrap();

    assert_eq!(
        store.graph_label_drift().unwrap(),
        vec![],
        "same set, different order — not drift"
    );
}

// ---------------------------------------------------------------------------
// Acceptance 5: migration idempotent; a pre-migration store opens unchanged
// ---------------------------------------------------------------------------

#[test]
fn the_migration_is_idempotent() {
    let store = Store::open_in_memory().unwrap();
    // Running it again over an already-migrated store must be a no-op, not a
    // duplicate-column error.
    Store::migrate_graph_labels(&store.conn).unwrap();
    Store::migrate_graph_labels(&store.conn).unwrap();

    let cols: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('graphs') \
             WHERE name IN ('fresh_rank','trust_rank','trust_chain','policy','labels_tx')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cols, 5, "five cache columns, added exactly once");
}

#[test]
fn a_pre_label_store_migrates_without_touching_existing_graphs() {
    // Simulate a store created before #65: drop the columns by rebuilding the
    // registry the old way, then migrate and confirm nothing was fabricated.
    let store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:legacy", 0).unwrap();

    Store::migrate_graph_labels(&store.conn).unwrap();

    let l = store.label_of("urn:g:legacy").unwrap();
    assert_eq!(
        l.freshness.coverage,
        Coverage::None,
        "an existing graph must not acquire a label from the migration"
    );
    assert!(l.trust.value.is_none());
    assert_eq!(store.graph_label_drift().unwrap(), vec![]);
}

#[test]
fn the_meta_graph_is_reserved_committed_and_empty() {
    let store = Store::open_in_memory().unwrap();
    let meta_g = store.meta_graph_id().unwrap();

    let (class, parent): (String, Option<i64>) = store
        .conn
        .query_row(
            "SELECT class, parent_branch FROM graphs WHERE g = ?1",
            params![meta_g],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(class, "committed", "labels are durable and bitemporal");
    assert_eq!(parent, None, "self-rooted, like ROOT");

    let facts: i64 = store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM facts WHERE g = ?1",
            params![meta_g],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        facts, 0,
        "reserving it must add no facts, so no query changes"
    );
}

#[test]
fn the_meta_graph_id_is_a_runtime_rowid_not_a_constant() {
    // Why the seed cannot live in INIT_SQL: unlike ROOT's g=0, this id depends
    // on what the store already interned. Two stores with different histories
    // give it different ids, and a hardcoded INSERT would be wrong in both.
    let a = Store::open_in_memory().unwrap();
    let b = Store::open_in_memory().unwrap();
    b.intern("urn:some:other:term").unwrap();
    b.intern("urn:yet:another").unwrap();
    Store::migrate_graph_labels(&b.conn).unwrap();

    assert_ne!(
        a.meta_graph_id().unwrap(),
        0,
        "and it is never ROOT's constant"
    );
    // Both resolve correctly regardless of the numeric id they landed on.
    assert_eq!(
        a.resolve(a.meta_graph_id().unwrap()).unwrap(),
        META_GRAPH_IRI
    );
    assert_eq!(
        b.resolve(b.meta_graph_id().unwrap()).unwrap(),
        META_GRAPH_IRI
    );
}

// ---------------------------------------------------------------------------
// quipu #67 — the dataset fold
// ---------------------------------------------------------------------------

#[test]
fn a_dataset_is_only_as_fresh_as_its_weakest_member() {
    // #67 acceptance: FROM a b with fresh+stale graphs -> labels report stale.
    let mut store = Store::open_in_memory().unwrap();
    for (iri, f) in [("urn:g:a", Freshness::Fresh), ("urn:g:b", Freshness::Stale)] {
        store.overlay_create(iri, 0).unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    freshness: Some(f),
                    ..Default::default()
                },
                TS,
                None,
            )
            .unwrap();
    }
    let a = store.lookup("urn:g:a").unwrap().unwrap();
    let b = store.lookup("urn:g:b").unwrap().unwrap();

    let l = store.dataset_labels(&[a, b]).unwrap();
    assert_eq!(l.freshness.value, Some(Freshness::Stale));
    assert_eq!(l.freshness.coverage, Coverage::Full);
}

#[test]
fn an_undeclared_member_makes_coverage_partial_without_moving_the_value() {
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:lab", 0).unwrap();
    store.overlay_create("urn:g:bare", 0).unwrap();
    store
        .set_graph_label(
            "urn:g:lab",
            &GraphLabel {
                freshness: Some(Freshness::Fresh),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();
    let lab = store.lookup("urn:g:lab").unwrap().unwrap();
    let bare = store.lookup("urn:g:bare").unwrap().unwrap();

    let l = store.dataset_labels(&[lab, bare]).unwrap();
    assert_eq!(
        l.freshness.value,
        Some(Freshness::Fresh),
        "an undeclared graph must not drag the value to the floor"
    );
    assert_eq!(
        l.freshness.coverage,
        Coverage::Partial,
        "but it must be visible — silence must not flatter"
    );
}

#[test]
fn a_wholly_unlabelled_dataset_is_undeclared() {
    let store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:x", 0).unwrap();
    let x = store.lookup("urn:g:x").unwrap().unwrap();
    let l = store.dataset_labels(&[x]).unwrap();
    assert!(
        l.is_undeclared(),
        "reported as null, not as a fabricated label"
    );
    assert_eq!(l.freshness.coverage, Coverage::None);
}

#[test]
fn dataset_obligations_accumulate_by_union() {
    let mut store = Store::open_in_memory().unwrap();
    for (iri, tok) in [("urn:g:p1", "pii"), ("urn:g:p2", "no-export")] {
        store.overlay_create(iri, 0).unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    policy: Some(PolicyClass::new([tok])),
                    ..Default::default()
                },
                TS,
                None,
            )
            .unwrap();
    }
    let p1 = store.lookup("urn:g:p1").unwrap().unwrap();
    let p2 = store.lookup("urn:g:p2").unwrap().unwrap();

    let l = store.dataset_labels(&[p1, p2]).unwrap();
    let p = l.policy.value.expect("declared");
    assert!(
        p.contains("pii") && p.contains("no-export"),
        "a clean graph must not launder a restricted one"
    );
}

#[test]
fn a_cross_chain_dataset_refuses_rather_than_composing_ranks() {
    let mut store = Store::open_in_memory().unwrap();
    for (iri, chain) in [("urn:g:c1", "urn:chain:one"), ("urn:g:c2", "urn:chain:two")] {
        store.overlay_create(iri, 0).unwrap();
        store
            .set_graph_label(
                iri,
                &GraphLabel {
                    trust: Some(trust(&format!("{iri}#t"), chain, 10)),
                    ..Default::default()
                },
                TS,
                None,
            )
            .unwrap();
    }
    let c1 = store.lookup("urn:g:c1").unwrap().unwrap();
    let c2 = store.lookup("urn:g:c2").unwrap().unwrap();

    let err = store
        .dataset_labels(&[c1, c2])
        .expect_err("ranks from different chains are incomparable");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:chain:one") && msg.contains("urn:chain:two"),
        "{msg}"
    );
}

#[test]
fn the_meta_graph_is_excluded_from_all_named_graph_ids() {
    // Folding the label-holding graph's own label into "all named graphs" is a
    // category error, and would also make every GRAPH ?g query's label depend
    // on whether anyone had labelled the meta-graph.
    let store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:real", 0).unwrap();
    let ids = store.all_named_graph_ids().unwrap();
    let meta_g = store.meta_graph_id().unwrap();
    assert!(
        !ids.contains(&meta_g),
        "meta-graph must not be a dataset member"
    );
    assert!(!ids.contains(&0), "ROOT is not a NAMED graph");
    assert!(ids.contains(&store.lookup("urn:g:real").unwrap().unwrap()));
}

// ---------------------------------------------------------------------------
// quipu #68 — label floors
// ---------------------------------------------------------------------------

fn store_with_labeled(iri: &str, label: GraphLabel) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create(iri, 0).unwrap();
    store.set_graph_label(iri, &label, TS, None).unwrap();
    store
}

#[test]
fn floors_unset_is_zero_behaviour_change() {
    // #68 acceptance 1. The default store refuses nothing, whatever the labels.
    let store = store_with_labeled(
        "urn:g:stale",
        GraphLabel {
            freshness: Some(Freshness::Stale),
            ..Default::default()
        },
    );
    let g = store.lookup("urn:g:stale").unwrap().unwrap();
    assert!(store.labels_config().is_unset());
    assert!(
        store.check_label_floor(&[g]).is_ok(),
        "an unconfigured store must never refuse"
    );
}

#[test]
fn a_freshness_floor_refuses_and_names_the_offending_graph_and_axis() {
    // #68 acceptance 2.
    let mut store = store_with_labeled(
        "urn:g:old",
        GraphLabel {
            freshness: Some(Freshness::Stale),
            ..Default::default()
        },
    );
    store.labels_config_mut().min_freshness = Some("fresh".into());
    let g = store.lookup("urn:g:old").unwrap().unwrap();

    let err = store.check_label_floor(&[g]).expect_err("stale < fresh");
    let msg = err.to_string();
    assert!(msg.contains("urn:g:old"), "names the graph: {msg}");
    assert!(msg.contains("freshness"), "names the axis: {msg}");
    assert!(msg.contains("stale") && msg.contains("fresh"), "{msg}");
}

#[test]
fn a_member_above_the_floor_is_not_refused() {
    // Control: the floor must not refuse everything.
    let mut store = store_with_labeled(
        "urn:g:new",
        GraphLabel {
            freshness: Some(Freshness::Fresh),
            ..Default::default()
        },
    );
    store.labels_config_mut().min_freshness = Some("fresh".into());
    let g = store.lookup("urn:g:new").unwrap().unwrap();
    assert!(store.check_label_floor(&[g]).is_ok());
}

#[test]
fn undeclared_coverage_fails_a_configured_floor() {
    // #68 acceptance 3: fail-safe at enforcement, honest at reporting. The
    // graph still READS as undeclared (never fabricated) — it just does not
    // pass a floor.
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:bare", 0).unwrap();
    store.labels_config_mut().min_freshness = Some("stale".into());
    let g = store.lookup("urn:g:bare").unwrap().unwrap();

    assert!(
        store.label_of_id(g).unwrap().freshness.value.is_none(),
        "reporting stays honest: still undeclared"
    );
    let err = store
        .check_label_floor(&[g])
        .expect_err("undeclared must not pass a floor");
    assert!(err.to_string().contains("declares no freshness"), "{err}");
}

#[test]
fn partial_coverage_fails_because_one_member_is_undeclared() {
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:has", 0).unwrap();
    store.overlay_create("urn:g:hasnt", 0).unwrap();
    store
        .set_graph_label(
            "urn:g:has",
            &GraphLabel {
                freshness: Some(Freshness::Fresh),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();
    store.labels_config_mut().min_freshness = Some("fresh".into());
    let a = store.lookup("urn:g:has").unwrap().unwrap();
    let b = store.lookup("urn:g:hasnt").unwrap().unwrap();

    let err = store
        .check_label_floor(&[a, b])
        .expect_err("partial coverage fails a configured floor");
    assert!(
        err.to_string().contains("urn:g:hasnt"),
        "names the culprit: {err}"
    );
}

#[test]
fn a_trust_rank_floor_without_a_chain_is_refused_as_meaningless() {
    let mut store = store_with_labeled(
        "urn:g:t",
        GraphLabel {
            trust: Some(trust("urn:t:v", "urn:chain:a", 10)),
            ..Default::default()
        },
    );
    store.labels_config_mut().min_trust_rank = Some(30);
    // min_trust_chain deliberately left unset.
    let g = store.lookup("urn:g:t").unwrap().unwrap();
    let err = store
        .check_label_floor(&[g])
        .expect_err("a rank needs a chain");
    assert!(err.to_string().contains("min_trust_chain"), "{err}");
}

#[test]
fn a_trust_floor_in_another_chain_cannot_be_evaluated() {
    let mut store = store_with_labeled(
        "urn:g:t2",
        GraphLabel {
            trust: Some(trust("urn:t:v", "urn:chain:theirs", 90)),
            ..Default::default()
        },
    );
    store.labels_config_mut().min_trust_rank = Some(30);
    store.labels_config_mut().min_trust_chain = Some("urn:chain:ours".into());
    let g = store.lookup("urn:g:t2").unwrap().unwrap();

    let err = store.check_label_floor(&[g]).expect_err("cross-chain");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:chain:theirs") && msg.contains("urn:chain:ours"),
        "{msg}"
    );
    assert!(
        !msg.contains("below the configured floor"),
        "rank 90 must NOT be compared to 30 across chains: {msg}"
    );
}

#[test]
fn a_denied_policy_token_refuses_and_names_it() {
    let mut store = store_with_labeled(
        "urn:g:secret",
        GraphLabel {
            policy: Some(PolicyClass::new(["no-export", "pii"])),
            ..Default::default()
        },
    );
    store.labels_config_mut().deny_policy_tokens = vec!["no-export".into()];
    let g = store.lookup("urn:g:secret").unwrap().unwrap();

    let err = store.check_label_floor(&[g]).expect_err("denied token");
    let msg = err.to_string();
    assert!(
        msg.contains("urn:g:secret") && msg.contains("no-export"),
        "{msg}"
    );
}

#[test]
fn an_undenied_policy_token_passes() {
    let mut store = store_with_labeled(
        "urn:g:ok",
        GraphLabel {
            policy: Some(PolicyClass::new(["pii"])),
            ..Default::default()
        },
    );
    store.labels_config_mut().deny_policy_tokens = vec!["no-export".into()];
    let g = store.lookup("urn:g:ok").unwrap().unwrap();
    assert!(store.check_label_floor(&[g]).is_ok());
}

#[test]
fn the_floor_reaches_the_store_from_a_parsed_config_file() {
    // A config-switched feature can pass every unit test that sets the struct
    // directly while being UNREACHABLE from a real deployment, because the
    // parse or the wiring is what is broken. So start from TOML text, exactly
    // as a deployment does.
    let toml = r#"
[quipu.labels]
min_freshness = "fresh"
deny_policy_tokens = ["no-export"]
"#;
    // Parse the WHOLE QuipuConfig, so the serde rename is exercised too: the
    // Rust field is `label_floors` but the documented TOML key is `labels`, and
    // a rename that silently stopped matching would leave the floor unset with
    // nothing to see.
    let parsed: toml::Value = toml::from_str(toml).unwrap();
    let full: crate::config::QuipuConfig = parsed["quipu"].clone().try_into().unwrap();
    let cfg = full.label_floors;

    assert_eq!(cfg.min_freshness.as_deref(), Some("fresh"));
    assert_eq!(cfg.deny_policy_tokens, vec!["no-export".to_string()]);
    assert!(!cfg.is_unset(), "a parsed floor must not read as unset");

    // And it must actually gate once wired the way `server.rs` wires it.
    let mut store = store_with_labeled(
        "urn:g:cfg",
        GraphLabel {
            freshness: Some(Freshness::Stale),
            ..Default::default()
        },
    );
    store.labels_config_mut().clone_from(&cfg);
    let g = store.lookup("urn:g:cfg").unwrap().unwrap();
    assert!(
        store.check_label_floor(&[g]).is_err(),
        "the floor must gate when it arrives from a config FILE, not just from a setter"
    );
}

// ---------------------------------------------------------------------------
// quipu #80 — retrieval policy: RECOMMEND, NEVER ENFORCE
// ---------------------------------------------------------------------------

#[test]
fn a_recommended_floor_is_queryable() {
    let mut store = store_with_graph("urn:g:pack");
    store
        .set_recommended_floor(
            "urn:g:pack",
            &RecommendedFloor {
                min_freshness: Some(Freshness::Fresh),
                min_trust: Some("urn:t:attested".into()),
            },
            TS,
            None,
        )
        .unwrap();

    let f = store.recommended_floor("urn:g:pack").unwrap();
    assert_eq!(f.min_freshness, Some(Freshness::Fresh));
    assert_eq!(f.min_trust.as_deref(), Some("urn:t:attested"));
    let line = f.line("urn:g:pack");
    assert!(line.contains("RECOMMENDS"), "{line}");
    assert!(
        line.contains("advisory only"),
        "the banner must not read as something the store applied: {line}"
    );
}

#[test]
fn an_ungoverned_graph_recommends_nothing() {
    let store = store_with_graph("urn:g:bare");
    assert!(store.recommended_floor("urn:g:bare").unwrap().is_empty());
    assert!(
        store
            .recommended_floor("urn:g:never-made")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn enforcement_is_byte_identical_with_and_without_a_recommendation() {
    // #80 acceptance 1, and the LOAD-BEARING rule of the whole issue.
    //
    // A pack that could TIGHTEN enforcement could DoS its consumer; one that
    // could LOOSEN it could bypass the consumer's floor. So the recommendation
    // must make NO difference to what is enforced — in either direction.
    //
    // Both arms, because "recommend never enforces" is two claims:
    //   A. an aggressive recommendation must not start refusing things;
    //   B. a permissive recommendation must not stop the consumer's own floor.
    let mk = || {
        let mut s = Store::open_in_memory().unwrap();
        s.overlay_create("urn:g:p", 0).unwrap();
        s.set_graph_label(
            "urn:g:p",
            &GraphLabel {
                freshness: Some(Freshness::Recomputing),
                ..Default::default()
            },
            TS,
            None,
        )
        .unwrap();
        s
    };

    // --- A. no consumer floor. A demanding recommendation must change nothing.
    let plain = mk();
    let g = plain.lookup("urn:g:p").unwrap().unwrap();
    let baseline = plain.check_label_floor(&[g]).is_ok();
    assert!(baseline, "no floor configured -> nothing refused");

    let mut recommended = mk();
    recommended
        .set_recommended_floor(
            "urn:g:p",
            &RecommendedFloor {
                min_freshness: Some(Freshness::Fresh),
                min_trust: None,
            },
            TS,
            None,
        )
        .unwrap();
    let g2 = recommended.lookup("urn:g:p").unwrap().unwrap();
    assert_eq!(
        recommended.check_label_floor(&[g2]).is_ok(),
        baseline,
        "a pack recommending `fresh` must NOT start refusing a `recomputing` graph"
    );

    // --- B. a consumer floor IS set, and a permissive recommendation must not
    // relax it.
    let mut consumer = mk();
    consumer.labels_config_mut().min_freshness = Some("fresh".into());
    let gc = consumer.lookup("urn:g:p").unwrap().unwrap();
    let refused_without = consumer.check_label_floor(&[gc]).is_err();
    assert!(refused_without, "the consumer's own floor refuses");

    consumer
        .set_recommended_floor(
            "urn:g:p",
            &RecommendedFloor {
                min_freshness: Some(Freshness::Stale),
                min_trust: None,
            },
            TS,
            None,
        )
        .unwrap();
    assert_eq!(
        consumer.check_label_floor(&[gc]).is_err(),
        refused_without,
        "a pack recommending `stale` must NOT bypass the consumer's `fresh` floor"
    );
}

#[test]
fn an_empty_recommendation_is_refused() {
    let mut store = store_with_graph("urn:g:e");
    assert!(
        store
            .set_recommended_floor("urn:g:e", &RecommendedFloor::default(), TS, None)
            .is_err()
    );
}

#[test]
fn a_default_dataset_resolves_through_dataset_expansion() {
    // #80 acceptance 2: the declaration names a dataset, and that name goes
    // through #69's expansion — it is not a second, parallel resolution path.
    use crate::store::datasets::DatasetMember;
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:pack", 0).unwrap();
    store.overlay_create("urn:g:terms", 0).unwrap();
    store
        .dataset_create(
            "urn:ds:pack",
            &[
                DatasetMember::new("urn:g:pack"),
                DatasetMember::new("urn:g:terms"),
            ],
            TS,
            None,
        )
        .unwrap();
    store
        .set_default_dataset("urn:g:pack", "urn:ds:pack", TS, None)
        .unwrap();

    let declared = store.default_dataset("urn:g:pack").unwrap().unwrap();
    assert_eq!(declared, "urn:ds:pack");
    // And it expands through #69 rather than needing its own resolver.
    assert_eq!(
        store.dataset_member_ids(&declared).unwrap().len(),
        2,
        "the declared name resolves through the ONE dataset expansion"
    );
}

#[test]
fn declaring_a_default_dataset_does_not_activate_it() {
    // A dataset is never implicitly active (#69), and #80 must not smuggle in
    // an exception. Declaring one must not change what a bare query reads.
    use crate::store::datasets::DatasetMember;
    use crate::types::{Op, Value};
    let mut store = Store::open_in_memory().unwrap();
    let g = store.overlay_create("urn:g:pack", 0).unwrap();
    let e = store.intern("http://example.org/s").unwrap();
    let a = store.intern("http://example.org/p").unwrap();
    store
        .overlay_write(g, Op::Assert, e, a, Value::Str("in-pack".into()), TS)
        .unwrap();
    store
        .dataset_create("urn:ds:pack", &[DatasetMember::new("urn:g:pack")], TS, None)
        .unwrap();

    let before = crate::sparql::query(&store, "SELECT ?o WHERE { ?s ?p ?o }")
        .unwrap()
        .rows()
        .len();
    store
        .set_default_dataset("urn:g:pack", "urn:ds:pack", TS, None)
        .unwrap();
    let after = crate::sparql::query(&store, "SELECT ?o WHERE { ?s ?p ?o }")
        .unwrap()
        .rows()
        .len();
    assert_eq!(
        before, after,
        "declaring is not activating; ROOT-alone survives"
    );
}

#[test]
fn the_recommendation_banner_is_surfaced_for_every_graph_that_declares_one() {
    // The `doctor labels` surface walks `all_named_graph_ids` and prints
    // `line()` per declaring graph. Asserted at that seam rather than by
    // capturing stdout, so the test breaks if the DATA stops being reachable —
    // which is the part that could silently regress.
    let mut store = Store::open_in_memory().unwrap();
    store.overlay_create("urn:g:a", 0).unwrap();
    store.overlay_create("urn:g:b", 0).unwrap();
    store
        .set_recommended_floor(
            "urn:g:a",
            &RecommendedFloor {
                min_freshness: Some(Freshness::Fresh),
                min_trust: None,
            },
            TS,
            None,
        )
        .unwrap();

    let declaring: Vec<String> = store
        .all_named_graph_ids()
        .unwrap()
        .into_iter()
        .filter_map(|g| {
            let iri = store.resolve(g).ok()?;
            let rec = store.recommended_floor(&iri).ok()?;
            (!rec.is_empty()).then(|| rec.line(&iri))
        })
        .collect();

    assert_eq!(declaring.len(), 1, "only the declaring graph is surfaced");
    assert!(declaring[0].contains("urn:g:a"));
    assert!(declaring[0].contains("advisory only"));
}
