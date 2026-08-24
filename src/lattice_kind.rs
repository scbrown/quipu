//! The `dataKind` axis — what sort of data a graph holds.
//!
//! Design: `docs/design/graph-kinds-and-deep-freeze.md` §2. A categorical
//! axis, deliberately **not ordered**: there is no "weaker" of `knowledge`
//! vs `operational`, and inventing an order would be exactly the sign error
//! [`crate::lattice`]'s module docs warn about. A single graph declares one
//! kind; a *dataset* composes to the **union** of its members' kinds
//! ([`KindSet`], a [`Join`] like [`crate::lattice::PolicyClass`]) — a dataset
//! touching an archive graph *is* partly archive, and that information must
//! accumulate, never average away.
//!
//! ## Open-but-conventioned values
//!
//! The value space is lexical (`[a-z][a-z0-9-]*`), not an enum: kind is
//! descriptive vocabulary, and a fifth kind must not require a quipu release.
//! The strict *parse* elsewhere in the lattice protects ordered comparisons;
//! kind has none. The conventioned set this stack uses:
//!
//! - [`KIND_KNOWLEDGE`] — durable semantic content;
//! - [`KIND_OPERATIONAL`] — high-churn workflow/run/ticket state, the
//!   freeze candidate;
//! - [`KIND_IDENTITY`] — principals and verifier registrations, split out so
//!   freezing an operational window never strands the keys that verify its
//!   signatures;
//! - [`KIND_ARCHIVE`] — frozen, read-only history; set by the freeze
//!   operation.

use std::collections::BTreeSet;
use std::fmt;

use crate::error::{Error, Result};
use crate::lattice::Join;

/// Conventioned kind: durable semantic content.
pub const KIND_KNOWLEDGE: &str = "knowledge";
/// Conventioned kind: high-churn workflow/run/ticket state.
pub const KIND_OPERATIONAL: &str = "operational";
/// Conventioned kind: principals, verifier registrations, public keys.
pub const KIND_IDENTITY: &str = "identity";
/// Conventioned kind: frozen, read-only history.
pub const KIND_ARCHIVE: &str = "archive";

/// A declared data kind — one well-formed token.
///
/// Any token matching the lexical rule is a value; an unrecognised *shape*
/// (uppercase, whitespace, leading digit) is an error, never silently
/// normalised. Declared, like every label axis: quipu never synthesizes one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataKind(String);

impl DataKind {
    /// Parse a declared kind token.
    ///
    /// # Errors
    /// [`Error::InvalidValue`] when the token does not match
    /// `[a-z][a-z0-9-]*` — refused rather than normalised, so `"Archive"`
    /// and `"archive"` can never mint two kinds for one concept.
    pub fn parse(s: &str) -> Result<Self> {
        let mut chars = s.chars();
        let ok = matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
            && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(Error::InvalidValue(format!(
                "data kind '{s}' is not a valid kind token; kinds match \
                 [a-z][a-z0-9-]* (conventioned values: {KIND_KNOWLEDGE}, \
                 {KIND_OPERATIONAL}, {KIND_IDENTITY}, {KIND_ARCHIVE})"
            )));
        }
        Ok(Self(s.to_string()))
    }

    /// The declared token.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DataKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The kinds present across a composed dataset.
///
/// Composes by **union**: like obligations, kind information accumulates so
/// that a caller reading `{knowledge, archive}` knows cold data contributed
/// to the answer. A single graph's `KindSet` is a singleton.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KindSet {
    kinds: BTreeSet<String>,
}

impl KindSet {
    /// A set holding one declared kind.
    #[must_use]
    pub fn singleton(kind: &DataKind) -> Self {
        let mut kinds = BTreeSet::new();
        kinds.insert(kind.as_str().to_string());
        Self { kinds }
    }

    /// Whether no kind is present. The identity for [`Join`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// Whether `kind` is among the composed kinds.
    #[must_use]
    pub fn contains(&self, kind: &str) -> bool {
        self.kinds.contains(kind)
    }

    /// The kinds, sorted, so a report is stable.
    #[must_use]
    pub fn kinds(&self) -> Vec<&str> {
        self.kinds.iter().map(String::as_str).collect()
    }
}

impl fmt::Display for KindSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kinds.is_empty() {
            f.write_str("(no kinds)")
        } else {
            f.write_str(&self.kinds().join(", "))
        }
    }
}

impl Join for KindSet {
    /// The **union** — a dataset holds every kind any member holds.
    fn join(&self, other: &Self) -> Result<Self> {
        Ok(Self {
            kinds: self.kinds.union(&other.kinds).cloned().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_conventioned_and_novel_tokens() {
        for k in [
            KIND_KNOWLEDGE,
            KIND_OPERATIONAL,
            KIND_IDENTITY,
            KIND_ARCHIVE,
            "reference-v2",
        ] {
            assert_eq!(DataKind::parse(k).unwrap().as_str(), k);
        }
    }

    #[test]
    fn parse_refuses_bad_shapes_rather_than_normalising() {
        for bad in ["Archive", "two words", "-lead", "9lead", "", "under_score"] {
            assert!(DataKind::parse(bad).is_err(), "'{bad}' must be refused");
        }
    }

    #[test]
    fn kindset_joins_by_union_and_reports_sorted() {
        let a = KindSet::singleton(&DataKind::parse("operational").unwrap());
        let b = KindSet::singleton(&DataKind::parse("archive").unwrap());
        let joined = a.join(&b).unwrap();
        assert_eq!(joined.kinds(), vec!["archive", "operational"]);
        assert!(joined.contains("archive"));
        // Idempotent: joining a set with itself changes nothing.
        assert_eq!(joined.join(&joined).unwrap(), joined);
        // Identity: the empty set.
        assert_eq!(KindSet::default().join(&joined).unwrap(), joined);
    }
}
