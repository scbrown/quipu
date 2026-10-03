//! Tests for the label lattice (quipu #66).
//!
//! The centrepiece is [`homomorphism`]: `label(A ∪ B) = label(A) ⊓ label(B)`.
//! Everything else pins a specific rule the design states in prose.

use super::*;

#[test]
fn durability_meet_is_the_least_recoverable_input_and_unknown_is_absent() {
    assert_eq!(
        Durability::Backed.meet(&Durability::SoleRecord).unwrap(),
        Durability::SoleRecord
    );
    assert_eq!(
        Durability::parse("reproducible"),
        Some(Durability::Reproducible)
    );
    assert_eq!(Durability::parse("unknown"), None);
}
use super::props::{self, arb_graphset, arb_world};
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Freshness
// ---------------------------------------------------------------------------

#[test]
fn freshness_orders_fresh_above_recomputing_above_stale() {
    assert!(Freshness::Fresh > Freshness::Recomputing);
    assert!(Freshness::Recomputing > Freshness::Stale);
}

#[test]
fn freshness_meet_takes_the_weakest() {
    let m = Freshness::Fresh.meet(&Freshness::Stale).unwrap();
    assert_eq!(
        m,
        Freshness::Stale,
        "a union is only as fresh as its weakest"
    );
    let m = Freshness::Fresh.meet(&Freshness::Recomputing).unwrap();
    assert_eq!(m, Freshness::Recomputing);
}

#[test]
fn recomputing_collapses_to_stale_never_to_fresh() {
    // §2: the conservative reading is the only one that cannot overstate.
    assert_eq!(Freshness::Recomputing.collapse_binary(), Freshness::Stale);
    assert_eq!(Freshness::Fresh.collapse_binary(), Freshness::Fresh);
    assert_eq!(Freshness::Stale.collapse_binary(), Freshness::Stale);
}

#[test]
fn unknown_freshness_string_is_not_silently_fresh() {
    assert_eq!(Freshness::parse("fresh"), Some(Freshness::Fresh));
    assert_eq!(Freshness::parse("FRESH"), None, "no case-folding fallback");
    assert_eq!(Freshness::parse("very-fresh"), None);
    assert_eq!(Freshness::parse(""), None);
}

// ---------------------------------------------------------------------------
// Trust — including the refusal that is the whole point of the axis
// ---------------------------------------------------------------------------

fn t(iri: &str, chain: &str, rank: i64) -> Trust {
    Trust::new(iri, chain, rank)
}

#[test]
fn trust_meet_within_a_chain_takes_the_lower_rank() {
    let a = t("ex:canonical", "ex:chain", 40);
    let b = t("ex:learned", "ex:chain", 10);
    assert_eq!(a.meet(&b).unwrap(), b, "the less-trusted member wins");
    assert_eq!(b.meet(&a).unwrap(), b, "and it is commutative");
}

#[test]
fn cross_chain_trust_comparison_errors_naming_both_chains() {
    // #66 acceptance. A silent int compare here is how "learned tactic beats
    // canonical" ships.
    let a = t("smac:canonical", "smac:ruleTierChain", 40);
    let b = t("hank:attested", "hank:tierChain", 10);
    let err = a.meet(&b).expect_err("cross-chain must refuse");
    let msg = err.to_string();
    assert!(
        msg.contains("smac:ruleTierChain"),
        "names the first chain: {msg}"
    );
    assert!(
        msg.contains("hank:tierChain"),
        "names the second chain: {msg}"
    );
    assert!(
        msg.contains("refused") || msg.contains("comparable"),
        "says what it refused to do: {msg}"
    );
}

#[test]
fn cross_chain_refusal_is_not_rescued_by_equal_ranks() {
    // Equal ints across chains is the case most likely to look "obviously
    // fine" to a future simplifier. It is still a category error.
    let a = t("smac:canonical", "chain:a", 10);
    let b = t("hank:attested", "chain:b", 10);
    assert!(
        a.meet(&b).is_err(),
        "equal ranks do not make chains comparable"
    );
}

#[test]
fn cross_chain_error_propagates_out_of_the_fold() {
    let items = vec![Some(t("a", "chain:a", 5)), Some(t("b", "chain:b", 5))];
    assert!(
        fold_meet(items).is_err(),
        "the fold must not swallow the refusal"
    );
}

