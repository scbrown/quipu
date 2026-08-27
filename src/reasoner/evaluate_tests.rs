//! Integration tests for the reasoner evaluator.
//!
//! These exercise the full round-trip: load rules, seed the EAVT store
//! with base facts, call [`evaluate`], and assert that the derived facts
//! show up in `current_facts` with the expected `reasoner:<rule-id>`
//! provenance tag.

use super::parse::parse_rules;
use super::{RULE_NS, evaluate, evaluate_in_graph};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

const PFX: &str = "http://ex/";
const TS: &str = "2026-04-07T00:00:00Z";

/// Intern a predicate and assert a `(subject, predicate, object)` triple
/// into the store.
fn assert_triple(store: &mut Store, subject: &str, predicate: &str, object: &str) -> i64 {
    let s = store.intern(subject).expect("intern subject");
    let p = store.intern(predicate).expect("intern predicate");
    let o = store.intern(object).expect("intern object");
    let datum = Datum {
        entity: s,
        attribute: p,
        value: Value::Ref(o),
        valid_from: TS.to_string(),
        valid_to: None,
        op: Op::Assert,
    };
    store
        .transact(&[datum], TS, Some("test"), Some("base"))
        .expect("transact base fact");
    o
}

/// Count facts whose attribute matches `predicate` and whose transaction
/// source matches `source`.
fn count_derived(store: &Store, predicate: &str, source: &str) -> usize {
    let attr = store
        .lookup(predicate)
        .expect("lookup")
        .expect("predicate should exist after derivation");
    let mut stmt = store
        .conn
        .prepare(
            "SELECT COUNT(*) FROM facts f \
             JOIN transactions t ON f.tx = t.id \
             WHERE f.a = ?1 AND t.source = ?2 \
               AND f.op = 1 AND f.valid_to IS NULL",
        )
        .unwrap();
    let count: i64 = stmt
        .query_row(rusqlite::params![attr, source], |row| row.get(0))
        .unwrap();
    usize::try_from(count).expect("non-negative count")
}

#[test]
fn empty_ruleset_is_a_noop() {
    let mut store = Store::open_in_memory().unwrap();
    let rs = super::parse::RuleSet::empty(PFX);
    let report = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(report.asserted, 0);
    assert_eq!(report.retracted, 0);
    assert_eq!(report.strata_run, 0);
}

#[test]
fn single_atom_projection_derives_facts() {
    // Rule: `h(?x, ?y) :- p(?x, ?y)` — simple projection.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();

    let mut store = Store::open_in_memory().unwrap();
    assert_triple(&mut store, "ex:a", &format!("{PFX}p"), "ex:b");
    assert_triple(&mut store, "ex:c", &format!("{PFX}p"), "ex:d");

    let report = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(report.asserted, 2);
    assert_eq!(report.retracted, 0);
    assert_eq!(count_derived(&store, &format!("{PFX}h"), "reasoner:R1"), 2);
}

#[test]
fn graph_scoped_evaluation_reads_and_writes_only_that_graph() {
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    assert_triple(&mut store, "ex:root-a", &format!("{PFX}p"), "ex:root-b");

    let graph = store.overlay_create("http://ex/branch", 0).unwrap();
    let branch_a = store.intern("ex:branch-a").unwrap();
    let branch_b = store.intern("ex:branch-b").unwrap();
    let predicate = store.lookup(&format!("{PFX}p")).unwrap().unwrap();
    store
        .overlay_write(
            graph,
            Op::Assert,
            branch_a,
            predicate,
            Value::Ref(branch_b),
            TS,
        )
        .unwrap();

    let report = evaluate_in_graph(&mut store, &rs, TS, graph).unwrap();
    assert_eq!(report.asserted, 1);
    let head = store.lookup(&format!("{PFX}h")).unwrap().unwrap();
    let branch_facts = store.current_facts_in_graph(graph).unwrap();
    assert!(branch_facts.iter().any(|fact| {
        fact.entity == branch_a && fact.attribute == head && fact.value == Value::Ref(branch_b)
    }));
    assert!(
        !store
            .current_facts()
            .unwrap()
            .iter()
            .any(|fact| fact.attribute == head),
        "branch derivations must not be written into ROOT"
    );
}

