//! The bitemporal verifier registry (signing-plane S1, aegis-kzt0ql.9.1).
//!
//! `aegis:VerifierRegistration` facts are ordinary graph facts, so they are
//! already bitemporal: every fact has a valid interval and the transaction
//! that recorded it. Until S1 every verifier asked the registry about NOW, so
//! rotating a key voided every decision, verdict and transition it had ever
//! signed. This module asks about the instant the signature was recorded
//! instead.
//!
//! # Which instant
//!
//! The signature's own claimed time cannot be used: whoever holds a revoked key
//! could back-date a signature into the window when the key was valid. Instead,
//! a [`Witness`] is what the STORE observed, on two axes:
//!
//! * **transaction** — the tx that first recorded the signature. Tx ids are
//!   monotonic and assigned by the store, so this ordering cannot be forged.
//!   A registration counts only if it was asserted at or before that tx and not
//!   yet closed by it. Rotation or revocation at tx R therefore rejects every
//!   signature recorded after R.
//! * **valid time** — the timestamp of that tx. The registration's valid
//!   interval, as currently known, must cover it. This is what lets a
//!   compromise revocation reach back: closing a key's `valid_to` at an earlier
//!   instant X distrusts what was recorded after X, even though it was recorded
//!   before the revocation itself. Transaction timestamps are writer-supplied,
//!   so this axis is best-effort; the transaction axis above is the one that
//!   cannot be defeated.
//!
//! Rotation is close-then-insert (retract the old `aegis:publicKey` and assert
//! the new one), revocation is a close, and expiry is a `valid_to` in the
//! future. No new schema is needed.
//!
//! # One registration, not two
//!
//! A key counts only when the key and its scope (`aegis:attests`) are facts of
//! the SAME registration. Asking "is some key registered?" and "is some scope
//! registered?" separately would let a scope granted under one registration be
//! exercised with the key of another.

use crate::error::Result;
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::store::Store;
use crate::types::Value;

/// The instant a signature was recorded, as the store observed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Witness {
    /// The transaction that recorded the signature. `None` means "as of the
    /// latest transaction": the write gate, verifying a signature that is being
    /// recorded right now.
    pub tx: Option<i64>,
    /// The valid-time instant the registration must cover.
    pub at: String,
}

impl Witness {
    /// Verify against the registry as it stands now. This is for a signature
    /// being recorded in this very write.
    #[must_use]
    pub fn now() -> Self {
        Self {
            tx: None,
            at: crate::time::now_iso(),
        }
    }

    /// When the store first recorded `value` as `entity`'s `predicate`: that
    /// transaction and its timestamp. `None` if the fact was never recorded.
    pub fn of_fact(
        store: &Store,
        entity: &str,
        predicate: &str,
        value: &str,
    ) -> Result<Option<Self>> {
        let (Some(e), Some(a)) = (store.lookup(entity)?, store.lookup(predicate)?) else {
            return Ok(None);
        };
        let mut stmt = store.prepare(
            "SELECT f.tx, t.timestamp FROM facts f JOIN transactions t ON t.id = f.tx \
             WHERE f.e = ?1 AND f.a = ?2 AND f.v = ?3 AND f.op = 1 \
             ORDER BY f.tx LIMIT 1",
        )?;
        let bytes = Value::Str(value.to_string()).to_bytes();
        let mut rows = stmt.query(rusqlite::params![e, a, bytes])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(Self {
            tx: Some(row.get(0)?),
            at: row.get(1)?,
        }))
    }
}

/// Where registrations may live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only ROOT, the default graph the SPARQL-based verifiers always read.
    Root,
    /// Every graph. The transition gate reads this way, because the identity
    /// graph is a named graph of the operator's choosing.
    AllGraphs,
    /// Only HUMAN trust-root registrations: ROOT, and carrying an ASSERTED
    /// `aegis:trustTier "human"` in ROOT (aegis-kzt0ql.9.4). Human decisions
    /// verify only against these. Agent registrations stay writable by agents
    /// and can never verify a human decision. The marker is matched as an
    /// asserted ROOT fact in SQL, so neither inference nor a named graph can
    /// supply it, and adding or changing it passes the trust-root gate
    /// (`crate::governance::trust_root`).
    HumanTier,
}

/// The predicate and value that mark a human trust-root registration.
pub const TRUST_TIER: &str = "http://aegis.gastown.local/ontology/trustTier";
/// See [`TRUST_TIER`].
pub const HUMAN_TIER: &str = "human";

