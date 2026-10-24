//! The applier's transaction digest and its retained `AppliedTx` rows
//! (`docs/design/shared-applier-api.md`: "Transaction digest", rule 6).
//!
//! A redelivered transaction is a duplicate only when its CONTENT equals the
//! one applied, so the applier needs the identity of what it applied. The
//! digest is computed from the decoded records, never from a digest a frame
//! declares, and it does not depend on framing: one page or a chunked
//! begin/part/commit of the same transaction hash the same.

use rusqlite::{OptionalExtension, params};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

use super::{Outcome, Source};
use crate::error::Result;
use crate::store::Store;

const DOMAIN: &[u8] = b"quipu-applier-tx/v1";

pub(super) const APPLIED_TX_DDL: &str = "CREATE TABLE IF NOT EXISTS replica_applied_tx (\
     store_id TEXT NOT NULL, log_epoch TEXT NOT NULL, source_tx INTEGER NOT NULL, \
     tx_digest TEXT NOT NULL, PRIMARY KEY (store_id, log_epoch, source_tx))";

/// Append one length-prefixed field, so no two field sequences encode alike.
fn put(out: &mut Vec<u8>, field: &[u8]) {
    out.extend_from_slice(&u64::try_from(field.len()).unwrap_or(u64::MAX).to_be_bytes());
    out.extend_from_slice(field);
}

/// The digest of one source transaction's records, or why a record cannot be
/// encoded exactly (the same records the applier refuses to write).
pub(super) fn tx_digest(
    source: &Source,
    tx: i64,
    records: &[&Json],
) -> std::result::Result<String, String> {
    let timestamp = records
        .first()
        .and_then(|r| r["timestamp"].as_str())
        .unwrap_or_default();
    let mut encoded = Vec::with_capacity(records.len());
    for record in records {
        encoded.push(encode_record(record, timestamp)?);
    }
    // Record order is not part of a transaction's identity.
    encoded.sort();
    let mut out = Vec::new();
    put(&mut out, DOMAIN);
    put(&mut out, source.store_id.as_bytes());
    put(&mut out, source.log_epoch.as_bytes());
    put(&mut out, &tx.to_be_bytes());
    for record in &encoded {
        put(&mut out, record);
    }
    Ok(hex::encode(Sha256::digest(&out)))
}

/// One record as the fields the applier writes, with the same defaults
/// (`valid_from` falls back to the transaction time; a retract carries no
/// `valid_to`).
fn encode_record(record: &Json, tx_timestamp: &str) -> std::result::Result<Vec<u8>, String> {
    let (Some(entity), Some(attribute), Some(op)) = (
        record["entity"].as_str(),
        record["attribute"].as_str(),
        record["op"].as_str(),
    ) else {
        return Err("record lacks entity, attribute or op".into());
    };
    let retract = op == "retract";
    let value = encode_value(&record[if retract { "old_value" } else { "value" }])?;
    let mut out = Vec::new();
    put(
        &mut out,
        record["graph"].as_str().unwrap_or("ROOT").as_bytes(),
    );
    put(&mut out, entity.as_bytes());
    put(&mut out, attribute.as_bytes());
    put(&mut out, &value);
    put(
        &mut out,
        record["valid_from"]
            .as_str()
            .unwrap_or(tx_timestamp)
            .as_bytes(),
    );
    match record["valid_to"].as_str().filter(|_| !retract) {
        Some(to) => put(&mut out, &[b"\x01", to.as_bytes()].concat()),
        None => put(&mut out, b"\x00"),
    }
    put(&mut out, if retract { b"retract" } else { b"assert" });
    Ok(out)
}

