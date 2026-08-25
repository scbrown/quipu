//! Declared labels at the federation edge (quipu-fd1).
//!
//! Design: `docs/design/multi-db-composition.md` §5 — the seam composition and
//! federation share is labels: `ProviderStatus` carries the label fields, and a
//! remote carries a **declared** trust label rather than an inferred one. This
//! is the SARC trust boundary surfaced at the federation edge.
//!
//! ## The one rule: the label is declared by the LOCAL operator
//!
//! A remote's label comes from `[[quipu.federation.remotes]]` in the *local*
//! config, never from the remote itself. A remote asserting its own
//! trustworthiness defeats the boundary — the same reason a tenant with
//! authority over its own graph must not be able to relabel itself `attested`
//! (`src/store/labels.rs`, the authority consequence).
//!
//! ## Undeclared semantics, stated once
//!
//! An undeclared remote composes exactly like an unlabelled local graph
//! (graph-labels.md §2.1): it reads back as *undeclared* — never a fabricated
//! value — and a configured `[quipu.labels]` freshness or trust floor refuses
//! it rather than reading silence as permission. With no floor configured,
//! nothing changes: undeclared remotes federate exactly as before.

use serde_json::Value as JsonValue;

use crate::config::{FederationConfig, LabelsConfig, RemoteEndpoint};
use crate::error::{Error, Result};
use crate::lattice::{Composed, Coverage, Freshness, Trust};
use crate::sparql::TemporalContext;
use crate::store::Store;
use crate::store::labels::DatasetLabels;

/// The label a federation member's rows carry — declared by the local
/// operator, never read from the member itself.
///
/// Only the axes an operator can honestly declare about a peer: trust and
/// freshness. Durability, policy and kind stay undeclared for a remote — a
/// remote member therefore degrades a dataset's coverage on those axes to
/// `partial`, which is the conservative reading, not an omission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeclaredLabel {
    /// Declared trust, with the chain that ranks it.
    pub trust: Option<Trust>,
    /// Declared freshness.
    pub freshness: Option<Freshness>,
}

impl DeclaredLabel {
    /// Whether nothing is declared on any axis.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trust.is_none() && self.freshness.is_none()
    }

    /// The JSON shape reported beside provider status/outcomes.
    #[must_use]
    pub fn to_json(&self) -> JsonValue {
        serde_json::json!({
            "trust": self.trust.as_ref().map(|t| serde_json::json!({
                "iri": t.iri, "chain": t.chain, "rank": t.rank,
            })),
            "freshness": self.freshness.map(Freshness::as_str),
        })
    }
}

impl std::fmt::Display for DeclaredLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.trust, self.freshness) {
            (None, None) => f.write_str("undeclared"),
            (Some(t), None) => write!(f, "trust {t}"),
            (None, Some(fr)) => write!(f, "freshness {fr}"),
            (Some(t), Some(fr)) => write!(f, "trust {t}, freshness {fr}"),
        }
    }
}

impl RemoteEndpoint {
    /// The label the local operator declared for this remote.
    ///
    /// # Errors
    /// A **partial** trust declaration is refused rather than silently dropped:
    /// `trust`, `trust_chain` and `trust_rank` are one declaration — a rank
    /// means nothing outside its chain, and an IRI without a rank cannot meet a
    /// floor. An unparseable freshness string is likewise refused, never
    /// silently undeclared — a typo'd declaration that vanishes would leave the
    /// operator believing a label flows when none does.
    pub fn declared_label(&self) -> Result<DeclaredLabel> {
        let trust = match (&self.trust, &self.trust_chain, self.trust_rank) {
            (None, None, None) => None,
            (Some(iri), Some(chain), Some(rank)) => Some(Trust::new(iri, chain, rank)),
            _ => {
                return Err(Error::InvalidValue(format!(
                    "remote '{}': a declared trust label needs all three of `trust`, \
                     `trust_chain` and `trust_rank` in [[quipu.federation.remotes]]. \
                     A rank means nothing outside the chain that declared it, and an \
                     IRI without a rank cannot be compared to a floor — declare all \
                     three, or none.",
                    self.name
                )));
            }
        };
        let freshness = match self.freshness.as_deref() {
            None => None,
            Some(s) => Some(Freshness::parse(s).ok_or_else(|| {
                Error::InvalidValue(format!(
                    "remote '{}': declared freshness '{s}' is not a freshness value \
                     (fresh|recomputing|stale)",
                    self.name
                ))
            })?),
        };
        Ok(DeclaredLabel { trust, freshness })
    }
}

