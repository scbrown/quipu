//! Tests for the denial quarantine and verdict replay. Size-exempt (`*tests.rs`).
//!
//! The five properties the quarantine exists for, each tested on its own:
//! GS2 still holds (refused content is invisible to queries and search), a
//! full-retention denial re-derives, a digest-only denial re-derives from the
//! right presented delta and refuses a tampered one, a purge keeps the verdict
//! and its digest, and a rule change after the denial does not move the replay.

use super::*;
use crate::governance::denial_replay::{Basis, Seal, replay_verdict};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::sparql::{self, QueryResult};

const TS: &str = "2026-01-01T00:00:00Z";
const LATER: &str = "2026-02-01T00:00:00Z";
const DOC_TYPE: &str = "http://ex/Doc";
const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const BODY: &str = "http://ex/body";
const POLICY: &str = "http://ex/policy/label";
/// A `deny` claim: the target must carry an `rdfs:label`.
const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";
/// The amended claim: anything that exists passes.
const ANYTHING: &str = "ASK { $target ?p ?o }";
/// A string that exists only inside the refused attempt. If it is ever
/// readable through the governed graph, GS2 is broken.
const CANARY: &str = "QUARANTINE-CANARY-7f3a";

fn signed_store(full: bool) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    if full {
        store.governance_config_mut().quarantine.retain_full = vec!["*".into()];
    }
    let dir = tempfile::tempdir().unwrap();
    let identity =
        crate::signing::SigningIdentity::load(&dir.path().join("k.pk8"), "quipu").unwrap();
    store.set_signing_identity(std::sync::Arc::new(identity));
    define_policy(&mut store);
    store
}

fn datum(store: &Store, s: &str, p: &str, v: Value, op: Op, ts: &str) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: ts.to_string(),
        valid_to: None,
        op,
    }
}

fn policy_field(store: &Store, field: &str, v: &str, op: Op, ts: &str) -> Datum {
    datum(
        store,
        POLICY,
        &format!("{DEFAULT_BASE_NS}{field}"),
        Value::Str(v.to_string()),
        op,
        ts,
    )
}

fn define_policy(store: &mut Store) {
    let class_ref = Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}Policy")).unwrap());
    let datums = vec![
        datum(store, POLICY, RDF_TYPE, class_ref, Op::Assert, TS),
        policy_field(store, "targets", DOC_TYPE, Op::Assert, TS),
        policy_field(store, "claim", REQUIRE_LABEL, Op::Assert, TS),
        policy_field(store, "boundary", "action", Op::Assert, TS),
        policy_field(store, "effect", "deny", Op::Assert, TS),
    ];
    store.transact(&datums, TS, None, None).unwrap();
}

/// A Doc with the canary body and no label: the policy refuses it.
fn unlabeled_doc(store: &Store, iri: &str, body: &str) -> Vec<Datum> {
    let doc_type = Value::Ref(store.intern(DOC_TYPE).unwrap());
    vec![
        datum(store, iri, RDF_TYPE, doc_type, Op::Assert, TS),
        datum(store, iri, BODY, Value::Str(body.into()), Op::Assert, TS),
    ]
}

/// A Doc that satisfies the policy.
fn labeled_doc(store: &Store, iri: &str) -> Vec<Datum> {
    let mut datums = unlabeled_doc(store, iri, "fine");
    datums.push(datum(
        store,
        iri,
        RDFS_LABEL,
        Value::Str("a doc".into()),
        Op::Assert,
        TS,
    ));
    datums
}

/// Attempt the refused write and return the verdict IRI the quarantine keys.
fn refuse(store: &mut Store) -> (String, Vec<Datum>) {
    let datums = unlabeled_doc(store, "http://ex/doc1", CANARY);
    store.set_principal_chain(vec!["amaru".into()]);
    let err = store
        .transact(&datums, TS, Some("amaru"), Some("test"))
        .expect_err("an unlabeled Doc is refused");
    assert!(matches!(err, Error::PolicyDenied(_)), "{err}");
    let entries = entries(store, None).unwrap();
    assert_eq!(entries.len(), 1, "one verdict, one quarantine entry");
    (entries[0].verdict.clone(), datums)
}

fn select_count(store: &Store, q: &str) -> usize {
    match sparql::query(store, q).unwrap() {
        QueryResult::Select { rows, .. } => rows.len(),
        _ => panic!("expected SELECT"),
    }
}

fn verdict_with_outcome(store: &Store, outcome: &str) -> String {
    let q = format!(
        "SELECT ?v WHERE {{ ?v a <{DEFAULT_BASE_NS}Verdict> ; \
         <{DEFAULT_BASE_NS}outcome> \"{outcome}\" }}"
    );
    match sparql::query(store, &q).unwrap() {
        QueryResult::Select { rows, .. } => match rows[0].get("v") {
            Some(Value::Ref(id)) => store.resolve(*id).unwrap(),
            other => panic!("{other:?}"),
        },
        _ => panic!("expected SELECT"),
    }
}

