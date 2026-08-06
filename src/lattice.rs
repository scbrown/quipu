//! The label lattice — freshness, trust and policy composing under one rule.
//!
//! Design: [`docs/design/graph-labels.md`] §1, §2, §4. This module is the
//! algebra only: **pure Rust, no I/O, no store access.** The store side
//! (`set_graph_label`/`label_of`, the `graphs` cache columns, the meta-graph)
//! and the query side (`query_labeled`) build on it.
//!
//! ## The one invariant: composition never widens
//!
//! Three modules in this codebase independently arrived at the same rule
//! before it had a name:
//!
//! - [`crate::governance::authority::Authority::intersect`] — delegation only
//!   narrows; an empty intersection is a refusal, never a fallback.
//! - SHACL branch composition — a branch may only *add* constraints.
//! - Hank's confidence rule — a policy is only as confident as its
//!   least-confident leaf.
//!
//! The invariant is deliberately **not** "everything is a meet", because the
//! operator flips direction by axis:
//!
//! - **freshness** and **trust** compose by [`Meet`] — a union of graphs is
//!   only as fresh and as trusted as its weakest member;
//! - **obligations** compose by [`Join`] — a union of graphs carries the
//!   *union* of their restrictions, so a `no-export` graph taints the set.
//!
//! Naming the invariant rather than the operator is what prevents the sign
//! error: someone who remembers "labels are a meet" will union obligations in
//! the widening direction and quietly drop a restriction.
//!
//! ## Why `meet`/`join` are fallible
//!
//! Trust ranks are only comparable **within a declared chain** (§2). Comparing
//! `smac:canonical` against Hank's `attested` is not a small number vs a large
//! one — it is a category error, and silently comparing the ints is exactly how
//! a "learned tactic outranks canonical" bug ships. So the trait methods return
//! [`Result`] and [`Trust`] returns an error naming *both* chains.
//!
//! [`Freshness`], [`PolicyClass`] and [`Authority`] cannot fail; their impls
//! are always `Ok`. One fallible trait keeps [`fold_meet`]/[`fold_join`]
//! uniform, which matters more than making three infallible impls prove it in
//! the type.
//!
//! ## Undeclared is not a lattice value
//!
//! Every graph in every existing deployment is unlabelled, so the default
//! decides backward compatibility, and **neither pole is right** (§2.1):
//! defaulting to ⊤ fail-opens trust (an unlabelled graph would read
//! `attested`); defaulting to ⊥ drags every existing query to the floor.
//!
//! So a composed label is a *pair* — the fold over the **declared** labels,
//! plus a [`Coverage`] saying how much of the dataset declared anything. See
//! [`Composed`].

use std::collections::BTreeSet;
use std::fmt;

use crate::error::{Error, Result};
use crate::governance::authority::Authority;

/// Composition in the narrowing direction: the result permits/claims no more
/// than either input.
///
/// Fallible — see the module docs. Implementors must be **associative,
/// commutative and idempotent**; [`fold_meet`] relies on all three, and the
/// homomorphism proptest is what holds us to it.
pub trait Meet: Sized {
    /// Compose with `other`, narrowing.
    ///
    /// # Errors
    /// When the two values are incomparable — see [`Trust::meet`].
    fn meet(&self, other: &Self) -> Result<Self>;
}

/// Composition in the accumulating direction, for obligations.
///
/// This is still "never widens": a larger obligation set is a *stronger*
/// restriction. The set grows so that permission does not.
pub trait Join: Sized {
    /// Compose with `other`, accumulating obligations.
    ///
    /// # Errors
    /// Never, for the types in this module; the signature matches [`Meet`] so
    /// the two folds stay symmetric.
    fn join(&self, other: &Self) -> Result<Self>;
}

// ---------------------------------------------------------------------------
// Axis 1: freshness
// ---------------------------------------------------------------------------

