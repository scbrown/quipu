//! The dataset folds over the label lattice — split from [`crate::lattice`]
//! to keep that module under the file-size ratchet.
//!
//! Same rules as the algebra: undeclared members contribute to *coverage*,
//! never to the *value*, and the fold identity is [`Coverage::Empty`].

use crate::error::Result;
use crate::lattice::{Composed, Coverage, Join, Meet};

/// Fold a dataset's declared values on a [`Meet`] axis (freshness, trust,
/// authority).
///
/// Each item is one graph's declared value on this axis — `None` for a graph
/// that declared nothing. The result pairs the meet over the declared values
/// with the [`Coverage`].
///
/// # Errors
/// Propagates the axis's own incomparability error (see
/// [`crate::lattice::Trust::meet`]).
pub fn fold_meet<T, I>(items: I) -> Result<Composed<T>>
where
    T: Meet + Clone,
    I: IntoIterator<Item = Option<T>>,
{
    fold_with(items, Meet::meet)
}

/// Fold a dataset's declared values on a [`Join`] axis (policy, kind).
///
/// # Errors
/// Propagates the axis's own composition error;
/// [`crate::lattice::PolicyClass`] never fails.
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
