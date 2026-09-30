//! Which registry instant an MCP verification asks about (signing-plane S1,
//! aegis-kzt0ql.9.1). Split from `governance.rs` for the file-size ratchet.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::governance::verifier_registry::{Scope, Witness};
use crate::store::Store;

/// The registry instant a verification asks about (signing-plane S1).
///
/// * `verdict` (an IRI): the store-witnessed instant at which that verdict's
///   `aegis:signature` was recorded. This is the form a trust decision
///   should use. The signer cannot choose it.
/// * `signed_at` / `tx`: an explicit instant, for "would this have verified
///   then?" questions. The caller chose it, and the output says so.
/// * neither: now.
pub(super) fn witness_from(
    store: &Store,
    input: &JsonValue,
    signature: &str,
) -> Result<(Witness, &'static str)> {
    if let Some(verdict) = input.get("verdict").and_then(JsonValue::as_str) {
        super::governance::guard_iri(verdict)?;
        let witness = Witness::of_fact(
            store,
            verdict,
            "http://aegis.gastown.local/ontology/signature",
            signature,
        )?
        .ok_or_else(|| {
            Error::InvalidValue(format!(
                "verdict '{verdict}' has no recorded aegis:signature with that value; \
                 there is no store-witnessed instant to verify it at"
            ))
        })?;
        return Ok((witness, "recorded"));
    }
    Ok(explicit_witness(input))
}

/// An instant the caller named (`signed_at` / `tx`), or now.
pub(super) fn explicit_witness(input: &JsonValue) -> (Witness, &'static str) {
    let at = input.get("signed_at").and_then(JsonValue::as_str);
    let tx = input.get("tx").and_then(JsonValue::as_i64);
    if at.is_none() && tx.is_none() {
        return (Witness::now(), "now");
    }
    let at = at.map_or_else(crate::time::now_iso, str::to_string);
    (Witness { tx, at }, "caller-supplied")
}

pub(super) fn witness_json(witness: &Witness, basis: &str) -> JsonValue {
    serde_json::json!({ "tx": witness.tx, "at": witness.at, "basis": basis })
}

/// Is `verifier` registered (Phase-0 root of trust) to attest `predicate_id`
/// at `witness`? True iff an `aegis:VerifierRegistration` in effect then names
/// both. A governed authority check, independent of (and prior to) signing.
pub(super) fn is_registered_verifier(
    store: &Store,
    verifier: &str,
    predicate_id: &str,
    witness: &Witness,
) -> Result<bool> {
    crate::governance::verifier_registry::is_authorized(
        store,
        verifier,
        predicate_id,
        witness,
        Scope::Root,
    )
}
