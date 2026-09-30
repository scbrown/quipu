//! The human trust root: registry amendments only when signed by an enrolled
//! human key (S3), and the one-time console bootstrap (aegis-kzt0ql.9.4).
//!
//! # The defect this closes
//!
//! `aegis:VerifierRegistration` was graph-writable. Any writer holding the
//! bearer could register its own key as `verifier "stiwi"` for a decision
//! policy, and a verdict it then signed verified as Stiwi's. No governance
//! flag stopped it: `enforce_graph_authority` does nothing without a principal
//! chain, and nothing in production sets one.
//!
//! # Two tiers
//!
//! A registration is HUMAN-tier iff it carries an asserted
//! `aegis:trustTier "human"` in ROOT ([`Scope::HumanTier`]). Human decisions
//! (the decision seal, the escalation router) verify only against human-tier
//! keys. Agent registrations (shuttle's identity graph, certifiers) stay
//! writable as before and can never verify a human decision.
//!
//! # The gate compares STATE, not datums
//!
//! Before a write is staged, [`snapshot`] records every human-tier
//! registration and the RDFC digest of its ROOT content. After staging,
//! [`check`] recomputes it. ANY difference (a new registration, an edit, a
//! revocation, the marker added to or removed from an entity) must be covered
//! by an `aegis:RegistryAmendment` in the same transaction, signed over
//!
//! ```text
//! quipu-registry-amendment-v1|<store id>|<registration>|<new digest | "revoked">|<nonce>
//! ```
//!
//! by a key that was human-tier and authorized for [`TRUST_ROOT_POLICY`]
//! BEFORE the write. A signer cannot enrol itself in the same write. Comparing
//! state rather than filtering datums also catches changes no datum names,
//! such as functional-property supersede closures.
//!
//! # No flag, on purpose
//!
//! This gate has no configuration switch and is not skipped while recording
//! verdicts. A trust root that an agent can turn off by editing config, or by
//! reaching a code path that sets an internal flag, is not a trust root. Tests
//! enrol test keys through [`bootstrap`] and sign real amendments. Do not add
//! a switch "for tests".
//!
//! # Bootstrap
//!
//! The first human key is enrolled by [`bootstrap`], reached only from the
//! local CLI. It refuses if a human-tier marker has EVER been asserted in ROOT
//! (read from the full bitemporal history), so it cannot be re-run to add a
//! key: a second device is an amendment signed by the first. It requires
//! proof of possession, and it records an `aegis:TrustRootBootstrap` carrying
//! the key's fingerprint, which the ceremony checks against the device.
//!
//! # Stated limits
//!
//! Proof of possession proves the key, not the person. Until the ceremony,
//! anyone who can run the CLI on the quipu host could bootstrap their own key
//! first. The runbook's fingerprint comparison and a standing alert on the
//! bootstrap record exist for that window. Root on the quipu host (the DB
//! file, the binary, the offline import/unpack/restore paths, a physical
//! delete) defeats all of this. This gate constrains graph writers, not hosts.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::store::{Datum, Store};
use crate::types::{Op, Value};

use super::verifier_registry::{HUMAN_TIER, TRUST_TIER};

/// The policy a human key must attest to amend the registry.
pub const TRUST_ROOT_POLICY: &str = "urn:quipu:policy:trust-root";
/// The amendment message format.
pub const AMENDMENT_SCHEME: &str = "quipu-registry-amendment-v1";
/// The bootstrap proof-of-possession message format.
pub const BOOTSTRAP_SCHEME: &str = "quipu-trust-root-bootstrap-v1";
/// The digest an amendment names when it revokes a registration.
pub const REVOKED: &str = "revoked";

/// Registration fields that record USE, not what was enrolled. The hardware
/// schemes persist a signature counter after every verdict; sealing it would
/// make every human verdict a registry amendment.
const UNSEALED: [&str; 1] = ["http://aegis.gastown.local/ontology/signCount"];

fn ns(name: &str) -> String {
    format!("{DEFAULT_BASE_NS}{name}")
}

