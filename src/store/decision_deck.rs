//! Native, default-off Decision Deck backend. No REST/MCP route or enrollment
//! ceremony is installed by this module. Database-file access is operator
//! authority; graph writers cannot author requests or enroll credentials here.
//! Assertions prove an enrolled credential, origin/RP and UP/UV, not an exact
//! URL path, physical device custody, or the identity of an unenrolled person.
//! # arming: library — callers invoke the native methods; no transport is armed.

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::{Datum, Store};
use crate::error::{Error, Result};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::types::{Op, Value};
use crate::verdict_schemes::{Scheme, webauthn};

mod admission;
#[cfg(test)]
mod golden_tests;
mod operator;
#[cfg(test)]
mod tests;
mod transaction;

/// Reserved, protected request and receipt projection. Explicit dataset only.
pub const GRAPH: &str = "urn:quipu:decision-deck";

/// Bounded explicit-graph audit query. The projection is protected against
/// generic writers, but query text and a returned row are not cryptographic
/// verification. Re-verify the exact protected receipt before trusting it.
pub fn audit_query() -> super::queries::StoredQuery {
    super::queries::StoredQuery {
        name: "decision_deck_audit_v1".into(),
        description: "Admitted Decision Deck receipts; re-verify each exact nonce using the native receipt API".into(),
        template: format!("PREFIX a: <{DEFAULT_BASE_NS}> SELECT ?decision ?outcome ?actor ?digest ?credential ?time ?scheme ?admission ?receipt FROM <{GRAPH}> WHERE {{ ?decision a a:Decision ; a:outcome ?outcome ; a:by ?actor ; a:evidenceHash ?digest ; a:deckCredential ?credential ; a:deckRecordedAt ?time ; a:deckScheme ?scheme ; a:deckAdmission ?admission ; a:deckReceipt ?receipt . }} ORDER BY ?decision LIMIT 200"),
        dataset: None,
        params: Vec::new(),
    }
}

/// Operator-installed application scope. Reconfiguration requires a new store
/// or a future explicitly reviewed migration; enrolled scope cannot drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Exact dedicated HTTPS origin, without a path.
    pub origin: String,
    /// Exact host of the dedicated origin (no parent-domain enrollment).
    pub rp_id: String,
    /// Independently established human identity.
    pub actor: String,
    /// Sole trusted request curator's identity.
    pub author: String,
}

/// A curated governed `DecisionRequest`, never inferred from free prose.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Request {
    /// Absolute `DecisionRequest` IRI.
    pub id: String,
    /// Authoritative strictly increasing version, assigned by the curator.
    pub revision: u32,
    /// Policy whose scope is being decided.
    pub policy: String,
    /// Target within that policy.
    pub target: String,
    /// Exact human-readable proposition, included in the signature binding.
    pub summary: String,
    /// Server epoch expiry of this request.
    pub expires_at: i64,
    /// Nonempty subset of approve/reject/changes.
    pub choices: Vec<String>,
    /// Curator identity; checked against the operator-installed author.
    pub author: String,
}

/// Public credential, independently enrolled by an operator. No private key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    /// Unpadded base64url authenticator credential ID.
    pub id: String,
    /// Supported `WebAuthn` scheme tag.
    pub scheme: String,
    /// COSE public key bytes.
    pub cose_key: Vec<u8>,
    /// Credential binding deadline; expiry never authorizes a new enrollment.
    pub expires_at: i64,
}

/// Short operator-readable fingerprint of a COSE public key: the first 16 hex
/// digits of its SHA-256, in groups of four. The enrollment page computes the
/// same value from the bytes it shows, and the operator compares the two before
/// trusting an enrollment. It names a key; it is not a security proof.
pub fn credential_fingerprint(cose_key: &[u8]) -> String {
    let digest = hex::encode(ring::digest::digest(&ring::digest::SHA256, cose_key));
    digest.as_bytes()[..16]
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

/// One enrolled credential as the operator sees it. No key material beyond
/// the public fingerprint.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CredentialStatus {
    /// Unpadded base64url credential ID.
    pub id: String,
    /// `WebAuthn` scheme tag.
    pub scheme: String,
    /// [`credential_fingerprint`] of the enrolled COSE key.
    pub fingerprint: String,
    /// Binding deadline (epoch seconds).
    pub expires_at: i64,
    /// Permanently revoked.
    pub revoked: bool,
    /// Last recorded authenticator counter.
    pub sign_count: u32,
}

