//! Which store tables a reconstruction share carries, and which it must not.
//!
//! The contract is `docs/design/standard-share-artifact.md`, section
//! "Reconstruction completeness" (aegis-9f899e): a share is lossless with
//! respect to a set it DECLARES, rather than lossless full stop.
//!
//! The declaration lives here, in code, for the reason the doc gives — for the
//! [`Disposition::Excluded`] group it is a security boundary, not a convenience,
//! and a boundary that exists only in prose is one a later contributor
//! "completes".

/// What a reconstruction does with one store table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Serialized into the share and restored verbatim.
    Content,
    /// Not serialized; rebuilt on unpack from a pinned, declared recipe.
    Regenerated,
    /// The change feed. Serialized; reader POSITION is not.
    Log,
    /// Never carried. See [`DECLARED`] for the reason on each one.
    Excluded,
}

/// Every table a live store may hold, and what a reconstruction does with it.
///
/// Kept in one list rather than split per disposition so that adding a table
/// forces a choice to be made in one place, in front of the reasons.
pub const DECLARED: &[(&str, Disposition)] = &[
    // -- content ------------------------------------------------------------
    // `facts` is the WHOLE row — g, tx, valid_from, valid_to, op, retracted_tx —
    // not the (e,a,v) projection a current-facts export emits. That projection
    // is the entire gap 9f899e exists to close.
    ("facts", Disposition::Content),
    ("transactions", Disposition::Content),
    ("terms", Disposition::Content),
    ("graphs", Disposition::Content),
    ("shapes", Disposition::Content),
    ("ontologies", Disposition::Content),
    ("queries", Disposition::Content),
    ("query_params", Disposition::Content),
    ("datasets", Disposition::Content),
    ("dataset_members", Disposition::Content),
    ("forks", Disposition::Content),
    ("proposals", Disposition::Content),
    ("term_spaces", Disposition::Content),
    ("schema_terms", Disposition::Content),
    ("store_identity", Disposition::Content),
    // -- regenerated --------------------------------------------------------
    // ~2.2 GB of floats at homelab scale, which rules out text. The pinned
    // embedding model and config are part of the declared set precisely because
    // regeneration is only reconstruction if the recipe travels.
    ("vectors", Disposition::Regenerated),
    // -- log ----------------------------------------------------------------
    ("events", Disposition::Log),
    // -- excluded -----------------------------------------------------------
    // A TRUST REGISTRY. Carrying it would undo aegis-tadzdf by the back door:
    // `share_attestation.rs` refuses to let a bundle register its own producer
    // key at the front door ("quipu never self-registers"), and a share that
    // restored this table would grant exactly that, arriving labelled as
    // completeness rather than as an attestation.
    ("attestation_bindings", Disposition::Excluded),
    // REPLAY STATE, and wrong in BOTH directions: carry spent nonces and a
    // legitimate re-import is refused as a replay; omit them silently and a
    // replay the origin had already spent is accepted on the copy. Excluding it
    // visibly is the only honest option.
    ("attestation_nonces", Disposition::Excluded),
    // A READER's cursor. Restoring it resumes someone else's position.
    ("consumers", Disposition::Excluded),
    // Webhook URLs. Restoring them aims a new store at another store's endpoints.
    ("subscriptions", Disposition::Excluded),
    // `path` is a producer-local filesystem path; restored verbatim it points at
    // files that do not exist on the consumer.
    ("frozen_packs", Disposition::Excluded),
    // In-flight multipart transfer scaffolding, meaningless off its own host.
    ("snapshot_uploads", Disposition::Excluded),
    ("snapshot_upload_parts", Disposition::Excluded),
];

/// What a reconstruction does with `table`, or `None` if it is undeclared.
///
/// `None` is the finding the audit test exists to surface: a table nobody has
/// classified is one a reconstruction silently drops.
#[must_use]
pub fn disposition(table: &str) -> Option<Disposition> {
    DECLARED
        .iter()
        .find(|(name, _)| *name == table)
        .map(|(_, d)| *d)
}

/// Table names carried into the share, in declaration order.
#[must_use]
pub fn carried() -> Vec<&'static str> {
    DECLARED
        .iter()
        .filter(|(_, d)| matches!(d, Disposition::Content | Disposition::Log))
        .map(|(name, _)| *name)
        .collect()
}

#[cfg(test)]
#[path = "share_completeness_tests.rs"]
mod tests;
