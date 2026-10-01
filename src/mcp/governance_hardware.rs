//! `quipu_verdict_verify` for the hardware verdict schemes
//! (`crate::verdict_schemes`): the registry reads and the dispatch to the
//! `WebAuthn` / SSHSIG verifiers. The v1 ed25519 path stays in
//! [`super::governance`]; this module is reached only for a verdict naming a
//! hardware `scheme`.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value as JsonValue;

use super::governance::{guard_iri, run_ask};
use super::verdict_witness::{is_registered_verifier, witness_json};
use crate::error::{Error, Result};
use crate::governance::verifier_registry::Witness;
use crate::sparql::{self, QueryResult, TemporalContext};
use crate::store::Store;
use crate::verdict_schemes::{Scheme, b64url_decode, disabled_message, sshsig, webauthn};

/// Escape a value for safe inlining as a SPARQL string literal (reject the
/// characters that could break out of the quotes or inject).
fn sparql_string_literal(value: &str) -> Result<String> {
    if value.contains(['"', '\n', '\r', '\\']) {
        return Err(Error::InvalidValue(
            "value must not contain a quote, backslash, or newline".into(),
        ));
    }
    Ok(format!("\"{value}\""))
}

/// A verdict naming a hardware scheme takes this module's path, read against
/// the SAME witness as the ed25519 path: the recorded-basis rule binds every
/// scheme. `None` (no scheme, or "ed25519") means the caller continues on the
/// v1 path.
pub(super) fn dispatch(
    store: &Store,
    input: &JsonValue,
    claim: &Claim<'_>,
    witness: &Witness,
    basis: &str,
) -> Result<Option<JsonValue>> {
    let Some(tag) = input.get("scheme").and_then(JsonValue::as_str) else {
        return Ok(None);
    };
    let scheme = Scheme::parse(tag).map_err(Error::InvalidValue)?;
    if !scheme.is_hardware() {
        return Ok(None);
    }
    verify_hardware_verdict(store, scheme, input, claim, witness, basis).map(Some)
}

/// The registry as it stood at `witness` (signing-plane S1): valid at its
/// instant and, when it names one, as of its transaction. Every registry read
/// on the hardware path goes through this, so a registration enrolled, revoked
/// or re-scoped after the signature cannot decide the verdict.
fn as_of(witness: &Witness) -> TemporalContext {
    TemporalContext {
        valid_at: Some(witness.at.clone()),
        as_of_tx: witness.tx,
        ..TemporalContext::default()
    }
}

/// The verdict fields the hardware path verifies.
pub(super) struct Claim<'a> {
    pub verifier: &'a str,
    pub predicate_id: &'a str,
    pub message: &'a [u8],
    pub signature: &'a str,
}

/// One hardware-scheme registration of a verifier, as read from the graph.
struct HardwareRegistration {
    iri: String,
    public_key: String,
    rp_id: Option<String>,
    origin: Option<String>,
}

/// Every registration of `verifier` declaring `scheme`, one entry per
/// registration IRI. A registration carrying more than one `aegis:publicKey`,
/// `aegis:webauthnRpId` or `aegis:webauthnOrigin` is AMBIGUOUS: it is returned
/// as an `Err` reason rather than letting any pairing of its values verify.
fn hardware_registrations(
    store: &Store,
    verifier: &str,
    scheme: Scheme,
    ctx: &TemporalContext,
) -> Result<Vec<std::result::Result<HardwareRegistration, String>>> {
    let v = sparql_string_literal(verifier)?;
    let s = sparql_string_literal(scheme.tag())?;
    let q = format!(
        // Dual-read (aegis-9dpcta): the class, verifier and publicKey match in the
        // legacy or the Quechua namespace (VALUES/UNION; property-path
        // alternation is unreliable, aegis-sxlptn). signatureScheme and the
        // webauthn terms have no published twin yet, so they stay legacy-only.
        "PREFIX a: <http://aegis.gastown.local/ontology/> \
         PREFIX q: <https://scbrown.github.io/quechua/ns#> \
         SELECT ?r ?k ?rp ?o WHERE {{ {{ ?r a a:VerifierRegistration }} UNION {{ ?r a q:VerifierRegistration }} \
         VALUES ?_verifier {{ a:verifier q:verifier }} VALUES ?_key {{ a:publicKey q:publicKey }} \
         ?r ?_verifier {v} ; a:signatureScheme {s} ; ?_key ?k . \
         OPTIONAL {{ ?r a:webauthnRpId ?rp }} OPTIONAL {{ ?r a:webauthnOrigin ?o }} }}"
    );
    let text = |row: &std::collections::HashMap<String, crate::types::Value>, k: &str| {
        row.get(k).and_then(|v| match v {
            crate::types::Value::Str(s) => Some(s.clone()),
            _ => None,
        })
    };
    let QueryResult::Select { rows, .. } = sparql::query_temporal(store, &q, ctx)? else {
        return Ok(Vec::new());
    };
    // iri -> (keys, rp ids, origins); BTree for a deterministic order.
    type Values = (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>);
    let mut grouped: BTreeMap<String, Values> = BTreeMap::new();
    for row in &rows {
        let (Some(crate::types::Value::Ref(r)), Some(key)) = (row.get("r"), text(row, "k")) else {
            continue;
        };
        let entry = grouped.entry(store.resolve(*r)?).or_default();
        entry.0.insert(key);
        entry.1.extend(text(row, "rp"));
        entry.2.extend(text(row, "o"));
    }
    Ok(grouped
        .into_iter()
        .map(|(iri, (keys, rps, origins))| {
            if keys.len() > 1 || rps.len() > 1 || origins.len() > 1 {
                return Err(format!(
                    "{iri}: ambiguous registration (more than one publicKey, webauthnRpId \
                     or webauthnOrigin)"
                ));
            }
            Ok(HardwareRegistration {
                iri,
                public_key: keys.into_iter().next().unwrap_or_default(),
                rp_id: rps.into_iter().next(),
                origin: origins.into_iter().next(),
            })
        })
        .collect())
}