/// The human-tier registry before a write: each registration's digest, and
/// the (verifier, key) pairs authorized to amend it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    registrations: BTreeMap<String, String>,
    amenders: Vec<(String, String)>,
}

/// Every entity carrying an asserted ROOT `trustTier "human"` now, with the
/// digest of its ROOT content.
fn human_registry(store: &Store) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let Some(tier) = store.lookup(TRUST_TIER)? else {
        return Ok(out);
    };
    let mut stmt = store.prepare(
        "SELECT DISTINCT e FROM facts WHERE a = ?1 AND v = ?2 AND g = 0 \
         AND op = 1 AND valid_to IS NULL",
    )?;
    let human = Value::Str(HUMAN_TIER.into()).to_bytes();
    let ids = stmt
        .query_map(params![tier, human], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in ids {
        let iri = store.resolve(id)?;
        let digest =
            super::decision_seal::content_digest(store, &iri, &UNSEALED)?.unwrap_or_default();
        out.insert(iri, digest);
    }
    Ok(out)
}

/// Current ROOT string values of `entity`'s `predicate`.
fn strings(store: &Store, entity: &str, predicate: &str) -> Result<Vec<String>> {
    let (Some(e), Some(a)) = (store.lookup(entity)?, store.lookup(predicate)?) else {
        return Ok(Vec::new());
    };
    Ok(store
        .entity_facts(e)?
        .into_iter()
        .filter(|f| f.attribute == a)
        .filter_map(|f| match f.value {
            Value::Str(s) | Value::Typed { lexical: s, .. } => Some(s),
            _ => None,
        })
        .collect())
}

fn sole(values: Vec<String>) -> Option<String> {
    let mut values = values;
    if values.len() == 1 {
        values.pop()
    } else {
        None
    }
}

fn is_registration(store: &Store, entity: &str) -> Result<bool> {
    let (Some(e), Some(t), Some(c)) = (
        store.lookup(entity)?,
        store.lookup(RDF_TYPE)?,
        store.lookup(&ns("VerifierRegistration"))?,
    ) else {
        return Ok(false);
    };
    Ok(store
        .entity_facts(e)?
        .into_iter()
        .any(|f| f.attribute == t && f.value == Value::Ref(c)))
}

/// Record the human-tier registry before a write is staged.
pub(crate) fn snapshot(store: &Store) -> Result<Snapshot> {
    let registrations = human_registry(store)?;
    let mut amenders = Vec::new();
    for iri in registrations.keys() {
        if !is_registration(store, iri)?
            || !strings(store, iri, &ns("attests"))?
                .iter()
                .any(|p| p == TRUST_ROOT_POLICY)
        {
            continue;
        }
        let Some(verifier) = sole(strings(store, iri, &ns("verifier"))?) else {
            continue;
        };
        for key in strings(store, iri, &ns("publicKey"))? {
            amenders.push((verifier.clone(), key));
        }
    }
    Ok(Snapshot {
        registrations,
        amenders,
    })
}

/// Current human-tier registrations as (registration, verifier, key).
pub fn human_keys(store: &Store) -> Result<Vec<(String, String, String)>> {
    let mut out = Vec::new();
    for iri in human_registry(store)?.keys() {
        let verifier = sole(strings(store, iri, &ns("verifier"))?).unwrap_or_default();
        for key in strings(store, iri, &ns("publicKey"))? {
            out.push((iri.clone(), verifier.clone(), key));
        }
    }
    Ok(out)
}

/// The canonical bytes a human key signs to amend `registration`.
#[must_use]
pub fn amendment_message(store_id: &str, registration: &str, digest: &str, nonce: &str) -> Vec<u8> {
    format!("{AMENDMENT_SCHEME}|{store_id}|{registration}|{digest}|{nonce}").into_bytes()
}

/// The canonical bytes a new key signs to prove possession at bootstrap.
#[must_use]
pub fn bootstrap_message(store_id: &str, verifier: &str, public_key: &str) -> Vec<u8> {
    format!("{BOOTSTRAP_SCHEME}|{store_id}|{verifier}|{public_key}").into_bytes()
}