/// Check one federation member's declared label against the configured floors —
/// the per-member half of `Store::check_label_floor`, applied at the federation
/// edge so the refusal can **name the remote** that failed.
///
/// Mirrors the local semantics axis by axis: a declared value below the floor
/// fails; **undeclared fails a configured freshness or trust floor** (fail-safe
/// at enforcement, honest at reporting — graph-labels.md §2.1); the policy and
/// kind blocklists pass an undeclared member, exactly as they pass an unlabelled
/// local graph. Unset floors are a no-op.
///
/// # Errors
/// [`Error::PolicyDenied`] naming the remote and its declared label, or an
/// [`Error::InvalidValue`] when the floor itself is malformed (same refusals as
/// `check_label_floor`).
pub fn check_member_floor(floor: &LabelsConfig, remote: &str, label: &DeclaredLabel) -> Result<()> {
    if floor.is_unset() {
        return Ok(());
    }

    if let Some(s) = floor.min_freshness.as_deref() {
        let min = Freshness::parse(s).ok_or_else(|| {
            Error::InvalidValue(format!(
                "[quipu.labels] min_freshness = '{s}' is not a freshness value \
                 (fresh|recomputing|stale)"
            ))
        })?;
        match label.freshness {
            Some(f) if f >= min => {}
            Some(f) => {
                return Err(Error::PolicyDenied(format!(
                    "federated query refused: remote '{remote}' is declared '{f}', below \
                     the configured freshness floor '{min}'."
                )));
            }
            None => {
                return Err(Error::PolicyDenied(format!(
                    "federated query refused: remote '{remote}' declares no freshness, and \
                     a freshness floor of '{min}' is configured. A remote's label is \
                     declared by the local operator in [[quipu.federation.remotes]] — \
                     undeclared fails a floor rather than reading silence as '{min}'."
                )));
            }
        }
    }

    if floor.min_trust_rank.is_some() && floor.min_trust_chain.is_none() {
        return Err(Error::InvalidValue(
            "[quipu.labels] min_trust_rank is set without min_trust_chain. A rank \
             means nothing outside the chain that declared it — say which chain the \
             floor is expressed in, or ranks from an unrelated vocabulary would be \
             compared as bare integers."
                .into(),
        ));
    }
    if let (Some(min_rank), Some(min_chain)) =
        (floor.min_trust_rank, floor.min_trust_chain.as_deref())
    {
        match &label.trust {
            Some(t) if t.chain != min_chain => {
                return Err(Error::PolicyDenied(format!(
                    "federated query refused: remote '{remote}' is declared trust '{}' \
                     ranked in chain '{}', but the configured floor is expressed in \
                     chain '{min_chain}'. Ranks are not comparable across chains, so \
                     this cannot be evaluated rather than being evaluated wrongly.",
                    t.iri, t.chain
                )));
            }
            Some(t) if t.rank < min_rank => {
                return Err(Error::PolicyDenied(format!(
                    "federated query refused: remote '{remote}' is declared trust '{}' \
                     at rank {} in chain '{min_chain}', below the configured floor of \
                     {min_rank}.",
                    t.iri, t.rank
                )));
            }
            Some(_) => {}
            None => {
                return Err(Error::PolicyDenied(format!(
                    "federated query refused: remote '{remote}' declares no trust, and a \
                     trust floor of {min_rank} in chain '{min_chain}' is configured. A \
                     remote's label is declared by the local operator in \
                     [[quipu.federation.remotes]] — undeclared fails a floor."
                )));
            }
        }
    }

    // deny_policy_tokens / deny_data_kinds: a remote declares neither axis, and
    // both are blocklists that an undeclared member passes — the same treatment
    // an unlabelled local graph gets from `check_label_floor`.
    Ok(())
}