/// The highest `aegis:signCount` recorded on a registration (0 when none).
fn recorded_sign_count(store: &Store, registration: &str, ctx: &TemporalContext) -> Result<u32> {
    guard_iri(registration)?;
    let q = format!(
        "SELECT ?c WHERE {{ <{registration}> <http://aegis.gastown.local/ontology/signCount> ?c }}"
    );
    let QueryResult::Select { rows, .. } = sparql::query_temporal(store, &q, ctx)? else {
        return Ok(0);
    };
    let mut max = 0u32;
    for value in rows.iter().filter_map(|r| r.get("c")) {
        let n = match value {
            crate::types::Value::Int(i) => u32::try_from(*i).ok(),
            crate::types::Value::Str(s) => s.parse::<u32>().ok(),
            _ => None,
        }
        .ok_or_else(|| {
            Error::InvalidValue(format!(
                "registration {registration} has a non-numeric aegis:signCount"
            ))
        })?;
        max = max.max(n);
    }
    Ok(max)
}

/// Does THIS registration authorize `predicate_id`? For hardware verdicts the
/// key that verified and the grant that authorizes must be one registration,
/// so a device enrolled for one predicate cannot borrow another registration's
/// grant under the same verifier name.
fn registration_attests(
    store: &Store,
    registration: &str,
    predicate_id: &str,
    ctx: &TemporalContext,
) -> Result<bool> {
    guard_iri(registration)?;
    let p = sparql_string_literal(predicate_id)?;
    let ask = format!(
        "PREFIX a: <http://aegis.gastown.local/ontology/> \
         PREFIX q: <https://scbrown.github.io/quechua/ns#> \
         ASK {{ VALUES ?_attests {{ a:attests q:attests }} <{registration}> ?_attests {p} }}"
    );
    run_ask(store, &ask, ctx)
}

/// The decoded `WebAuthn` assertion parts: `(authenticatorData,
/// clientDataJSON, signature)`.
type WebauthnParts = (Vec<u8>, Vec<u8>, Vec<u8>);

fn webauthn_parts(input: &JsonValue, signature: &str) -> Result<WebauthnParts> {
    let part = |k: &str| -> Result<Vec<u8>> {
        let raw = input
            .get(k)
            .and_then(JsonValue::as_str)
            .ok_or_else(|| Error::InvalidValue(format!("missing '{k}' parameter")))?;
        b64url_decode(raw).map_err(|e| Error::InvalidValue(format!("'{k}': {e}")))
    };
    Ok((
        part("authenticator_data")?,
        part("client_data_json")?,
        b64url_decode(signature).map_err(|e| Error::InvalidValue(format!("'signature': {e}")))?,
    ))
}

/// Verify `message` against one registration.
fn verify_one(
    scheme: Scheme,
    reg: &HardwareRegistration,
    recorded: u32,
    parts: Option<&WebauthnParts>,
    signature: &str,
    message: &[u8],
) -> std::result::Result<crate::verdict_schemes::Verified, String> {
    match (parts, scheme) {
        (Some((ad, cdj, sig)), _) => {
            let (Some(rp_id), Some(origin)) = (&reg.rp_id, &reg.origin) else {
                return Err(
                    "a WebAuthn registration needs aegis:webauthnRpId and aegis:webauthnOrigin"
                        .into(),
                );
            };
            let cose_key = b64url_decode(&reg.public_key)?;
            webauthn::verify(
                scheme,
                &webauthn::Registered {
                    cose_key: &cose_key,
                    rp_id,
                    origin,
                    recorded_sign_count: recorded,
                },
                &webauthn::Assertion {
                    authenticator_data: ad,
                    client_data_json: cdj,
                    signature: sig,
                },
                message,
            )
        }
        (None, Scheme::SshsigSkEd25519) => {
            sshsig::verify_sk(signature, &reg.public_key, recorded, message)
        }
        (None, _) => Err("scheme has no verifier".into()),
    }
}