// ---------------------------------------------------------------------------
// Policy — the join direction
// ---------------------------------------------------------------------------

#[test]
fn policy_join_is_union_so_obligations_accumulate() {
    let a = PolicyClass::new(["pii"]);
    let b = PolicyClass::new(["no-export"]);
    let j = a.join(&b).unwrap();
    assert!(j.contains("pii") && j.contains("no-export"));
    assert_eq!(j.tokens().len(), 2);
}

#[test]
fn policy_join_never_drops_a_restriction() {
    // The sign error this module's naming exists to prevent: composing
    // obligations by intersection would silently drop `no-export`.
    let tainted = PolicyClass::new(["no-export"]);
    let clean = PolicyClass::empty();
    let j = tainted.join(&clean).unwrap();
    assert!(
        j.contains("no-export"),
        "a clean graph must not launder a restricted one"
    );
}

// ---------------------------------------------------------------------------
// Authority — behaviour must be unchanged
// ---------------------------------------------------------------------------

#[test]
fn authority_meet_is_exactly_intersect() {
    let a = Authority::over(["g:1", "g:2"]);
    let b = Authority::over(["g:2", "g:3"]);
    assert_eq!(a.meet(&b).unwrap(), a.intersect(&b));
    assert_eq!(
        Authority::any().meet(&a).unwrap(),
        a.intersect(&Authority::any())
    );
    assert_eq!(
        Authority::none().meet(&a).unwrap(),
        a.intersect(&Authority::none())
    );
}

#[test]
fn authority_wildcard_is_the_meet_identity() {
    let a = Authority::over(["g:1"]);
    assert_eq!(Authority::any().meet(&a).unwrap(), a);
    assert_eq!(a.meet(&Authority::any()).unwrap(), a);
}

// ---------------------------------------------------------------------------
// Coverage
// ---------------------------------------------------------------------------

#[test]
fn coverage_empty_is_the_identity() {
    for c in [
        Coverage::Empty,
        Coverage::None,
        Coverage::Partial,
        Coverage::Full,
    ] {
        assert_eq!(Coverage::Empty.compose(c), c);
        assert_eq!(c.compose(Coverage::Empty), c);
    }
}

#[test]
fn any_mix_of_declared_and_undeclared_is_partial() {
    assert_eq!(Coverage::Full.compose(Coverage::None), Coverage::Partial);
    assert_eq!(Coverage::None.compose(Coverage::Full), Coverage::Partial);
    assert_eq!(Coverage::Full.compose(Coverage::Partial), Coverage::Partial);
    assert_eq!(Coverage::None.compose(Coverage::Partial), Coverage::Partial);
}

#[test]
fn coverage_pure_states_are_preserved() {
    assert_eq!(Coverage::Full.compose(Coverage::Full), Coverage::Full);
    assert_eq!(Coverage::None.compose(Coverage::None), Coverage::None);
}

#[test]
fn only_full_coverage_satisfies_a_floor() {
    // §2.1 / #68: fail-safe at enforcement. Partial and none must NOT pass.
    assert!(Coverage::Full.is_full());
    assert!(!Coverage::Partial.is_full());
    assert!(!Coverage::None.is_full());
    assert!(!Coverage::Empty.is_full());
}

// ---------------------------------------------------------------------------
// The fold
// ---------------------------------------------------------------------------

#[test]
fn unlabelled_dataset_is_undeclared_never_fabricated() {
    // #65 acceptance, resting on this: never a fabricated fresh/⊤/⊥.
    let items: Vec<Option<Freshness>> = vec![None, None, None];
    let c = fold_meet(items).unwrap();
    assert!(c.is_undeclared(), "no value may be invented");
    assert_eq!(c.coverage, Coverage::None);
}

#[test]
fn empty_dataset_folds_to_the_identity_not_to_none() {
    let items: Vec<Option<Freshness>> = vec![];
    let c = fold_meet(items).unwrap();
    assert_eq!(
        c.coverage,
        Coverage::Empty,
        "no graphs at all is distinct from graphs that declared nothing"
    );
    assert!(c.is_undeclared());
}