/// Read-only operator view of the deck: scope, switch and enrollments.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeckStatus {
    /// Exact dedicated HTTPS origin.
    pub origin: String,
    /// Relying-party host.
    pub rp_id: String,
    /// Configured human identity.
    pub actor: String,
    /// Configured request curator.
    pub author: String,
    /// The store's operator switch (the server's `deck_http` is separate).
    pub enabled: bool,
    /// Every enrollment, including revoked and expired ones, oldest first.
    pub credentials: Vec<CredentialStatus>,
}

/// Complete canonical server-issued action. Serialize this struct, never a
/// caller-supplied JSON object. The `WebAuthn` challenge is derived over these
/// bytes using the existing published verifier domain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Challenge {
    /// New application domain; cannot be interpreted as a generic verdict.
    pub domain: String,
    /// Receiving store lineage, not physical-host identity.
    pub audience: String,
    /// Random 256-bit one-use identifier.
    pub nonce: String,
    /// Exact origin and RP fixed by the operator.
    pub origin: String,
    /// Relying party host.
    pub rp_id: String,
    /// Independently enrolled actor.
    pub actor: String,
    /// Selected credential.
    pub credential: String,
    /// Full curated request including its version, scope and proposition.
    pub request: Request,
    /// One declared choice, selected before signing.
    pub choice: String,
    /// Server-issued challenge deadline, at most 300 seconds.
    pub expires_at: i64,
}

impl Challenge {
    /// Exact message re-derived by the server at verification.
    pub fn message(&self) -> Result<Vec<u8>> {
        encode(self)
    }

    /// Raw challenge to supply to `navigator.credentials.get()`.
    pub fn webauthn_challenge(&self) -> Result<[u8; 32]> {
        Ok(crate::verdict_schemes::webauthn_challenge(&self.message()?))
    }
}

/// Retained immutable audit material. A read-only exact-nonce lookup reconciles
/// an indeterminate response; submitting again is refused, not a second action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    /// Deterministic decision IRI.
    pub decision: String,
    /// Complete signed challenge content.
    pub challenge: Challenge,
    /// Operator-enrolled public key snapshot for historical verification.
    pub cose_key: Vec<u8>,
    /// Actual scheme used.
    pub scheme: String,
    /// Raw assertion bytes (retain exactly, never reserialize client JSON).
    pub authenticator_data: Vec<u8>,
    /// Authenticator-signed client data.
    pub client_data_json: Vec<u8>,
    /// Authenticator signature.
    pub signature: Vec<u8>,
    /// Server-witnessed admission time, not the browser clock.
    pub recorded_at: i64,
}

fn refuse(message: &str) -> Error {
    Error::PolicyDenied(format!("decision deck: {message}"))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|e| Error::InvalidValue(e.to_string()))
}

fn decode<T: for<'a> Deserialize<'a>>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|e| Error::InvalidValue(e.to_string()))
}

fn now() -> Result<i64> {
    i64::try_from(crate::time::epoch_secs()).map_err(|_| refuse("invalid server clock"))
}

fn datum(store: &Store, id: &str, field: &str, value: Value, ts: &str) -> Result<Datum> {
    Ok(Datum {
        entity: store.intern(id)?,
        attribute: store.intern(&format!("{DEFAULT_BASE_NS}{field}"))?,
        value,
        valid_from: ts.into(),
        valid_to: None,
        op: Op::Assert,
    })
}

fn typed(store: &Store, id: &str, class: &str, ts: &str) -> Result<Datum> {
    Ok(Datum {
        entity: store.intern(id)?,
        attribute: store.intern(RDF_TYPE)?,
        value: Value::Ref(store.intern(&format!("{DEFAULT_BASE_NS}{class}"))?),
        valid_from: ts.into(),
        valid_to: None,
        op: Op::Assert,
    })
}