/// Enforce the configured `[quipu.labels]` floors over a whole federated read:
/// the local members (the same dataset fold `tool_query` applies) **and** every
/// configured remote's declared label (quipu-fd1).
///
/// This is what closes the widening: before it, `store.check_label_floor` ran
/// on the local `/query` path only, so a deployment with a `min_trust` floor
/// silently widened its trust the moment `federated: true` was set.
///
/// A no-op when no floor is configured — the fast path reads no labels at all.
///
/// # Errors
/// [`Error::PolicyDenied`] naming the local graph or the remote that failed;
/// parse errors from the query text; a refusal for a malformed declaration.
pub fn check_federated_floor(
    store: &Store,
    sparql: &str,
    federation: &FederationConfig,
) -> Result<()> {
    if store.labels_config().is_unset() {
        return Ok(());
    }
    // The federated path refuses the temporal/graph parameters, so the local
    // member set is the query's own dataset clause over the default context —
    // the same resolution evaluation uses.
    let member_ids = crate::sparql::dataset_member_ids(store, sparql, &TemporalContext::default())?;
    store.check_label_floor(&member_ids)?;
    for remote in &federation.remotes {
        check_member_floor(
            store.labels_config(),
            &remote.name,
            &remote.declared_label()?,
        )?;
    }
    Ok(())
}

/// The composed label of a federated dataset: the local members' fold with
/// every remote's declared label folded in **as a member** — trust and
/// freshness by meet, so composition never widens, and the axes a remote
/// cannot declare (durability, policy, kind) folded as undeclared, degrading
/// coverage to `partial` rather than pretending the remote said anything.
///
/// Returns `None` when nothing local or remote declared anything — reported as
/// `"labels": null`, exactly like the local path.
///
/// # Errors
/// A cross-chain trust meet between a local graph and a remote (or two
/// remotes) refuses, naming both chains — the same refusal as the local fold.
pub fn federated_dataset_labels(
    store: &Store,
    sparql: &str,
    federation: &FederationConfig,
) -> Result<Option<DatasetLabels>> {
    let member_ids = crate::sparql::dataset_member_ids(store, sparql, &TemporalContext::default())?;
    let mut labels = store.dataset_labels(&member_ids)?;
    for remote in &federation.remotes {
        labels = fold_member(&labels, &remote.declared_label()?)?;
    }
    Ok(if labels.is_undeclared() {
        None
    } else {
        Some(labels)
    })
}

/// Fold one member's declared label into a composed dataset label.
fn fold_member(labels: &DatasetLabels, member: &DeclaredLabel) -> Result<DatasetLabels> {
    fn declared<T: Clone>(v: Option<&T>) -> Composed<T> {
        Composed {
            value: v.cloned(),
            coverage: if v.is_some() {
                Coverage::Full
            } else {
                Coverage::None
            },
        }
    }
    fn undeclared<T>() -> Composed<T> {
        Composed {
            value: None,
            coverage: Coverage::None,
        }
    }
    Ok(DatasetLabels {
        freshness: labels
            .freshness
            .compose_meet(&declared(member.freshness.as_ref()))?,
        durability: labels.durability.compose_meet(&undeclared())?,
        trust: labels
            .trust
            .compose_meet(&declared(member.trust.as_ref()))?,
        policy: labels.policy.compose_join(&undeclared())?,
        kind: labels.kind.compose_join(&undeclared())?,
    })
}

#[cfg(test)]
#[path = "label_tests.rs"]
mod tests;