/// Verify a verdict signed under a hardware scheme.
///
/// Refused outright (an error, not `trusted: false`) while
/// `[quipu.governance] hardware_verdict_schemes` is off. Otherwise every
/// registration of the verifier declaring the scheme, as it stood at `witness`,
/// is tried; the verdict is trusted iff one of them both verifies the signature
/// and attests the predicate, AND `basis` is one quipu chose (`recorded` or
/// `now`). A caller-supplied instant is a what-if, exactly as on the ed25519
/// path: the answer goes in `would_verify_as_of_supplied_instant` and `trusted`
/// is false. Scanning continues past a registration that verifies but does not
/// authorize, so enrolment order cannot decide the outcome. The reported
/// `sign_count` is the counter to record as that registration's new
/// `aegis:signCount`; this read-only check does not write it.
fn verify_hardware_verdict(
    store: &Store,
    scheme: Scheme,
    input: &JsonValue,
    claim: &Claim<'_>,
    witness: &Witness,
    basis: &str,
) -> Result<JsonValue> {
    if !store.governance_config().hardware_verdict_schemes {
        return Err(Error::InvalidValue(disabled_message(scheme.tag())));
    }
    let ctx = as_of(witness);
    let registrations = hardware_registrations(store, claim.verifier, scheme, &ctx)?;
    let verifier_authorized =
        is_registered_verifier(store, claim.verifier, claim.predicate_id, witness)?;
    let parts = if matches!(scheme, Scheme::WebauthnEs256 | Scheme::WebauthnEddsa) {
        Some(webauthn_parts(input, claim.signature)?)
    } else {
        None
    };
    // An ALLOW-list, as on the ed25519 path: a basis added later fails closed.
    let trustworthy_basis = matches!(basis, "recorded" | "now");
    let verdict = |seal_ok: bool, mut out: JsonValue| {
        out["trusted"] = JsonValue::Bool(trustworthy_basis && seal_ok);
        out["would_verify_as_of_supplied_instant"] = if trustworthy_basis {
            JsonValue::Null
        } else {
            JsonValue::Bool(seal_ok)
        };
        out["as_of"] = witness_json(witness, basis);
        out
    };

    let mut reasons: Vec<String> = Vec::new();
    let mut unauthorized: Option<JsonValue> = None;
    for reg in &registrations {
        let reg = match reg {
            Ok(reg) => reg,
            Err(reason) => {
                reasons.push(reason.clone());
                continue;
            }
        };
        let recorded = recorded_sign_count(store, &reg.iri, &ctx)?;
        match verify_one(
            scheme,
            reg,
            recorded,
            parts.as_ref(),
            claim.signature,
            claim.message,
        ) {
            Ok(v) => {
                let authorized = registration_attests(store, &reg.iri, claim.predicate_id, &ctx)?;
                let out = serde_json::json!({
                    "scheme": scheme.tag(),
                    "signature_valid": true,
                    "verifier_registered": true,
                    "verifier_authorized": verifier_authorized,
                    "registration": reg.iri,
                    "registration_authorized": authorized,
                    "user_present": v.user_present,
                    "user_verified": v.user_verified,
                    "sign_count": v.sign_count,
                });
                if authorized {
                    return Ok(verdict(true, out));
                }
                unauthorized.get_or_insert(out);
            }
            Err(reason) => reasons.push(format!("{}: {reason}", reg.iri)),
        }
    }
    if let Some(out) = unauthorized {
        return Ok(verdict(false, out));
    }

    if registrations.is_empty() {
        reasons.push(format!(
            "verifier has no aegis:VerifierRegistration declaring scheme '{}' in effect at \
             the verdict's instant",
            scheme.tag()
        ));
    }
    Ok(verdict(
        false,
        serde_json::json!({
            "scheme": scheme.tag(),
            "signature_valid": false,
            "verifier_registered": !registrations.is_empty(),
            "verifier_authorized": verifier_authorized,
            "reasons": reasons
        }),
    ))
}

#[cfg(test)]
#[path = "governance_hardware_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "governance_hardware_asof_tests.rs"]
mod asof_tests;