#[test]
fn two_atom_join_derives_cross_product_via_shared_var() {
    // `affects(?pkg, ?svc) :- installedIn(?pkg, ?c), runsService(?c, ?svc)`
    // With one package in a container running two services, we expect two
    // affects tuples.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "affects(?pkg, ?svc)" ;
    rule:body "installedIn(?pkg, ?c), runsService(?c, ?svc)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();

    let mut store = Store::open_in_memory().unwrap();
    assert_triple(
        &mut store,
        "ex:nginx",
        &format!("{PFX}installedIn"),
        "ex:ctA",
    );
    assert_triple(
        &mut store,
        "ex:ctA",
        &format!("{PFX}runsService"),
        "ex:proxy",
    );
    assert_triple(&mut store, "ex:ctA", &format!("{PFX}runsService"), "ex:api");

    let report = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(report.asserted, 2);
    assert_eq!(
        count_derived(&store, &format!("{PFX}affects"), "reasoner:R1"),
        2
    );
}

#[test]
fn recursive_rule_computes_transitive_closure() {
    // `dependsOn(?a, ?c) :- dependsOn(?a, ?b), dependsOn(?b, ?c)`
    // Seeded with a→b and b→c, the closure also gives us a→c.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "dependsOn(?a, ?c)" ;
    rule:body "dependsOn(?a, ?b), dependsOn(?b, ?c)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();

    let mut store = Store::open_in_memory().unwrap();
    assert_triple(&mut store, "ex:a", &format!("{PFX}dependsOn"), "ex:b");
    assert_triple(&mut store, "ex:b", &format!("{PFX}dependsOn"), "ex:c");

    let report = evaluate(&mut store, &rs, TS).unwrap();
    // Only the (a,c) closure tuple is new — (a,b) and (b,c) are base facts.
    assert_eq!(report.asserted, 1);

    // Re-running should be a no-op: the derivation is already persisted.
    let second = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(second.asserted, 0);
    assert_eq!(second.retracted, 0);
}