// -- GS2 ----------------------------------------------------------------------

#[test]
fn gs2_holds_quarantined_content_is_invisible_to_queries_and_search() {
    let mut store = signed_store(true);
    let (verdict, _) = refuse(&mut store);

    // Not vacuous: the canary IS retained, in the quarantine.
    let entry = &entries(&store, Some(&verdict)).unwrap()[0];
    let sealed = sealed_delta(&store, &entry.attempt).unwrap().unwrap();
    assert!(sealed.contains(CANARY), "full retention keeps the attempt");

    // ...and nothing that reads the governed graph can see it.
    let q = format!(
        "SELECT ?s WHERE {{ GRAPH ?g {{ ?s ?p ?o }} FILTER(CONTAINS(STR(?o), \"{CANARY}\")) }}"
    );
    assert_eq!(select_count(&store, &q), 0, "SPARQL over every graph");
    let q = format!("SELECT ?s WHERE {{ ?s ?p ?o FILTER(CONTAINS(STR(?o), \"{CANARY}\")) }}");
    assert_eq!(select_count(&store, &q), 0, "SPARQL over the default graph");
    assert_eq!(
        select_count(&store, "SELECT ?p ?o WHERE { <http://ex/doc1> ?p ?o }"),
        0,
        "the refused entity has no facts"
    );
    let found =
        crate::context::tools::tool_unified_search(&store, &serde_json::json!({ "query": CANARY }))
            .unwrap();
    assert_eq!(found["count"], 0, "search: {found}");
    let facts = store.current_facts().unwrap();
    assert!(
        !facts
            .iter()
            .any(|f| matches!(&f.value, Value::Str(s) if s.contains(CANARY))),
        "no current fact carries the canary"
    );
}

#[test]
fn the_sealed_content_never_travels_in_a_pack() {
    assert!(crate::pack_full::excluded().contains(&"quarantine_deltas"));
    // The digests do: they back verdicts that travel, and hold no content.
    assert_eq!(
        crate::share_completeness::disposition("denial_quarantine"),
        Some(crate::share_completeness::Disposition::Content)
    );
}

// -- replay: full retention -----------------------------------------------------

#[test]
fn full_retention_replay_rederives_the_denial() {
    let mut store = signed_store(true);
    let (verdict, _) = refuse(&mut store);
    let replays = replay_verdict(&store, &verdict, None).unwrap();
    assert_eq!(replays.len(), 1);
    let r = &replays[0];
    assert!(
        matches!(
            r.basis,
            Basis::Quarantined {
                delta: "sealed",
                ..
            }
        ),
        "{:?}",
        r.basis
    );
    assert_eq!(r.recorded, "unsatisfied");
    assert_eq!(r.replayed.as_deref(), Some("unsatisfied"));
    assert_eq!(r.refused, Some(true), "the write is refused again");
    assert_eq!(r.same_rules, Some(true));
    assert_eq!(r.same_post_state, Some(true), "byte-identical post-state");
    assert_eq!(r.seal, Some(Seal::Valid));
    assert!(r.rederived() && !r.contradicts(), "{}", r.line());
    // The replay wrote nothing to the live store.
    assert_eq!(
        select_count(&store, "SELECT ?p ?o WHERE { <http://ex/doc1> ?p ?o }"),
        0
    );
}

#[test]
fn a_rule_change_after_the_denial_does_not_move_the_replay() {
    let mut store = signed_store(true);
    let (verdict, datums) = refuse(&mut store);

    // Amend the claim so the same write now PASSES.
    store.set_principal_chain(Vec::new());
    let amend = vec![
        policy_field(&store, "claim", REQUIRE_LABEL, Op::Retract, LATER),
        policy_field(&store, "claim", ANYTHING, Op::Assert, LATER),
    ];
    store.transact(&amend, LATER, None, None).unwrap();
    store
        .transact(&datums, LATER, Some("amaru"), Some("test"))
        .expect("the amended rule admits the write the old one refused");

    // The replay runs as of the denial, under the rule then in force.
    let r = &replay_verdict(&store, &verdict, None).unwrap()[0];
    assert!(r.rederived(), "{}", r.line());
    assert_eq!(r.same_rules, Some(true), "the digest of the OLD rule set");
}