/// How current a graph's contents are — Hank's exact vocabulary (§2).
///
/// A total order, composed by meet (`min`). Quipu **never observes** staleness;
/// a producer declares it. There is no synthesized `fresh` tag, ever.
///
/// The discriminant order is the semantic order: `Stale < Recomputing < Fresh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Freshness {
    /// Known out of date.
    Stale,
    /// A recompute is in flight; not yet trustworthy as fresh.
    Recomputing,
    /// Current.
    Fresh,
}

impl Freshness {
    /// Parse the declared string form, or `None` if it is not a known value.
    ///
    /// Deliberately not `FromStr`-with-a-default: an unrecognised freshness
    /// string must not silently become `fresh`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fresh" => Some(Self::Fresh),
            "recomputing" => Some(Self::Recomputing),
            "stale" => Some(Self::Stale),
            _ => None,
        }
    }

    /// The declared string form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Recomputing => "recomputing",
            Self::Stale => "stale",
        }
    }

    /// Collapse onto a scale that only admits fresh/stale (§2).
    ///
    /// `recomputing` collapses to **stale**, never to fresh: the conservative
    /// reading is the only one that cannot overstate, which is the direction
    /// every mapping in this stack already chooses.
    #[must_use]
    pub fn collapse_binary(self) -> Self {
        match self {
            Self::Fresh => Self::Fresh,
            Self::Recomputing | Self::Stale => Self::Stale,
        }
    }
}

impl fmt::Display for Freshness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Meet for Freshness {
    fn meet(&self, other: &Self) -> Result<Self> {
        Ok(*self.min(other))
    }
}

// ---------------------------------------------------------------------------
// Axis 2: trust
// ---------------------------------------------------------------------------

/// The default trust chain Quipu ships. Consumers declare their own; Quipu
/// learns neither Hank's nor `NeuralAmplifier`'s vocabulary (§2).
pub const DEFAULT_TRUST_CHAIN: &str = "http://quipu.dev/ontology/defaultTrustChain";

/// A trust value: an IRI, its rank, and **the chain that ranked it**.
///
/// Trust is not a hardcoded enum. The values are IRIs and the *ordering is
/// data* — `smac:canonical quipu:trustRank 40 ; quipu:inChain smac:ruleTierChain`.
/// Carrying `chain` on the value is what makes a cross-chain comparison
/// detectable at all: without it, two ranks are just two integers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trust {
    /// The trust value's own IRI (e.g. `smac:canonical`).
    pub iri: String,
    /// The IRI of the chain that declares this value's rank.
    pub chain: String,
    /// Rank within `chain`. Higher is more trusted.
    pub rank: i64,
}

impl Trust {
    /// A trust value in a named chain.
    pub fn new(iri: impl Into<String>, chain: impl Into<String>, rank: i64) -> Self {
        Self {
            iri: iri.into(),
            chain: chain.into(),
            rank,
        }
    }
}

impl fmt::Display for Trust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (rank {} in {})", self.iri, self.rank, self.chain)
    }
}

impl Meet for Trust {
    /// The less-trusted of the two — **within one chain**.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] naming **both** chains when they differ. This is
    /// the refusal §2 requires: ranks from different chains are incomparable,
    /// and comparing them as integers is a category error, not a close call.
    fn meet(&self, other: &Self) -> Result<Self> {
        if self.chain != other.chain {
            return Err(Error::InvalidValue(format!(
                "cross-chain trust comparison refused: '{}' is ranked in chain \
                 '{}' but '{}' is ranked in chain '{}'. Ranks are only \
                 comparable within a declared chain — relate the chains \
                 explicitly, or compose these graphs in separate datasets.",
                self.iri, self.chain, other.iri, other.chain
            )));
        }
        Ok(if self.rank <= other.rank {
            self.clone()
        } else {
            other.clone()
        })
    }
}

// ---------------------------------------------------------------------------
// Axis 3: policy
// ---------------------------------------------------------------------------

/// A set of obligation tokens (`pii`, `no-export`, …) — the deliberately
/// *partial* axis (§2, Denning's information-flow lattice).
///
/// Incomparable sets are the normal case, and that blocks nothing: the join
/// always exists on a powerset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyClass {
    tokens: BTreeSet<String>,
}