#[test]
fn retracted_base_fact_triggers_retraction_of_derived_fact() {
    // Derive `h(?x, ?y) :- p(?x, ?y)`, then retract the single base fact
    // and re-run. The derived fact must be retracted by the second call.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x, ?y)" ;
    rule:body "p(?x, ?y)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();

    let mut store = Store::open_in_memory().unwrap();
    let a = store.intern("ex:a").unwrap();
    let b = store.intern("ex:b").unwrap();
    let p = store.intern(&format!("{PFX}p")).unwrap();
    store
        .transact(
            &[Datum {
                entity: a,
                attribute: p,
                value: Value::Ref(b),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Assert,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    let first = evaluate(&mut store, &rs, TS).unwrap();
    assert_eq!(first.asserted, 1);

    // Retract the base fact. Use a different timestamp so valid-time ranges
    // don't collapse.
    let ts2 = "2026-04-07T01:00:00Z";
    store
        .transact(
            &[Datum {
                entity: a,
                attribute: p,
                value: Value::Ref(b),
                valid_from: ts2.to_string(),
                valid_to: None,
                op: Op::Retract,
            }],
            ts2,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    let second = evaluate(&mut store, &rs, ts2).unwrap();
    assert_eq!(second.asserted, 0);
    assert_eq!(second.retracted, 1);
    assert_eq!(count_derived(&store, &format!("{PFX}h"), "reasoner:R1"), 0);
}

// ── Constants in body atoms (aegis-jgxas) ─────────────────────

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const COMMIT: &str = "http://aegis.gastown.local/ontology/Commit";
const GIT_COMMIT: &str = "http://aegis.gastown.local/ontology/GitCommit";

/// The class-equivalence rule the aegis ontology needs:
/// `rdf:type(?x, GitCommit) :- rdf:type(?x, Commit)`.
fn class_equivalence_ttl() -> String {
    format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:eq a rule:Rule ;
    rule:id "EQ" ;
    rule:head "<{RDF_TYPE}>(?x, <{GIT_COMMIT}>)" ;
    rule:body "<{RDF_TYPE}>(?x, <{COMMIT}>)" .
"#
    )
}

#[test]
fn type_atom_rule_derives_the_equivalent_class() {
    let ttl = class_equivalence_ttl();
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    // Two entities carrying the OLD vocab, one carrying an unrelated type.
    assert_triple(&mut store, "ex:c1", RDF_TYPE, COMMIT);
    assert_triple(&mut store, "ex:c2", RDF_TYPE, COMMIT);
    assert_triple(&mut store, "ex:p1", RDF_TYPE, "http://ex/Person");
    // GitCommit must already be interned for a head constant to resolve.
    assert_triple(&mut store, "ex:g1", RDF_TYPE, GIT_COMMIT);

    let report = evaluate(&mut store, &rs, TS).expect("type-atom rule must evaluate");

    assert_eq!(
        report.asserted, 2,
        "exactly the two Commit entities should gain GitCommit, got {}",
        report.asserted
    );
    assert_eq!(count_derived(&store, RDF_TYPE, "reasoner:EQ"), 2);
}

#[test]
fn type_atom_rule_does_not_over_derive_across_other_types() {
    // The failure mode a naive fix introduces: ignoring the body constant
    // makes EVERY typed entity match, so Person/GitCommit entities would be
    // retyped as GitCommit too. This test is the guard on that.
    let ttl = class_equivalence_ttl();
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    assert_triple(&mut store, "ex:c1", RDF_TYPE, COMMIT);
    let person = assert_triple(&mut store, "ex:p1", RDF_TYPE, "http://ex/Person");
    assert_triple(&mut store, "ex:g1", RDF_TYPE, GIT_COMMIT);

    evaluate(&mut store, &rs, TS).expect("type-atom rule must evaluate");

    let git_commit_id = store.lookup(GIT_COMMIT).unwrap().unwrap();
    let rdf_type_id = store.lookup(RDF_TYPE).unwrap().unwrap();
    let p1 = store.lookup("ex:p1").unwrap().unwrap();
    let _ = person;

    let derived: Vec<i64> = store
        .current_facts()
        .unwrap()
        .into_iter()
        .filter(|f| f.attribute == rdf_type_id && f.value == Value::Ref(git_commit_id))
        .map(|f| f.entity)
        .collect();

    assert!(
        !derived.contains(&p1),
        "ex:p1 is a Person and must NOT be derived as a GitCommit — \
         body constant was ignored (over-derivation)"
    );
    assert_eq!(
        count_derived(&store, RDF_TYPE, "reasoner:EQ"),
        1,
        "only ex:c1 should have been derived"
    );
}

#[test]
fn body_constant_in_the_subject_position_also_filters() {
    // `p(?y, ?x) :- q(<ex:root>, ?x)` style — constant in slot 0.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r a rule:Rule ;
    rule:id "SUBJ" ;
    rule:head "h(?y, ?y)" ;
    rule:body "p(<http://ex/root>, ?y)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    assert_triple(&mut store, "http://ex/root", &format!("{PFX}p"), "ex:a");
    assert_triple(&mut store, "http://ex/other", &format!("{PFX}p"), "ex:b");

    let report = evaluate(&mut store, &rs, TS).expect("subject-constant rule must evaluate");
    assert_eq!(
        report.asserted, 1,
        "only the root-rooted fact should match, got {}",
        report.asserted
    );
}

#[test]
fn body_constant_filters_before_a_later_stratum_reads_the_derivation() {
    // Reaches the datafrog path specifically. `project_rule_body` re-applies
    // the constant filter when attributing tuples to a rule, which MASKS an
    // unfiltered fixpoint in rule A's own output. But the unfiltered relation
    // is still drained into `world`, so a rule in a LATER stratum reading A's
    // head predicate sees the over-derivation. Without this test, dropping the
    // PrefixFilter in `step_one_atom` passes the whole suite.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:a a rule:Rule ;
    rule:id "A" ;
    rule:head "matched(?x, ?x)" ;
    rule:body "<{RDF_TYPE}>(?x, <{COMMIT}>)" .

ex:b a rule:Rule ;
    rule:id "B" ;
    rule:head "flagged(?x, ?y)" ;
    rule:body "matched(?x, ?y)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    assert_triple(&mut store, "ex:c1", RDF_TYPE, COMMIT);
    assert_triple(&mut store, "ex:p1", RDF_TYPE, "http://ex/Person");
    assert_triple(&mut store, "ex:p2", RDF_TYPE, "http://ex/Person");

    evaluate(&mut store, &rs, TS).expect("two-stratum rule set must evaluate");

    assert_eq!(
        count_derived(&store, &format!("{PFX}matched"), "reasoner:A"),
        1,
        "only ex:c1 is a Commit"
    );
    assert_eq!(
        count_derived(&store, &format!("{PFX}flagged"), "reasoner:B"),
        1,
        "the later stratum must not see over-derived `matched` tuples — \
         the fixpoint ignored the body constant"
    );
}

#[test]
fn body_constant_that_was_never_interned_derives_nothing() {
    // Not an error: a term that does not exist cannot appear in any fact, so
    // the empty set is the correct answer. The failure mode being guarded is
    // the opposite one — treating an unresolvable constant as "no filter" and
    // therefore matching everything.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:eq a rule:Rule ;
    rule:id "GHOST" ;
    rule:head "<{RDF_TYPE}>(?x, <{GIT_COMMIT}>)" ;
    rule:body "<{RDF_TYPE}>(?x, <http://ex/NeverInterned>)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    assert_triple(&mut store, "ex:c1", RDF_TYPE, COMMIT);
    assert_triple(&mut store, "ex:g1", RDF_TYPE, GIT_COMMIT);

    let report = evaluate(&mut store, &rs, TS).expect("unresolvable constant is not an error");
    assert_eq!(
        report.asserted, 0,
        "a body constant with no term id must match nothing, not everything"
    );
}

#[test]
fn mutual_class_equivalence_converges_under_retraction() {
    // REGRESSION GUARD, formerly `probe_mutual_class_equivalence_under_retraction`
    // — a probe that PINNED the failure this now guards against (aegis-jgxas,
    // fixed under quipu-923).
    //
    // Class equivalence is symmetric, so the obvious encoding is two rules:
    //   type(?x, GitCommit) :- type(?x, Commit)
    //   type(?x, Commit)    :- type(?x, GitCommit)
    // These are mutually recursive. `World::load` used to read
    // `current_facts()`, which did not distinguish base facts from derived
    // ones, so each rule's stored output was the other's input and the pair
    // held itself up forever after the base fact that started it was gone —
    // a stable non-converging fixpoint of (1, 2) derivations.
    //
    // The fix: `World::load` reads BASE facts only
    // (`current_base_facts_for_attributes_in_graph`); prior derivations are
    // diff targets in `write_rule_delta`, never premises. In-stratum chaining
    // still works — the first evaluation below still derives through both
    // rules — but retraction now propagates.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:fwd a rule:Rule ;
    rule:id "FWD" ;
    rule:head "<{RDF_TYPE}>(?x, <{GIT_COMMIT}>)" ;
    rule:body "<{RDF_TYPE}>(?x, <{COMMIT}>)" .

ex:rev a rule:Rule ;
    rule:id "REV" ;
    rule:head "<{RDF_TYPE}>(?x, <{COMMIT}>)" ;
    rule:body "<{RDF_TYPE}>(?x, <{GIT_COMMIT}>)" .
"#
    );
    let rs = parse_rules(&ttl, Some(PFX)).unwrap();
    let mut store = Store::open_in_memory().unwrap();

    // Intern both classes, then give ex:c1 only the old vocab.
    assert_triple(&mut store, "ex:seed", RDF_TYPE, GIT_COMMIT);
    assert_triple(&mut store, "ex:c1", RDF_TYPE, COMMIT);

    evaluate(&mut store, &rs, TS).expect("mutual equivalence must evaluate");
    assert_eq!(
        count_derived(&store, RDF_TYPE, "reasoner:FWD"),
        1,
        "ex:c1 should gain GitCommit"
    );

    // Now retract the BASE fact that was ex:c1's only support.
    let c1 = store.lookup("ex:c1").unwrap().unwrap();
    let rdf_type = store.lookup(RDF_TYPE).unwrap().unwrap();
    let commit = store.lookup(COMMIT).unwrap().unwrap();
    store
        .transact(
            &[Datum {
                entity: c1,
                attribute: rdf_type,
                value: Value::Ref(commit),
                valid_from: TS.to_string(),
                valid_to: None,
                op: Op::Retract,
            }],
            TS,
            Some("test"),
            Some("base"),
        )
        .unwrap();

    // Re-evaluate repeatedly: the pair must converge to "support gone,
    // derivation gone" — and stay there.
    let mut trace = Vec::new();
    for _ in 0..4 {
        evaluate(&mut store, &rs, TS).expect("re-evaluate after retraction");
        trace.push((
            count_derived(&store, RDF_TYPE, "reasoner:FWD"),
            count_derived(&store, RDF_TYPE, "reasoner:REV"),
        ));
    }

    // ex:seed's GitCommit is base, so REV keeps deriving seed→Commit (1).
    // Everything that rested on the retracted fact is gone from the first
    // re-evaluation on: FWD derives nothing, and REV's c1→Commit is retracted.
    assert_eq!(
        trace,
        vec![(0, 1); 4],
        "retraction must propagate through mutually supporting rules, got {trace:?}"
    );

    // What convergence MEANS: ex:c1's only base type was retracted, and no
    // derived type survives it.
    let git_commit = store.lookup(GIT_COMMIT).unwrap().unwrap();
    let still: Vec<i64> = store
        .current_facts()
        .unwrap()
        .into_iter()
        .filter(|f| {
            f.attribute == rdf_type
                && (f.value == Value::Ref(git_commit) || f.value == Value::Ref(commit))
        })
        .map(|f| f.entity)
        .collect();
    assert_eq!(
        still.iter().filter(|e| **e == c1).count(),
        0,
        "ex:c1 lost its only base support, so it must carry neither class"
    );
}