#[test]
fn one_stale_graph_drags_the_dataset_stale() {
    // Conservative by construction (§4): the stale graph need not have
    // contributed a single returned row.
    let items = vec![
        Some(Freshness::Fresh),
        Some(Freshness::Fresh),
        Some(Freshness::Stale),
    ];
    let c = fold_meet(items).unwrap();
    assert_eq!(c.value, Some(Freshness::Stale));
    assert_eq!(c.coverage, Coverage::Full);
}

#[test]
fn undeclared_members_move_coverage_but_never_the_value() {
    let items = vec![Some(Freshness::Fresh), None];
    let c = fold_meet(items).unwrap();
    assert_eq!(
        c.value,
        Some(Freshness::Fresh),
        "an undeclared graph must not drag the value to the floor"
    );
    assert_eq!(
        c.coverage,
        Coverage::Partial,
        "but it must be visible in coverage — silence must not flatter"
    );
}

// ---------------------------------------------------------------------------
// The homomorphism — #66's headline acceptance
// ---------------------------------------------------------------------------

/// The real freshness axis, every value.
const FRESHNESS: &[Freshness] = &[Freshness::Stale, Freshness::Recomputing, Freshness::Fresh];

// The laws live in `lattice_props.rs`, generic over the axis, so the evidence
// harness (`lattice_evidence_tests.rs`) runs the same code (aegis-xfuch4.4).
//
// The labelling: graph id -> its declared label. **A function**, and that is
// load-bearing rather than incidental.
//
// The first version of this test let A and B each carry their own label for
// the same graph id, and the proptest refuted the homomorphism in four cases
// — correctly. `{1: stale} ∪ {1: undeclared}` is not a world that can exist:
// a graph has exactly one label, held once in the meta-graph. The law is
// about two *datasets over one labelling*, not two independent labellings.
//
// What makes that true at runtime is #65's `shapes/graph-labels.ttl`
// (`minCount`/`maxCount 1` on the label predicates). So the SHACL cardinality
// is not decoration — it is the precondition of this algebra, and relaxing it
// would silently falsify every composed label.
proptest! {
    /// `label(A ∪ B) = label(A) ⊓ label(B)`
    ///
    /// The semilattice→lattice homomorphism (§4). If this fails, composition
    /// has stopped being associative and every derived answer is suspect.
    #[test]
    fn homomorphism(w in arb_world(FRESHNESS), a in arb_graphset(), b in arb_graphset()) {
        props::homomorphism(&w, &a, &b)?;
    }

    /// Union is idempotent, so the label must be too.
    #[test]
    fn idempotent(w in arb_world(FRESHNESS), a in arb_graphset()) {
        props::idempotent(&w, &a)?;
    }

    /// Union is commutative, so the label must be too.
    #[test]
    fn commutative(w in arb_world(FRESHNESS), a in arb_graphset(), b in arb_graphset()) {
        props::commutative(&w, &a, &b)?;
    }

    /// Union is associative, so the label must be too.
    #[test]
    fn associative(
        w in arb_world(FRESHNESS),
        a in arb_graphset(),
        b in arb_graphset(),
        c in arb_graphset(),
    ) {
        props::associative(&w, &a, &b, &c)?;
    }

    /// The empty dataset is the identity for the fold.
    #[test]
    fn empty_dataset_is_the_identity(w in arb_world(FRESHNESS), a in arb_graphset()) {
        props::empty_is_identity(&w, &a)?;
    }

    /// Composition never widens: the composed freshness is never above either
    /// input's. Stated directly, not via the operator, per §1.
    #[test]
    fn composition_never_widens(
        w in arb_world(FRESHNESS),
        a in arb_graphset(),
        b in arb_graphset(),
    ) {
        props::composition_never_widens(&w, &a, &b)?;
    }

    /// Adding a graph to a dataset can only narrow its label — monotonicity,
    /// the practical restatement of §1 that a reader can check by eye.
    #[test]
    fn adding_a_graph_never_widens(
        w in arb_world(FRESHNESS),
        a in arb_graphset(),
        g in 0u8..props::UNIVERSE,
    ) {
        props::adding_a_graph_never_widens(&w, &a, g)?;
    }
}