/// A value's exact bytes and kind, decoded the way `value_of` decodes it.
fn encode_value(json: &Json) -> std::result::Result<Vec<u8>, String> {
    use base64::Engine as _;
    let str_of = |o: &serde_json::Map<String, Json>, k: &str| {
        o.get(k)
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut out = Vec::new();
    match json {
        Json::String(s) => {
            put(&mut out, b"str");
            put(&mut out, s.as_bytes());
        }
        Json::Bool(b) => {
            put(&mut out, b"bool");
            put(&mut out, &[u8::from(*b)]);
        }
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => {
                put(&mut out, b"int");
                put(&mut out, &i.to_be_bytes());
            }
            (None, Some(f)) => {
                put(&mut out, b"float");
                put(&mut out, &f.to_bits().to_be_bytes());
            }
            (None, None) => return Err(format!("unrepresentable number {n}")),
        },
        Json::Object(o) if o.get("ref").and_then(Json::as_str).is_some() => {
            put(&mut out, b"ref");
            put(&mut out, str_of(o, "ref").as_bytes());
        }
        Json::Object(o) if o.contains_key("lang") => {
            put(&mut out, b"lang");
            put(&mut out, str_of(o, "value").as_bytes());
            put(&mut out, str_of(o, "lang").as_bytes());
        }
        Json::Object(o) if o.contains_key("datatype") => {
            put(&mut out, b"typed");
            put(&mut out, str_of(o, "value").as_bytes());
            put(&mut out, str_of(o, "datatype").as_bytes());
        }
        Json::Object(o) if o.get("base64").and_then(Json::as_str).is_some() => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(str_of(o, "base64"))
                .map_err(|_| "byte value is not valid base64".to_string())?;
            put(&mut out, b"bytes");
            put(&mut out, &bytes);
        }
        other => return Err(format!("value {other} has no exact encoding")),
    }
    Ok(out)
}

/// Keep the digest of a transaction just applied (inside its savepoint).
pub(super) fn retain(store: &Store, source: &Source, tx: i64, digest: &str) -> Result<()> {
    store.conn.execute(
        "INSERT INTO replica_applied_tx (store_id, log_epoch, source_tx, tx_digest) \
         VALUES (?1, ?2, ?3, ?4)",
        params![source.store_id, source.log_epoch, tx, digest],
    )?;
    Ok(())
}

/// The digest retained for an applied transaction, if one was kept.
pub(super) fn retained(store: &Store, source: &Source, tx: i64) -> Result<Option<String>> {
    Ok(store
        .conn
        .query_row(
            "SELECT tx_digest FROM replica_applied_tx \
             WHERE store_id = ?1 AND log_epoch = ?2 AND source_tx = ?3",
            params![source.store_id, source.log_epoch, tx],
            |r| r.get(0),
        )
        .optional()?)
}

/// Why a replica stopped.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stop {
    /// The redelivered transaction's content differs from the applied one.
    DigestMismatch {
        /// The source transaction.
        source_tx: i64,
        /// The digest retained when it was applied.
        applied: String,
        /// The digest of the redelivery (or why it has no exact encoding).
        received: String,
    },
    /// No digest was retained for the redelivered transaction.
    DigestUnverifiable {
        /// The source transaction.
        source_tx: i64,
    },
}

/// `None` when a redelivered transaction equals the one applied; otherwise
/// the [`Outcome::Stopped`] that halts the replica.
pub(super) fn check_duplicate(
    store: &Store,
    source: &Source,
    tx: i64,
    records: &[&Json],
) -> Result<Option<Outcome>> {
    let received = tx_digest(source, tx, records)
        .unwrap_or_else(|reason| format!("no exact encoding: {reason}"));
    let reason = match retained(store, source, tx)? {
        Some(applied) if applied == received => return Ok(None),
        Some(applied) => Stop::DigestMismatch {
            source_tx: tx,
            applied,
            received,
        },
        None => Stop::DigestUnverifiable { source_tx: tx },
    };
    Ok(Some(Outcome::Stopped { reason }))
}

#[cfg(test)]
#[path = "applier_digest_tests.rs"]
mod tests;
