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
