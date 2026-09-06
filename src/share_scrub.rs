//! The outward boundary: which destination a share is bound for, and the
//! block-tier identifier scrub that a share bound outward must pass.
//!
//! Split from `share.rs` because three producers need it — a full share, a
//! delta, and an import of a share someone else marked — and because the
//! destination and the check it governs are one decision, not two.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::store::Store;

/// Where a share is bound, and therefore whether the outward scrub applies.
///
/// The default is [`ShareDestination::Outward`] and it is load-bearing: every
/// caller that does not name a destination — including every existing one, and
/// every HTTP request — gets the scrub. `Internal` has to be asked for by name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShareDestination {
    /// Anywhere, including a public remote. The outward scrub applies.
    #[default]
    Outward,
    /// A LAN-internal destination only. The outward scrub is skipped, and the
    /// manifest is stamped so the exemption travels with the payload.
    Internal,
}

impl ShareDestination {
    /// Whether this destination skips the outward scrub.
    #[must_use]
    pub fn is_internal(self) -> bool {
        matches!(self, Self::Internal)
    }
}

/// The flag an operator passes to accept an internal destination, named in
/// every refusal so a reader is never left to guess the escape hatch exists.
pub const INTERNAL_FLAG: &str = "--destination internal";

fn outward_scrub_patterns(store: &Store) -> Result<Vec<(String, regex::Regex)>> {
    const QUERY: &str = "PREFIX aegis: <http://aegis.gastown.local/ontology/> \
        PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
        SELECT ?label ?regex WHERE { \
          ?rule a aegis:InternalIdentifierPattern ; \
                rdfs:label ?label ; \
                aegis:regex ?regex ; \
                aegis:enforcementTier \"block\" . \
        } ORDER BY ?label ?regex";
    let crate::sparql::QueryResult::Select { rows, .. } = crate::sparql::query(store, QUERY)?
    else {
        return Err(Error::Store(
            "share scrub: InternalIdentifierPattern query did not return rows".into(),
        ));
    };
    rows.into_iter()
        .map(|row| {
            let label = match row.get("label") {
                Some(crate::types::Value::Str(value)) => value.clone(),
                _ => {
                    return Err(Error::Store(
                        "share scrub: pattern has no string label".into(),
                    ));
                }
            };
            let Some(crate::types::Value::Str(source)) = row.get("regex") else {
                return Err(Error::Store(format!(
                    "share scrub: pattern {label:?} has no string regex"
                )));
            };
            let compiled = regex::Regex::new(source).map_err(|error| {
                Error::InvalidValue(format!(
                    "share scrub: pattern {label:?} has invalid regex: {error}"
                ))
            })?;
            Ok((label, compiled))
        })
        .collect()
}

/// Refuse a payload carrying a block-tier internal identifier.
///
/// `context` names the boundary being crossed so a refusal says which command
/// refused; the flag is named unconditionally, because a guard that reports a
/// dead end sends the operator looking for a silent way round it.
///
/// # Errors
/// The pattern catalogue cannot be read, or a payload file matches a block-tier
/// pattern.
pub(crate) fn scrub_outward_payload(
    store: &Store,
    files: &BTreeMap<String, String>,
    context: &str,
) -> Result<()> {
    for (label, pattern) in outward_scrub_patterns(store)? {
        for (name, contents) in files {
            if name == "manifest.json" {
                continue;
            }
            if let Some(hit) = pattern.find(contents) {
                return Err(Error::PolicyDenied(format!(
                    "{context} refused {name}: InternalIdentifierPattern {label:?} matched bytes {}..{}; \
                     identifiers are entity identity and are never rewritten at this boundary. \
                     Pass {INTERNAL_FLAG} if this share is bound for a LAN-internal destination only.",
                    hit.start(),
                    hit.end()
                )));
            }
        }
    }
    Ok(())
}

/// Apply the outward scrub unless the destination is explicitly internal.
///
/// The ONE place the exemption is expressed. Producers call this rather than
/// branching themselves, so a new producer cannot acquire a silent opt-out by
/// forgetting to check — it has to pass a destination in.
///
/// # Errors
/// The payload is bound outward and fails [`scrub_outward_payload`].
pub(crate) fn enforce_destination(
    store: &Store,
    files: &BTreeMap<String, String>,
    destination: ShareDestination,
    context: &str,
) -> Result<()> {
    if destination.is_internal() {
        return Ok(());
    }
    scrub_outward_payload(store, files, context)
}