/// The digest an amendment must name for `registration` as the pending write
/// would leave it (`"revoked"` when it would no longer be human-tier).
pub fn pending_digest(store: &Store, registration: &str) -> Result<String> {
    Ok(human_registry(store)?
        .remove(registration)
        .unwrap_or_else(|| REVOKED.to_string()))
}

/// The digest an amendment must name for `registration` if `datums` were
/// applied: what a client signs BEFORE it writes. The registration's ROOT
/// closure (itself and the blank nodes it reaches) is copied, the datums whose
/// subject is the registration or a blank node are applied to that copy, and
/// the result is digested in a scratch store. `"revoked"` when the result
/// would no longer be human-tier. Functional-property supersede is not
/// modelled: close a prior value explicitly in `datums`.
pub fn digest_after(store: &Store, registration: &str, datums: &[Datum]) -> Result<String> {
    // (subject, predicate, object) with every term as an IRI or literal.
    let term = |v: &Value| -> Result<Value> {
        Ok(match v {
            Value::Ref(id) => Value::Str(format!("\u{0}ref:{}", store.resolve(*id)?)),
            other => other.clone(),
        })
    };
    let mut triples: Vec<(String, String, Value)> = Vec::new();
    if let Some(root) = store.lookup(registration)? {
        let mut queue = vec![root];
        let mut seen = std::collections::BTreeSet::new();
        while let Some(subject) = queue.pop() {
            if !seen.insert(subject) {
                continue;
            }
            let s = store.resolve(subject)?;
            for f in store.entity_facts(subject)? {
                if let Value::Ref(o) = &f.value
                    && store.resolve(*o)?.starts_with("_:")
                {
                    queue.push(*o);
                }
                triples.push((s.clone(), store.resolve(f.attribute)?, term(&f.value)?));
            }
        }
    }
    for d in datums {
        let s = store.resolve(d.entity)?;
        if s != registration && !s.starts_with("_:") {
            continue;
        }
        let triple = (s, store.resolve(d.attribute)?, term(&d.value)?);
        match d.op {
            Op::Assert => {
                if !triples.contains(&triple) {
                    triples.push(triple);
                }
            }
            _ => triples.retain(|t| *t != triple),
        }
    }
    let mut scratch = Store::open_in_memory()?;
    let mut staged = Vec::new();
    for (s, p, o) in &triples {
        let value = match o {
            Value::Str(x) if x.starts_with("\u{0}ref:") => {
                Value::Ref(scratch.intern(&x["\u{0}ref:".len()..])?)
            }
            other => other.clone(),
        };
        staged.push(Datum {
            entity: scratch.intern(s)?,
            attribute: scratch.intern(p)?,
            value,
            valid_from: "1970-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    if !staged.is_empty() {
        // An empty registry admitting one registration is exactly the
        // bootstrap shape, so the scratch copy needs no special path.
        scratch.transact_trust_root_bootstrap(registration, &staged, "1970-01-01T00:00:00Z")?;
    }
    pending_digest(&scratch, registration)
}

/// The OpenSSH-style fingerprint of an ed25519 public key (hex), as
/// `ssh-keygen -lf` prints it for the same key: `SHA256:<base64, no padding>`
/// over the SSH wire encoding.
pub fn fingerprint(public_key_hex: &str) -> Result<String> {
    let key = hex::decode(public_key_hex)
        .map_err(|_| Error::InvalidValue("public key is not hex".into()))?;
    if key.len() != 32 {
        return Err(Error::InvalidValue(
            "an ed25519 public key is 32 bytes".into(),
        ));
    }
    let mut wire = Vec::new();
    for part in [b"ssh-ed25519".as_slice(), key.as_slice()] {
        wire.extend_from_slice(&u32::try_from(part.len()).unwrap_or(0).to_be_bytes());
        wire.extend_from_slice(part);
    }
    Ok(format!("SHA256:{}", base64_nopad(&Sha256::digest(&wire))))
}

fn base64_nopad(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// Refuse the staged write unless every change to the human-tier registry is
/// covered by a valid amendment signed by a key enrolled BEFORE this write.
/// `bootstrap` names the one registration a console bootstrap may create.
pub(crate) fn check(
    store: &Store,
    before: &Snapshot,
    datums: &[Datum],
    bootstrap: Option<&str>,
) -> Result<()> {
    let after = human_registry(store)?;
    let mut changed: BTreeMap<&str, String> = BTreeMap::new();
    for (iri, digest) in &after {
        if before.registrations.get(iri) != Some(digest) {
            changed.insert(iri, digest.clone());
        }
    }
    for iri in before.registrations.keys() {
        if !after.contains_key(iri) {
            changed.insert(iri, REVOKED.to_string());
        }
    }
    if changed.is_empty() {
        return Ok(());
    }
    if let Some(expected) = bootstrap {
        if before.registrations.is_empty() && changed.len() == 1 && changed.contains_key(expected) {
            return Ok(());
        }
        return Err(refuse(
            "a bootstrap may create exactly one human registration in an empty registry",
        ));
    }
    let amendments = staged_amendments(store, datums)?;
    let store_id = store.store_id()?;
    for (registration, digest) in changed {
        let covered = amendments.iter().any(|a| {
            a.amends == registration
                && a.digest == digest
                && before.amenders.iter().any(|(verifier, key)| {
                    *verifier == a.signer
                        && crate::signing::verify_hex(
                            key,
                            &amendment_message(&store_id, registration, &digest, &a.nonce),
                            &a.signature,
                        )
                })
        });
        if !covered {
            return Err(refuse(&format!(
                "the human trust-root registration {registration} would change (to {digest}) \
                 without an aegis:RegistryAmendment signed by a key enrolled for {TRUST_ROOT_POLICY} \
                 before this write"
            )));
        }
    }
    // Spend every amendment's nonce inside this write's savepoint: a refusal
    // later in the write rolls the spend back with everything else.
    for a in &amendments {
        if !store.spend_registry_amendment_nonce(&a.nonce, &a.amends, &a.iri)? {
            return Err(refuse(&format!(
                "registry amendment nonce {} was already used",
                a.nonce
            )));
        }
    }
    Ok(())
}

fn refuse(why: &str) -> Error {
    Error::PolicyDenied(format!("trust root (aegis-kzt0ql.9.4): {why}"))
}

#[derive(Debug)]
struct Amendment {
    iri: String,
    amends: String,
    digest: String,
    signer: String,
    nonce: String,
    signature: String,
}

/// Amendments this write asserts. Each field must be single-valued; an
/// ambiguous amendment is ignored, so it covers nothing.
fn staged_amendments(store: &Store, datums: &[Datum]) -> Result<Vec<Amendment>> {
    let (Some(t), Some(c)) = (
        store.lookup(RDF_TYPE)?,
        store.lookup(&ns("RegistryAmendment"))?,
    ) else {
        return Ok(Vec::new());
    };
    let Some(amends_attr) = store.lookup(&ns("amends"))? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for d in datums {
        if d.op != Op::Assert || d.attribute != t || d.value != Value::Ref(c) {
            continue;
        }
        let iri = store.resolve(d.entity)?;
        let refs: Vec<i64> = store
            .entity_facts(d.entity)?
            .into_iter()
            .filter(|f| f.attribute == amends_attr)
            .filter_map(|f| {
                if let Value::Ref(r) = f.value {
                    Some(r)
                } else {
                    None
                }
            })
            .collect();
        let amends = match refs.as_slice() {
            [one] => store.resolve(*one)?,
            _ => continue,
        };
        let (Some(digest), Some(signer), Some(nonce), Some(signature)) = (
            sole(strings(store, &iri, &ns("amendedDigest"))?),
            sole(strings(store, &iri, &ns("amendmentSigner"))?),
            sole(strings(store, &iri, &ns("amendmentNonce"))?),
            sole(strings(store, &iri, &ns("amendmentSignature"))?),
        ) else {
            continue;
        };
        if nonce.len() < 32 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        out.push(Amendment {
            iri,
            amends,
            digest,
            signer,
            nonce,
            signature,
        });
    }
    Ok(out)
}

/// What a bootstrap enrolled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enrolled {
    /// The registration IRI.
    pub registration: String,
    /// The key's `SHA256:` fingerprint, to compare against the device.
    pub fingerprint: String,
}

/// Whether a human-tier marker has EVER been asserted in ROOT, including
/// closed and retracted intervals.
pub fn ever_bootstrapped(store: &Store) -> Result<bool> {
    let Some(tier) = store.lookup(TRUST_TIER)? else {
        return Ok(false);
    };
    let human = Value::Str(HUMAN_TIER.into()).to_bytes();
    Ok(store
        .prepare("SELECT 1 FROM facts WHERE a = ?1 AND v = ?2 AND g = 0 AND op = 1 LIMIT 1")?
        .query_row(params![tier, human], |row| row.get::<_, i64>(0))
        .optional()?
        .is_some())
}

/// Enrol the FIRST human key. CLI-only (the console ceremony); never exposed
/// over REST or MCP. The registration always attests [`TRUST_ROOT_POLICY`],
/// plus `attests`.
pub fn bootstrap(
    store: &mut Store,
    verifier: &str,
    public_key: &str,
    attests: &[String],
    pop_signature: &str,
    timestamp: &str,
) -> Result<Enrolled> {
    if ever_bootstrapped(store)? {
        return Err(refuse(
            "a human key has already been enrolled in this store; a further device is an \
             amendment signed by an enrolled key, never a second bootstrap",
        ));
    }
    let fingerprint = fingerprint(public_key)?;
    if !crate::signing::verify_hex(
        public_key,
        &bootstrap_message(&store.store_id()?, verifier, public_key),
        pop_signature,
    ) {
        return Err(refuse(
            "the proof of possession does not verify under the key being enrolled",
        ));
    }
    let digest = hex::encode(Sha256::digest(public_key.as_bytes()));
    let registration = ns(&format!("human_registration_{}", &digest[..16]));
    let record = ns(&format!("trust_root_bootstrap_{}", &digest[..16]));
    let mut policies = vec![TRUST_ROOT_POLICY.to_string()];
    policies.extend(attests.iter().filter(|p| *p != TRUST_ROOT_POLICY).cloned());
    let datum = |store: &Store, e: &str, p: &str, v: Value| -> Result<Datum> {
        Ok(Datum {
            entity: store.intern(e)?,
            attribute: store.intern(p)?,
            value: v,
            valid_from: timestamp.to_string(),
            valid_to: None,
            op: Op::Assert,
        })
    };
    let mut datums = vec![
        datum(
            store,
            &registration,
            RDF_TYPE,
            Value::Ref(store.intern(&ns("VerifierRegistration"))?),
        )?,
        datum(
            store,
            &registration,
            &ns("verifier"),
            Value::Str(verifier.into()),
        )?,
        datum(
            store,
            &registration,
            &ns("publicKey"),
            Value::Str(public_key.into()),
        )?,
        datum(
            store,
            &registration,
            TRUST_TIER,
            Value::Str(HUMAN_TIER.into()),
        )?,
        datum(
            store,
            &record,
            RDF_TYPE,
            Value::Ref(store.intern(&ns("TrustRootBootstrap"))?),
        )?,
        datum(
            store,
            &record,
            &ns("enrolled"),
            Value::Ref(store.intern(&registration)?),
        )?,
        datum(
            store,
            &record,
            &ns("keyFingerprint"),
            Value::Str(fingerprint.clone()),
        )?,
    ];
    for p in &policies {
        datums.push(datum(
            store,
            &registration,
            &ns("attests"),
            Value::Str(p.clone()),
        )?);
    }
    store.transact_trust_root_bootstrap(&registration, &datums, timestamp)?;
    Ok(Enrolled {
        registration,
        fingerprint,
    })
}

#[cfg(test)]
#[path = "trust_root_tests.rs"]
mod tests;

/// Test support: a deterministic TEST trust root signing real amendments.
#[cfg(test)]
#[path = "trust_root_test_support.rs"]
pub(crate) mod test_support;