impl PolicyClass {
    /// A policy class over these obligation tokens.
    pub fn new<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            tokens: tokens.into_iter().map(Into::into).collect(),
        }
    }

    /// No obligations. The identity for [`Join`].
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether this class carries no obligations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// Whether `token` is carried.
    #[must_use]
    pub fn contains(&self, token: &str) -> bool {
        self.tokens.contains(token)
    }

    /// The tokens, sorted, so a refusal message is stable.
    #[must_use]
    pub fn tokens(&self) -> Vec<&str> {
        self.tokens.iter().map(String::as_str).collect()
    }
}

impl fmt::Display for PolicyClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.tokens.is_empty() {
            f.write_str("(no obligations)")
        } else {
            f.write_str(&self.tokens().join(", "))
        }
    }
}

impl Join for PolicyClass {
    /// The **union** — a dataset carries every obligation any member carries.
    fn join(&self, other: &Self) -> Result<Self> {
        Ok(Self {
            tokens: self.tokens.union(&other.tokens).cloned().collect(),
        })
    }
}

// ---------------------------------------------------------------------------
// Authority — the existing meet, made structural
// ---------------------------------------------------------------------------

impl Meet for Authority {
    /// Delegation narrows. Behaviour is **exactly**
    /// [`Authority::intersect`] — this impl routes to it rather than
    /// reimplementing, so every existing authority test remains the guard.
    fn meet(&self, other: &Self) -> Result<Self> {
        Ok(self.intersect(other))
    }
}

// ---------------------------------------------------------------------------
// Coverage and the composed pair
// ---------------------------------------------------------------------------

/// How much of a dataset actually declared a label on some axis (§2.1).
///
/// This is a property of a **dataset**, not of a graph.
///
/// [`Coverage::Empty`] is the fold identity — a dataset with *no graphs at
/// all*. It is deliberately distinct from [`Coverage::None`] ("graphs are
/// present and none of them declared anything"), and the distinction is
/// load-bearing in two directions:
///
/// - Without it the homomorphism `label(A ∪ B) = label(A) ⊓ label(B)` is
///   **false** for `A = ∅`, because `None ⊓ Full` is `Partial` while
///   `label(∅ ∪ B)` is `label(B)`. Folding then stops being associative, which
///   is the exact failure [`fold_meet`] exists to rule out.
/// - Collapsing them the other way — treating "no graphs" as ⊤ — is the
///   fail-open §2.1 refuses.
///
/// Note this is a fourth value beyond the design's `{full, partial, none}`; it
/// is the algebraic identity, and no graph can declare it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Coverage {
    /// No graphs in the dataset. The fold identity.
    Empty,
    /// Graphs present; none declared a value on this axis.
    None,
    /// Some graphs declared, some did not.
    Partial,
    /// Every graph declared.
    Full,
}

impl Coverage {
    /// Compose two coverages. Associative, commutative, idempotent, with
    /// [`Coverage::Empty`] as the identity.
    ///
    /// Any mix of declared and undeclared is [`Coverage::Partial`], which is
    /// therefore absorbing.
    #[must_use]
    pub fn compose(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, x) | (x, Self::Empty) => x,
            (Self::Full, Self::Full) => Self::Full,
            (Self::None, Self::None) => Self::None,
            _ => Self::Partial,
        }
    }

    /// Whether the coverage is complete. A configured floor treats everything
    /// else as a failure — fail-safe at enforcement, honest at reporting
    /// (§2.1, and quipu #68).
    #[must_use]
    pub fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }

    /// The reported string form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::None => "none",
            Self::Partial => "partial",
            Self::Full => "full",
        }
    }
}

impl fmt::Display for Coverage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A composed label on one axis: the fold over the **declared** values, plus
/// how much of the dataset declared one.
///
/// `value` is [`Option::None`] exactly when nothing was declared — reported as
/// *undeclared*, never as a fabricated `fresh` / ⊤ / ⊥.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Composed<T> {
    /// The folded value over declared members, or `None` if none declared.
    pub value: Option<T>,
    /// How much of the dataset declared a value on this axis.
    pub coverage: Coverage,
}

