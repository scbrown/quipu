//! The label-lattice laws as plain functions over any [`Meet`] axis
//! (aegis-xfuch4.4).
//!
//! `lattice_tests.rs` runs them as proptests on the real `Freshness` axis, and
//! `lattice_evidence_tests.rs` runs the SAME functions against a deliberately
//! widening axis, so the published evidence is about this code and not a copy.
//!
//! Each law returns whether its assertion was EXERCISED. The two direction laws
//! are guarded (`if let (Some, Some, ..)`): a case where a side is undeclared
//! passes without checking anything, and the evidence counts those apart.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;

use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

use super::{Composed, Meet, fold_meet};

/// Graph ids are drawn from `0..UNIVERSE`.
pub(super) const UNIVERSE: u8 = 8;

/// The labelling: graph id -> its declared label. A function, so two datasets
/// share one labelling (see `lattice_tests.rs` for why that is load-bearing).
pub(super) type World<T> = BTreeMap<u8, Option<T>>;

/// A dataset: a set of graph ids.
pub(super) type GraphSet = BTreeSet<u8>;

/// What a law needs from an axis.
pub(super) trait Axis: Meet + Clone + Ord + Debug + 'static {}
impl<T: Meet + Clone + Ord + Debug + 'static> Axis for T {}

/// A labelling of the whole universe: each graph is undeclared or one of
/// `values`, uniformly.
pub(super) fn arb_world<T: Axis>(values: &[T]) -> impl Strategy<Value = World<T>> + use<T> {
    let options: Vec<Option<T>> = std::iter::once(None)
        .chain(values.iter().cloned().map(Some))
        .collect();
    prop::collection::vec(prop::sample::select(options), UNIVERSE as usize).prop_map(|v| {
        v.into_iter()
            .enumerate()
            .map(|(i, m)| (u8::try_from(i).unwrap_or(0), m))
            .collect()
    })
}

/// A dataset over the universe; union is set union.
pub(super) fn arb_graphset() -> impl Strategy<Value = GraphSet> {
    prop::collection::btree_set(0u8..UNIVERSE, 0..UNIVERSE as usize)
}

/// The composed label of a dataset.
pub(super) fn label_of<T: Axis>(world: &World<T>, set: &GraphSet) -> Composed<T> {
    fold_meet(set.iter().map(|g| world.get(g).cloned().flatten())).unwrap()
}

/// `label(A ∪ B) = label(A) ⊓ label(B)`, the semilattice→lattice homomorphism.
pub(super) fn homomorphism<T: Axis>(
    w: &World<T>,
    a: &GraphSet,
    b: &GraphSet,
) -> Result<bool, TestCaseError> {
    let union: GraphSet = a.union(b).copied().collect();
    let lhs = label_of(w, &union);
    let rhs = label_of(w, a).compose_meet(&label_of(w, b)).unwrap();
    prop_assert_eq!(lhs.value, rhs.value, "folded value must agree");
    prop_assert_eq!(lhs.coverage, rhs.coverage, "coverage must agree");
    Ok(true)
}

/// Union is idempotent, so the label must be too.
pub(super) fn idempotent<T: Axis>(w: &World<T>, a: &GraphSet) -> Result<bool, TestCaseError> {
    let once = label_of(w, a);
    let twice = once.compose_meet(&once).unwrap();
    prop_assert_eq!(once, twice);
    Ok(true)
}

/// Union is commutative, so the label must be too.
pub(super) fn commutative<T: Axis>(
    w: &World<T>,
    a: &GraphSet,
    b: &GraphSet,
) -> Result<bool, TestCaseError> {
    let ab = label_of(w, a).compose_meet(&label_of(w, b)).unwrap();
    let ba = label_of(w, b).compose_meet(&label_of(w, a)).unwrap();
    prop_assert_eq!(ab, ba);
    Ok(true)
}

/// Union is associative, so the label must be too.
pub(super) fn associative<T: Axis>(
    w: &World<T>,
    a: &GraphSet,
    b: &GraphSet,
    c: &GraphSet,
) -> Result<bool, TestCaseError> {
    let (la, lb, lc) = (label_of(w, a), label_of(w, b), label_of(w, c));
    let left = la.compose_meet(&lb).unwrap().compose_meet(&lc).unwrap();
    let right = la.compose_meet(&lb.compose_meet(&lc).unwrap()).unwrap();
    prop_assert_eq!(left, right);
    Ok(true)
}

/// The empty dataset is the identity for the fold.
pub(super) fn empty_is_identity<T: Axis>(
    w: &World<T>,
    a: &GraphSet,
) -> Result<bool, TestCaseError> {
    let la = label_of(w, a);
    let empty: Composed<T> = Composed::empty();
    prop_assert_eq!(la.clone().compose_meet(&empty).unwrap(), la.clone());
    prop_assert_eq!(empty.compose_meet(&la).unwrap(), la);
    Ok(true)
}

/// Composition never widens: the composed value is never above either input.
/// Stated directly, not via the operator, because the algebraic laws above are
/// indifferent to the operator's direction (quipu #66).
pub(super) fn composition_never_widens<T: Axis>(
    w: &World<T>,
    a: &GraphSet,
    b: &GraphSet,
) -> Result<bool, TestCaseError> {
    let la = label_of(w, a);
    let lb = label_of(w, b);
    let composed = la.compose_meet(&lb).unwrap();
    if let (Some(va), Some(vb), Some(vc)) = (la.value, lb.value, composed.value) {
        prop_assert!(vc <= va, "composed rose above A");
        prop_assert!(vc <= vb, "composed rose above B");
        return Ok(true);
    }
    Ok(false)
}

/// Adding a graph to a dataset can only narrow its label.
pub(super) fn adding_a_graph_never_widens<T: Axis>(
    w: &World<T>,
    a: &GraphSet,
    g: u8,
) -> Result<bool, TestCaseError> {
    let before = label_of(w, a);
    let mut bigger = a.clone();
    bigger.insert(g);
    let after = label_of(w, &bigger);
    if let (Some(vb), Some(va)) = (before.value, after.value) {
        prop_assert!(va <= vb, "adding a graph raised the label");
        return Ok(true);
    }
    Ok(false)
}