#[test]
fn a_tampered_sealed_delta_is_reported_not_replayed() {
    let mut store = signed_store(true);
    let (verdict, _) = refuse(&mut store);
    store
        .conn
        .execute(
            "UPDATE quarantine_deltas SET delta = replace(delta, ?1, 'forged')",
            [CANARY],
        )
        .unwrap();
    let r = &replay_verdict(&store, &verdict, None).unwrap()[0];
    assert!(
        matches!(r.basis, Basis::AttestationOnly(_)),
        "{:?}",
        r.basis
    );
    assert!(r.contradicts(), "{}", r.line());
}

#[test]
fn a_forged_entry_fails_its_seal() {
    let mut store = signed_store(true);
    let (verdict, _) = refuse(&mut store);
    store
        .conn
        .execute("UPDATE denial_quarantine SET actor = 'someone-else'", [])
        .unwrap();
    let r = &replay_verdict(&store, &verdict, None).unwrap()[0];
    assert_eq!(r.seal, Some(Seal::Invalid));
    assert!(r.contradicts(), "{}", r.line());
}

// -- replay: digest-only ----------------------------------------------------------

#[test]
fn digest_only_verifies_the_right_delta_and_rejects_a_tampered_one() {
    let mut store = signed_store(false);
    let (verdict, datums) = refuse(&mut store);
    let entry = &entries(&store, Some(&verdict)).unwrap()[0];
    assert_eq!(entry.retention, "digest");
    assert!(!entry.sealed_delta, "digest-only keeps no content");

    // Nothing presented: attestation only — incomplete, not contrary.
    let r = &replay_verdict(&store, &verdict, None).unwrap()[0];
    assert!(
        matches!(r.basis, Basis::AttestationOnly(_)),
        "{:?}",
        r.basis
    );
    assert_eq!(r.rules_in_force, Some(true));
    assert!(!r.rederived() && !r.contradicts(), "{}", r.line());

    // The writer's own record of the attempt, presented in canonical form
    // (pretty-printed: the hash is over the canonical form, not the bytes).
    let presented = AttemptedDelta::from_datums(&store, &datums, 0).unwrap();
    let json = serde_json::to_string_pretty(&presented).unwrap();
    let r = &replay_verdict(&store, &verdict, Some(&json)).unwrap()[0];
    assert!(
        matches!(
            r.basis,
            Basis::Quarantined {
                delta: "presented",
                ..
            }
        ),
        "{:?}",
        r.basis
    );
    assert!(r.rederived(), "{}", r.line());

    // One changed character is a different attempt, and is refused as such.
    let tampered = json.replace(CANARY, "QUARANTINE-CANARY-7f3b");
    let err = replay_verdict(&store, &verdict, Some(&tampered)).unwrap_err();
    assert!(
        err.to_string().contains("not the delta the gate judged"),
        "{err}"
    );
}

// -- retention and erasure --------------------------------------------------------

#[test]
fn purge_erases_the_content_and_keeps_the_verdict_and_its_digest() {
    let mut store = signed_store(true);
    let (verdict, datums) = refuse(&mut store);
    let before = entries(&store, Some(&verdict)).unwrap()[0].clone();

    let purged = purge(&store, Some(ROOT_GRAPH_IRI), None, LATER).unwrap();
    assert_eq!(purged, 1);
    let after = entries(&store, Some(&verdict)).unwrap()[0].clone();
    assert_eq!(after.purged_at.as_deref(), Some(LATER));
    assert!(!after.sealed_delta);
    assert!(
        sealed_delta(&store, &after.attempt).unwrap().is_none(),
        "erased"
    );
    assert_eq!(
        after.attempt, before.attempt,
        "the digest survives the purge"
    );
    assert_eq!(after.post_digest, before.post_digest);
    let q = format!("ASK {{ <{verdict}> a <{DEFAULT_BASE_NS}Verdict> }}");
    assert!(matches!(
        sparql::query(&store, &q).unwrap(),
        QueryResult::Ask(true)
    ));

    // "Refused, content purged" — still sealed, still in force, not contrary.
    let r = &replay_verdict(&store, &verdict, None).unwrap()[0];
    match &r.basis {
        Basis::AttestationOnly(why) => assert!(why.contains("content purged"), "{why}"),
        other => panic!("expected attestation only, got {other:?}"),
    }
    assert_eq!(r.seal, Some(Seal::Valid), "purging does not break the seal");
    assert!(!r.contradicts());

    // The digest still verifies a delta someone presents.
    let json = AttemptedDelta::from_datums(&store, &datums, 0)
        .unwrap()
        .canonical();
    assert!(replay_verdict(&store, &verdict, Some(&json)).unwrap()[0].rederived());
}

#[test]
fn purge_by_age_leaves_newer_content() {
    let mut store = signed_store(true);
    refuse(&mut store);
    assert_eq!(
        purge(&store, None, Some("2025-12-31T00:00:00Z"), LATER).unwrap(),
        0
    );
    assert_eq!(
        purge(&store, Some("http://ex/elsewhere"), None, LATER).unwrap(),
        0
    );
    assert!(entries(&store, None).unwrap()[0].sealed_delta);
    assert_eq!(purge(&store, None, Some(LATER), LATER).unwrap(), 1);
    assert!(!entries(&store, None).unwrap()[0].sealed_delta);
}

