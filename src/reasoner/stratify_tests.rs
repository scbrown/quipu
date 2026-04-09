//! Tests for the reasoner stratifier.

use super::parse::parse_rules;
use super::stratify::{Strata, head_predicates_at, rule_at_index, stratify};
use super::{RULE_NS, ReasonerError};

const PFX: &str = "http://ex/";

fn load(ttl: &str) -> super::parse::RuleSet {
    parse_rules(ttl, Some(PFX)).expect("parse rules")
}

/// Assert that a stratum set contains exactly the given head predicate IRIs.
fn assert_heads(strata: &Strata, ruleset: &super::parse::RuleSet, level: usize, want: &[&str]) {
    let mut got = head_predicates_at(strata, ruleset, level);
    got.sort();
    let mut want_owned: Vec<String> = want.iter().map(|s| format!("{PFX}{s}")).collect();
    want_owned.sort();
    assert_eq!(
        got, want_owned,
        "stratum {level} head predicates did not match"
    );
}

#[test]
fn single_non_recursive_rule_has_one_stratum() {
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h(?x)" ;
    rule:body "p(?x)" .
"#
    );
    let rs = load(&ttl);
    let strata = stratify(&rs).unwrap();
    // Level 0 is empty (only extensional predicate `p` sits there), level
    // 1 holds the single rule whose head is `h`.
    assert_eq!(strata.level_count(), 2);
    assert!(strata.levels[0].is_empty());
    assert_heads(&strata, &rs, 1, &["h"]);
    assert_eq!(strata.predicate_stratum[&format!("{PFX}p")], 0);
    assert_eq!(strata.predicate_stratum[&format!("{PFX}h")], 1);
}

#[test]
fn chain_rules_produce_rising_strata() {
    // h1 <- p
    // h2 <- h1
    // h3 <- h2
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "h1(?x)" ;
    rule:body "p(?x)" .

ex:r2 a rule:Rule ;
    rule:id "R2" ;
    rule:head "h2(?x)" ;
    rule:body "h1(?x)" .

ex:r3 a rule:Rule ;
    rule:id "R3" ;
    rule:head "h3(?x)" ;
    rule:body "h2(?x)" .
"#
    );
    let rs = load(&ttl);
    let strata = stratify(&rs).unwrap();
    // Extensional predicate `p` → stratum 0. h1 → 1, h2 → 2, h3 → 3.
    assert_eq!(strata.predicate_stratum[&format!("{PFX}p")], 0);
    assert_eq!(strata.predicate_stratum[&format!("{PFX}h1")], 1);
    assert_eq!(strata.predicate_stratum[&format!("{PFX}h2")], 2);
    assert_eq!(strata.predicate_stratum[&format!("{PFX}h3")], 3);
    // Rule indices are grouped by their head's stratum.
    assert_eq!(strata.levels.len(), 4);
    assert!(strata.levels[0].is_empty());
    assert_eq!(strata.levels[1].len(), 1);
    assert_eq!(strata.levels[2].len(), 1);
    assert_eq!(strata.levels[3].len(), 1);
    assert_eq!(rule_at_index(&rs, strata.levels[1][0]).id, "R1");
    assert_eq!(rule_at_index(&rs, strata.levels[3][0]).id, "R3");
}

#[test]
fn positive_self_recursion_collapses_to_one_stratum() {
    // dependsOn is transitive — a classic positive-recursive rule. It must
    // stratify cleanly because datafrog handles positive recursion via
    // semi-naive evaluation.
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
    let rs = load(&ttl);
    let strata = stratify(&rs).unwrap();
    // dependsOn is in a non-trivial SCC with itself — one stratum, one rule.
    assert_eq!(strata.level_count(), 1);
    assert_heads(&strata, &rs, 0, &["dependsOn"]);
}

#[test]
fn mutual_positive_recursion_stays_in_single_stratum() {
    // a ↔ b mutually recursive but purely positive — legal.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "a(?x)" ;
    rule:body "b(?x)" .

ex:r2 a rule:Rule ;
    rule:id "R2" ;
    rule:head "b(?x)" ;
    rule:body "a(?x)" .
"#
    );
    let rs = load(&ttl);
    let strata = stratify(&rs).unwrap();
    // a and b share a stratum (they are in the same SCC).
    assert_eq!(
        strata.predicate_stratum[&format!("{PFX}a")],
        strata.predicate_stratum[&format!("{PFX}b")]
    );
    assert_eq!(strata.level_count(), 1);
    assert_eq!(strata.levels[0].len(), 2);
}

#[test]
fn negation_between_strata_is_allowed() {
    // atRisk(?s) :- dependsOn(?s, ?x), not validated(?x).
    // Negation references a lower stratum — legal stratified negation.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "atRisk(?s)" ;
    rule:body "dependsOn(?s, ?x), not validated(?x)" .
"#
    );
    let rs = load(&ttl);
    let strata = stratify(&rs).unwrap();
    // `validated` is extensional → stratum 0. `atRisk` sits one above.
    assert_eq!(strata.predicate_stratum[&format!("{PFX}validated")], 0);
    assert!(strata.predicate_stratum[&format!("{PFX}atRisk")] > 0);
}

#[test]
fn self_negation_cycle_is_rejected() {
    // `p :- not p` — classic non-stratifiable cycle.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "p(?x)" ;
    rule:body "q(?x), not p(?x)" .
"#
    );
    let rs = load(&ttl);
    let err = stratify(&rs).unwrap_err();
    match err {
        ReasonerError::UnstratifiableCycle { predicates } => {
            assert!(
                predicates.iter().any(|p| p == &format!("{PFX}p")),
                "expected p in cycle, got {predicates:?}"
            );
        }
        other => panic!("expected UnstratifiableCycle, got {other:?}"),
    }
}

#[test]
fn mutual_negation_cycle_is_rejected() {
    // p :- not q, q :- not p — the textbook non-stratifiable pair.
    let ttl = format!(
        r#"
@prefix rule: <{RULE_NS}> .
@prefix ex: <http://example.org/rules/> .

ex:r1 a rule:Rule ;
    rule:id "R1" ;
    rule:head "p(?x)" ;
    rule:body "r(?x), not q(?x)" .

ex:r2 a rule:Rule ;
    rule:id "R2" ;
    rule:head "q(?x)" ;
    rule:body "r(?x), not p(?x)" .
"#
    );
    let rs = load(&ttl);
    let err = stratify(&rs).unwrap_err();
    match err {
        ReasonerError::UnstratifiableCycle { predicates } => {
            // Both p and q must be named in the cycle.
            assert!(predicates.iter().any(|pred| pred == &format!("{PFX}p")));
            assert!(predicates.iter().any(|pred| pred == &format!("{PFX}q")));
        }
        other => panic!("expected UnstratifiableCycle, got {other:?}"),
    }
}

#[test]
fn empty_ruleset_stratifies_to_empty_levels() {
    let rs = super::parse::RuleSet::empty(PFX);
    let strata = stratify(&rs).unwrap();
    assert!(strata.levels.is_empty());
    assert!(strata.predicate_stratum.is_empty());
}