/// The hex public keys registered to `verifier`, in effect at `witness`, and,
/// when `attests` is given, authorized for it by the SAME registration. Every
/// registration fact must be in effect: its type, its verifier, its key, and
/// its scope.
pub fn registered_keys(
    store: &Store,
    verifier: &str,
    attests: Option<&str>,
    witness: &Witness,
    scope: Scope,
) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    for bytes in select(store, verifier, attests, witness, scope, true)? {
        // A key written as a plain or an `xsd:string`-typed literal is the
        // same key; anything else cannot be a hex public key.
        let (Value::Str(key) | Value::Typed { lexical: key, .. }) = Value::from_bytes(&bytes)?
        else {
            continue;
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    Ok(keys)
}

/// Whether `verifier` holds a registration in effect at `witness` that
/// authorizes `attests`. This is the authority half alone, and it does not
/// need a key: a registration can grant scope before any key is enrolled.
pub fn is_authorized(
    store: &Store,
    verifier: &str,
    attests: &str,
    witness: &Witness,
    scope: Scope,
) -> Result<bool> {
    Ok(!select(store, verifier, Some(attests), witness, scope, false)?.is_empty())
}

/// One row per matching registration: its `aegis:publicKey` value when
/// `with_key`, else the registration entity (as bytes, only counted).
fn select(
    store: &Store,
    verifier: &str,
    attests: Option<&str>,
    witness: &Witness,
    scope: Scope,
    with_key: bool,
) -> Result<Vec<Vec<u8>>> {
    let lookup = |name: &str| store.lookup(&format!("{DEFAULT_BASE_NS}{name}"));
    let (Some(rdf_type), Some(class), Some(verifier_attr)) = (
        store.lookup(RDF_TYPE)?,
        lookup("VerifierRegistration")?,
        lookup("verifier")?,
    ) else {
        // A term that was never interned means no registration exists yet.
        return Ok(Vec::new());
    };
    let key_attr = if with_key {
        match lookup("publicKey")? {
            Some(id) => Some(id),
            None => return Ok(Vec::new()),
        }
    } else {
        None
    };
    let attests_attr = match attests {
        Some(_) => match lookup("attests")? {
            Some(id) => Some(id),
            None => return Ok(Vec::new()),
        },
        None => None,
    };

    // ?1 = the instant, ?2 = the tx (NULL means latest), ?3/?4 = rdf:type and
    // the class, ?5/?6 = aegis:verifier and its value, ?7 = aegis:publicKey,
    // ?8/?9 = aegis:attests and its value.
    let in_effect = |alias: &str| {
        let graph = match scope {
            Scope::Root | Scope::HumanTier => format!(" AND {alias}.g = 0"),
            Scope::AllGraphs => String::new(),
        };
        format!(
            "{alias}.op = 1 AND {alias}.valid_from <= ?1 \
             AND ({alias}.valid_to IS NULL OR {alias}.valid_to > ?1) \
             AND (?2 IS NULL OR ({alias}.tx <= ?2 \
                  AND ({alias}.valid_to IS NULL OR {alias}.retracted_tx > ?2))){graph}"
        )
    };
    let mut sql = format!(
        "SELECT DISTINCT {out} FROM facts t \
         JOIN facts vr ON vr.e = t.e AND vr.a = ?5 AND vr.v = ?6 AND {vr}",
        out = if with_key {
            "pk.v"
        } else {
            "CAST(t.e AS BLOB)"
        },
        vr = in_effect("vr"),
    );
    if with_key {
        sql.push_str(&format!(
            " JOIN facts pk ON pk.e = t.e AND pk.a = ?7 AND {}",
            in_effect("pk")
        ));
    }
    if attests_attr.is_some() {
        sql.push_str(&format!(
            " JOIN facts sc ON sc.e = t.e AND sc.a = ?8 AND sc.v = ?9 AND {}",
            in_effect("sc")
        ));
    }
    let tier_attr = if scope == Scope::HumanTier {
        match store.lookup(TRUST_TIER)? {
            Some(id) => Some(id),
            // Never interned: no human registration can exist.
            None => return Ok(Vec::new()),
        }
    } else {
        None
    };
    if tier_attr.is_some() {
        sql.push_str(&format!(
            " JOIN facts tr ON tr.e = t.e AND tr.a = ?10 AND tr.v = ?11 AND {}",
            in_effect("tr")
        ));
    }
    sql.push_str(&format!(
        " WHERE t.a = ?3 AND t.v = ?4 AND {}",
        in_effect("t")
    ));

    let class_bytes = Value::Ref(class).to_bytes();
    let verifier_bytes = Value::Str(verifier.to_string()).to_bytes();
    let attests_bytes = attests.map(|p| Value::Str(p.to_string()).to_bytes());
    let tier_bytes = Value::Str(HUMAN_TIER.to_string()).to_bytes();
    let mut stmt = store.prepare(&sql)?;
    let mut params: Vec<(&str, &dyn rusqlite::ToSql)> = vec![
        ("?1", &witness.at),
        ("?2", &witness.tx),
        ("?3", &rdf_type),
        ("?4", &class_bytes),
        ("?5", &verifier_attr),
        ("?6", &verifier_bytes),
    ];
    if let Some(k) = &key_attr {
        params.push(("?7", k));
    }
    if let (Some(a), Some(b)) = (&attests_attr, &attests_bytes) {
        params.push(("?8", a));
        params.push(("?9", b));
    }
    if let Some(t) = &tier_attr {
        params.push(("?10", t));
        params.push(("?11", &tier_bytes));
    }
    let rows = stmt
        .query_map(params.as_slice(), |row| row.get(0))?
        .collect::<std::result::Result<Vec<Vec<u8>>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
#[path = "verifier_registry_tests.rs"]
mod tests;