impl<T> Composed<T> {
    /// The identity: an empty dataset.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            value: None,
            coverage: Coverage::Empty,
        }
    }

    /// Whether anything was declared.
    #[must_use]
    pub fn is_undeclared(&self) -> bool {
        self.value.is_none()
    }
}

impl<T> Default for Composed<T> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<T: Clone> Composed<T> {
    /// Compose two already-composed labels with `op`.
    ///
    /// This is the right-hand side of the homomorphism — `label(A) ⊓ label(B)`
    /// — and it is deliberately the *same* rule [`fold_with`] applies to raw
    /// members: an undeclared side contributes to coverage and never to the
    /// value. If these two ever disagree, folding stops being associative and
    /// every derived label is suspect, which is what the proptest pins.
    fn compose_with<F>(&self, other: &Self, op: F) -> Result<Self>
    where
        F: Fn(&T, &T) -> Result<T>,
    {
        let value = match (&self.value, &other.value) {
            (None, None) => None,
            (Some(v), None) | (None, Some(v)) => Some(v.clone()),
            (Some(a), Some(b)) => Some(op(a, b)?),
        };
        Ok(Self {
            value,
            coverage: self.coverage.compose(other.coverage),
        })
    }
}

impl<T: Meet + Clone> Composed<T> {
    /// Compose in the narrowing direction.
    ///
    /// # Errors
    /// Propagates the axis's incomparability error (see [`Trust::meet`]).
    pub fn compose_meet(&self, other: &Self) -> Result<Self> {
        self.compose_with(other, Meet::meet)
    }
}

impl<T: Join + Clone> Composed<T> {
    /// Compose in the obligation-accumulating direction.
    ///
    /// # Errors
    /// Propagates the axis's composition error; [`PolicyClass`] never fails.
    pub fn compose_join(&self, other: &Self) -> Result<Self> {
        self.compose_with(other, Join::join)
    }
}

/// Fold a dataset's declared values on a [`Meet`] axis (freshness, trust,
/// authority).
///
/// Each item is one graph's declared value on this axis — `None` for a graph
/// that declared nothing. The result pairs the meet over the declared values
/// with the [`Coverage`].
///
/// # Errors
/// Propagates the axis's own incomparability error (see [`Trust::meet`]).
pub fn fold_meet<T, I>(items: I) -> Result<Composed<T>>
where
    T: Meet + Clone,
    I: IntoIterator<Item = Option<T>>,
{
    fold_with(items, Meet::meet)
}

/// Fold a dataset's declared values on a [`Join`] axis (policy).
///
/// # Errors
/// Propagates the axis's own composition error; [`PolicyClass`] never fails.
pub fn fold_join<T, I>(items: I) -> Result<Composed<T>>
where
    T: Join + Clone,
    I: IntoIterator<Item = Option<T>>,
{
    fold_with(items, Join::join)
}

/// The shared fold. Both directions differ only in the binary operator — the
/// coverage bookkeeping, and the rule that undeclared members contribute to
/// *coverage* but never to the *value*, are identical and live here once.
fn fold_with<T, I, F>(items: I, op: F) -> Result<Composed<T>>
where
    T: Clone,
    I: IntoIterator<Item = Option<T>>,
    F: Fn(&T, &T) -> Result<T>,
{
    let mut value: Option<T> = None;
    let mut coverage = Coverage::Empty;

    for item in items {
        let this = match &item {
            Some(_) => Coverage::Full,
            None => Coverage::None,
        };
        coverage = coverage.compose(this);

        if let Some(v) = item {
            value = match value {
                None => Some(v),
                Some(acc) => Some(op(&acc, &v)?),
            };
        }
    }

    Ok(Composed { value, coverage })
}

#[cfg(test)]
#[path = "lattice_tests.rs"]
mod tests;