// -- what is and is not recorded ----------------------------------------------------

#[test]
fn a_disabled_quarantine_records_nothing_and_the_denial_is_attestation_only() {
    let mut store = signed_store(true);
    store.governance_config_mut().quarantine.enabled = false;
    let datums = unlabeled_doc(&store, "http://ex/doc1", CANARY);
    store
        .transact(&datums, TS, Some("amaru"), Some("test"))
        .unwrap_err();
    assert!(entries(&store, None).unwrap().is_empty());
    let verdict = verdict_with_outcome(&store, "unsatisfied");
    let r = &replay_verdict(&store, &verdict, None).unwrap()[0];
    assert!(
        matches!(r.basis, Basis::AttestationOnly(_)),
        "{:?}",
        r.basis
    );
    assert_eq!(r.rules_in_force, Some(true));
    assert!(!r.contradicts());
}

#[test]
fn an_unsigned_store_records_no_entry() {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().enforce_on_write = true;
    define_policy(&mut store);
    let datums = unlabeled_doc(&store, "http://ex/doc1", CANARY);
    store.transact(&datums, TS, None, None).unwrap_err();
    // No verdict (no identity), so nothing for an entry to back.
    assert!(entries(&store, None).unwrap().is_empty());
}

#[test]
fn an_accepted_write_is_never_quarantined() {
    let mut store = signed_store(true);
    let datums = labeled_doc(&store, "http://ex/doc2");
    store.transact(&datums, TS, Some("amaru"), None).unwrap();
    assert!(entries(&store, None).unwrap().is_empty());
}

// -- replay: committed verdicts keep working ------------------------------------------

#[test]
fn a_satisfied_verdict_replays_from_its_committed_write() {
    let mut store = signed_store(false);
    let datums = labeled_doc(&store, "http://ex/doc2");
    let write_tx = store.transact(&datums, TS, Some("amaru"), None).unwrap();
    let verdict = verdict_with_outcome(&store, "satisfied");
    // Retract the label afterwards: the replay must still judge as of the write.
    let retract = vec![datum(
        &store,
        "http://ex/doc2",
        RDFS_LABEL,
        Value::Str("a doc".into()),
        Op::Retract,
        LATER,
    )];
    store.governance_config_mut().enforce_on_write = false;
    store.transact(&retract, LATER, None, None).unwrap();

    let r = &replay_verdict(&store, &verdict, None).unwrap()[0];
    assert_eq!(r.basis, Basis::Committed { write_tx });
    assert_eq!(r.replayed.as_deref(), Some("satisfied"));
    assert!(r.rederived(), "{}", r.line());
}

#[test]
fn an_unknown_verdict_is_an_error() {
    let store = signed_store(false);
    let err = replay_verdict(&store, "verdict_nope", None).unwrap_err();
    assert!(err.to_string().contains("no aegis:Verdict"), "{err}");
}

// -- the canonical form -----------------------------------------------------------

#[test]
fn the_canonical_delta_round_trips_every_value_kind() {
    let store = Store::open_in_memory().unwrap();
    let values = [
        Value::Ref(store.intern("http://ex/o").unwrap()),
        Value::Str("s".into()),
        Value::Int(-3),
        Value::Float(1.5),
        Value::Bool(true),
        Value::Bytes(vec![0, 255]),
        Value::Lang {
            lexical: "hi".into(),
            lang: "en".into(),
        },
        Value::Typed {
            lexical: "2026-01-01".into(),
            datatype: "http://www.w3.org/2001/XMLSchema#date".into(),
        },
    ];
    let datums: Vec<Datum> = values
        .iter()
        .map(|v| {
            datum(
                &store,
                "http://ex/s",
                "http://ex/p",
                v.clone(),
                Op::Assert,
                TS,
            )
        })
        .collect();
    let delta = AttemptedDelta::from_datums(&store, &datums, 0).unwrap();
    let parsed = AttemptedDelta::parse(&delta.canonical()).unwrap();
    assert_eq!(parsed, delta);
    assert_eq!(parsed.hash(), delta.hash());
    let (back, graph) = parsed.to_datums(&store).unwrap();
    assert_eq!(graph, 0);
    for (a, b) in back.iter().zip(&datums) {
        assert_eq!(a.value, b.value);
        assert_eq!((a.entity, a.attribute, a.op), (b.entity, b.attribute, b.op));
    }
    assert!(AttemptedDelta::parse(r#"{"format":"other","graph":"g","datums":[]}"#).is_err());
}
