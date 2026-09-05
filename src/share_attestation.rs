//! The share-import attestation seam (aegis-c9c44).
//!
//! `share_import::verify_share` proves the payload HASHES match the manifest — the
//! bytes are intact. It says nothing about WHO produced them, and before this module
//! those two claims were the same green to anyone reading an import result.
//!
//! The verifier itself lives in [`crate::session_attestation`] and is shared with the
//! HTTP write path. It had been complete, tested against tamper, substitution, replay
//! and domain downgrade, and called by NOTHING: a capability nothing invokes is not a
//! capability. This module is the one seam that was missing.
//!
//! Lifted out of `share_import.rs` rather than left inline because that file was at
//! the 500-line ratchet and because the seam is a subject of its own — the same
//! extract-do-not-grandfather call malcolm made on `cli.rs` and I made on yupana's
//! hook files today.

use crate::error::{Error, Result};
use crate::share_import::ShareImportRequest;
use crate::store::Store;
use serde::{Deserialize, Serialize};

/// The provenance tier this import was admitted under (aegis-c9c44).
///
/// TWO TIERS, NAMED, AND NEITHER IS SILENCE. `verify_share` proves the bytes are
/// intact; it says nothing about who produced them, and before this those were
/// the same field in a reader's head. An unattested share is admitted — that is
/// the existing behaviour and it stays — but it now says so.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttestationStatus {
    /// `"attested"` when a valid envelope bound the manifest identity to a
    /// registered session; `"transport"` when none was supplied.
    pub tier: String,
    /// The verified agent, present only at `attested`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// The verified session, present only at `attested`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Why this tier, in the words a reader needs.
    pub note: String,
}

/// Verify the share attestation, or record explicitly that there was none.
///
/// ⚠ ABSENCE IS A TIER, NOT A PASS. Returning `Ok` for a share with no envelope is
/// the existing behaviour and it stays — refusing unattested shares would break
/// every current caller and is a policy decision nobody has made. What changes is
/// that the result SAYS which world it is in. Before this, "the hashes verified"
/// and "we know who sent it" were the same green to a reader.
pub fn verify_attestation(
    store: &Store,
    request: &ShareImportRequest,
    timestamp: &str,
) -> Result<AttestationStatus> {
    let Some(envelope) = request.attestation.as_ref() else {
        return Ok(AttestationStatus {
            tier: "transport".into(),
            agent: None,
            session: None,
            note: "no attestation envelope was supplied: the payload hashes verify, so the bytes are intact, but nothing here proves WHO produced them"
                .into(),
        });
    };

    // The binding is the manifest's own identity fields — the same four the
    // producer signed. Building it from anything else would verify a statement
    // about a different share.
    let binding = crate::session_attestation::SignedBinding::Share(
        crate::session_attestation::ShareBinding {
            share_id: &request.manifest.share_id,
            graph_hash: &request.manifest.graph_hash,
            shapes_hash: &request.manifest.shapes_hash,
            tx_anchor: request.manifest.tx_anchor,
        },
    );

    // The import's own timestamp, not `now()`: an import replayed from a recorded
    // request must reach the same verdict, and a wall clock here would make the
    // skew window depend on when you happened to run it.
    let now_epoch = epoch_of(timestamp)?;
    let principal = crate::session_attestation::verify_binding(
        store,
        envelope,
        &binding,
        now_epoch,
        ATTESTATION_SKEW_SECS,
    )?;
    Ok(AttestationStatus {
        tier: "attested".into(),
        agent: Some(principal.agent),
        session: Some(principal.session),
        note: "envelope verified against a registered session binding over this manifest's identity; the nonce is spent"
            .into(),
    })
}

/// Accepted clock skew between the attestation's issuance and this import.
const ATTESTATION_SKEW_SECS: u64 = 300;

/// Seconds since the epoch for an ISO-8601 `YYYY-MM-DDTHH:MM:SSZ` timestamp.
///
/// Hand-parsed rather than reaching for a date crate: this file has no such
/// dependency, the format is the one every quipu timestamp already uses, and a
/// REFUSAL on anything else is better than a lenient parser quietly yielding a
/// number that moves the skew window.
fn epoch_of(timestamp: &str) -> Result<u64> {
    let bad = || Error::InvalidValue(format!("import timestamp {timestamp:?} is not ISO-8601 Z"));
    let t = timestamp.strip_suffix('Z').ok_or_else(bad)?;
    let (date, time) = t.split_once('T').ok_or_else(bad)?;
    let mut d = date.split('-');
    let (y, mo, da) = (
        d.next()
            .ok_or_else(bad)?
            .parse::<i64>()
            .map_err(|_| bad())?,
        d.next()
            .ok_or_else(bad)?
            .parse::<i64>()
            .map_err(|_| bad())?,
        d.next()
            .ok_or_else(bad)?
            .parse::<i64>()
            .map_err(|_| bad())?,
    );
    if d.next().is_some() {
        return Err(bad());
    }
    let mut c = time.split(':');
    let (h, mi, sec) = (
        c.next()
            .ok_or_else(bad)?
            .parse::<i64>()
            .map_err(|_| bad())?,
        c.next()
            .ok_or_else(bad)?
            .parse::<i64>()
            .map_err(|_| bad())?,
        c.next()
            .ok_or_else(bad)?
            .split('.')
            .next()
            .ok_or_else(bad)?
            .parse::<i64>()
            .map_err(|_| bad())?,
    );
    if !(1..=12).contains(&mo) || !(1..=31).contains(&da) || h > 23 || mi > 59 || sec > 60 {
        return Err(bad());
    }
    // Days from the civil epoch (Howard Hinnant's algorithm) — exact, no crate.
    let y2 = y - i64::from(mo <= 2);
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + da - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + h * 3600 + mi * 60 + sec)
        .map_err(|_| Error::InvalidValue(format!("import timestamp {timestamp:?} predates 1970")))
}
