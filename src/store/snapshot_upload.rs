//! Bounded, resumable staging for atomic `/knot` snapshot replacement.

use rusqlite::{OptionalExtension, params};
use serde_json::Value as JsonValue;

use super::Store;
use crate::error::{Error, Result};

/// Maximum bytes accepted by one staging request.
pub const MAX_PART_BYTES: usize = 512 * 1024;
/// Maximum aggregate snapshot size accepted by one upload.
pub const MAX_UPLOAD_BYTES: usize = 128 * 1024 * 1024;
/// Maximum numbered parts in one upload.
pub const MAX_PARTS: usize = 512;
const UPLOAD_TTL_SECS: u64 = 60 * 60;

fn required_str<'a>(input: &'a JsonValue, key: &str) -> Result<&'a str> {
    input
        .get(key)
        .and_then(JsonValue::as_str)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| Error::InvalidValue(format!("missing non-empty '{key}' parameter")))
}

fn required_usize(input: &JsonValue, key: &str) -> Result<usize> {
    input
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| Error::InvalidValue(format!("missing valid '{key}' parameter")))
}

fn validate_hash(value: &str, key: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(Error::InvalidValue(format!(
            "'{key}' must use sha256:<hex>"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::InvalidValue(format!(
            "'{key}' must contain 64 hex digits"
        )));
    }
    Ok(())
}

/// Stable identity for a producer snapshot and its exact content.
#[must_use]
pub fn snapshot_upload_id(snapshot: &str, content_hash: &str) -> String {
    crate::share::sha256(format!("{snapshot}\n{content_hash}").as_bytes())
}

fn cleanup_expired(store: &Store, now: u64) -> Result<usize> {
    Ok(store.conn.execute(
        "DELETE FROM snapshot_uploads WHERE expires_at <= ?1",
        params![i64::try_from(now).unwrap_or(i64::MAX)],
    )?)
}

/// Stage one immutable numbered part. Re-sending the same bytes is a no-op;
/// changing any manifest field or a previously-seen part fails closed.
pub fn stage_snapshot_part(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let upload_id = required_str(input, "upload_id")?;
    let snapshot = required_str(input, "snapshot")?;
    let content_hash = required_str(input, "content_hash")?;
    let part_hash = required_str(input, "part_hash")?;
    let payload = required_str(input, "payload")?.as_bytes();
    let total_parts = required_usize(input, "total_parts")?;
    let total_bytes = required_usize(input, "total_bytes")?;
    let part_number = required_usize(input, "part_number")?;
    validate_hash(content_hash, "content_hash")?;
    validate_hash(part_hash, "part_hash")?;
    if upload_id != snapshot_upload_id(snapshot, content_hash) {
        return Err(Error::InvalidValue(
            "upload_id does not match sha256(snapshot + newline + content_hash)".into(),
        ));
    }
    if total_parts == 0 || total_parts > MAX_PARTS || part_number >= total_parts {
        return Err(Error::InvalidValue(format!(
            "part bounds invalid: part_number={part_number}, total_parts={total_parts}, max={MAX_PARTS}"
        )));
    }
    if payload.is_empty() || payload.len() > MAX_PART_BYTES {
        return Err(Error::InvalidValue(format!(
            "part payload must be 1..={MAX_PART_BYTES} bytes"
        )));
    }
    if total_bytes == 0 || total_bytes > MAX_UPLOAD_BYTES || payload.len() > total_bytes {
        return Err(Error::InvalidValue(format!(
            "total_bytes must be 1..={MAX_UPLOAD_BYTES} and cover this part"
        )));
    }
    if crate::share::sha256(payload) != part_hash {
        return Err(Error::InvalidValue(
            "part_hash does not match payload bytes".into(),
        ));
    }

    let now = crate::time::epoch_secs();
    cleanup_expired(store, now)?;
    let timestamp = input
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let actor = input.get("actor").and_then(JsonValue::as_str);
    let source = input.get("source").and_then(JsonValue::as_str);
    let graph = input.get("graph").and_then(JsonValue::as_str);
    let tx = store.conn.transaction()?;
    tx.execute(
        "INSERT OR IGNORE INTO snapshot_uploads
         (upload_id,snapshot,content_hash,total_parts,total_bytes,timestamp,actor,source,graph,created_at,expires_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![upload_id, snapshot, content_hash, total_parts as i64, total_bytes as i64,
            timestamp, actor, source, graph, now as i64, now.saturating_add(UPLOAD_TTL_SECS) as i64],
    )?;
    type Manifest = (
        String,
        String,
        i64,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let manifest: Option<Manifest> = tx
        .query_row(
            "SELECT snapshot,content_hash,total_parts,total_bytes,timestamp,actor,source,graph
         FROM snapshot_uploads WHERE upload_id=?1",
            params![upload_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()?;
    let expected = (
        snapshot.to_string(),
        content_hash.to_string(),
        total_parts as i64,
        total_bytes as i64,
        timestamp.to_string(),
        actor.map(str::to_string),
        source.map(str::to_string),
        graph.map(str::to_string),
    );
    if manifest.as_ref() != Some(&expected) {
        return Err(Error::InvalidValue(
            "upload_id already exists with a different immutable manifest".into(),
        ));
    }
    let existing: Option<(String, Vec<u8>)> = tx.query_row(
        "SELECT part_hash,payload FROM snapshot_upload_parts WHERE upload_id=?1 AND part_number=?2",
        params![upload_id, part_number as i64], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let idempotent =
        if let Some((old_hash, old_payload)) = existing {
            if old_hash != part_hash || old_payload != payload {
                return Err(Error::InvalidValue(
                    "part_number already exists with different bytes".into(),
                ));
            }
            true
        } else {
            tx.execute(
            "INSERT INTO snapshot_upload_parts (upload_id,part_number,part_hash,byte_count,payload)
             VALUES (?1,?2,?3,?4,?5)",
            params![upload_id, part_number as i64, part_hash, payload.len() as i64, payload],
        )?;
            false
        };
    let received: i64 = tx.query_row(
        "SELECT COUNT(*) FROM snapshot_upload_parts WHERE upload_id=?1",
        params![upload_id],
        |row| row.get(0),
    )?;
    tx.commit()?;
    Ok(serde_json::json!({
        "upload_id": upload_id, "part_number": part_number, "received_parts": received,
        "total_parts": total_parts, "idempotent": idempotent,
        "expires_in_seconds": UPLOAD_TTL_SECS,
    }))
}

/// Verify every staged part and atomically replace the active snapshot. Any
/// missing/corrupt byte returns before [`crate::mcp::knot::tool_knot`] runs.
pub fn promote_snapshot_upload(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let upload_id = required_str(input, "upload_id")?;
    cleanup_expired(store, crate::time::epoch_secs())?;
    type Manifest = (
        String,
        String,
        i64,
        i64,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let manifest: Option<Manifest> = store
        .conn
        .query_row(
            "SELECT snapshot,content_hash,total_parts,total_bytes,timestamp,actor,source,graph
         FROM snapshot_uploads WHERE upload_id=?1",
            params![upload_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()?;
    let Some((
        snapshot,
        content_hash,
        total_parts_raw,
        total_bytes_raw,
        timestamp,
        actor,
        source,
        graph,
    )) = manifest
    else {
        return Err(Error::InvalidValue("unknown or expired upload_id".into()));
    };
    let total_parts = usize::try_from(total_parts_raw)
        .map_err(|_| Error::InvalidValue("stored total_parts is out of range".into()))?;
    let total_bytes = usize::try_from(total_bytes_raw)
        .map_err(|_| Error::InvalidValue("stored total_bytes is out of range".into()))?;
    let mut statement = store.conn.prepare(
        "SELECT part_number,part_hash,payload FROM snapshot_upload_parts
         WHERE upload_id=?1 ORDER BY part_number",
    )?;
    let rows = statement.query_map(params![upload_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    let mut payload = Vec::with_capacity(total_bytes);
    let mut seen = 0_usize;
    for row in rows {
        let (part_number_raw, part_hash, bytes) = row?;
        let part_number = usize::try_from(part_number_raw)
            .map_err(|_| Error::InvalidValue("stored part_number is out of range".into()))?;
        if part_number != seen || crate::share::sha256(&bytes) != part_hash {
            return Err(Error::InvalidValue(format!(
                "missing or corrupt part {seen}"
            )));
        }
        payload.extend_from_slice(&bytes);
        seen += 1;
    }
    drop(statement);
    if seen != total_parts || payload.len() != total_bytes {
        return Err(Error::InvalidValue(format!(
            "upload incomplete: received {seen}/{total_parts} parts and {}/{total_bytes} bytes",
            payload.len()
        )));
    }
    if crate::share::sha256(&payload) != content_hash {
        return Err(Error::InvalidValue(
            "content_hash does not match assembled payload".into(),
        ));
    }
    let turtle = String::from_utf8(payload)
        .map_err(|_| Error::InvalidValue("assembled snapshot is not UTF-8 Turtle".into()))?;
    let mut knot = serde_json::json!({
        "turtle": turtle,
        "timestamp": if timestamp.is_empty() { crate::time::now_iso() } else { timestamp },
        "actor": actor, "source": source, "graph": graph,
        "replace_snapshot": true, "snapshot": snapshot,
    });
    knot.as_object_mut()
        .expect("JSON object")
        .retain(|_, value| !value.is_null());
    let mut result = crate::mcp::knot::tool_knot(store, &knot)?;
    if result.get("conforms").and_then(JsonValue::as_bool) == Some(false) {
        return Ok(result);
    }
    // Keep the content-addressed stage until its TTL. If the response is lost,
    // the producer can repeat promote safely: tool_knot's snapshot replacement
    // is itself idempotent, while deleting here would turn an indeterminate
    // successful promotion into an unrecoverable "unknown upload" response.
    let object = result.as_object_mut().expect("knot returns object");
    object.insert("upload_id".into(), JsonValue::String(upload_id.to_string()));
    object.insert("content_hash".into(), JsonValue::String(content_hash));
    object.insert("parts".into(), serde_json::json!(total_parts));
    object.insert("total_bytes".into(), serde_json::json!(total_bytes));
    object.insert("promoted".into(), JsonValue::Bool(true));
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(snapshot: &str, turtle: &str, split: usize) -> Vec<JsonValue> {
        let bytes = turtle.as_bytes();
        let pieces: Vec<&[u8]> = bytes.chunks(split).collect();
        let content_hash = crate::share::sha256(bytes);
        pieces.iter().enumerate().map(|(part, bytes)| {
            let payload = std::str::from_utf8(bytes).unwrap();
            serde_json::json!({
                "upload_id": snapshot_upload_id(snapshot, &content_hash), "snapshot": snapshot,
                "content_hash": content_hash, "total_parts": pieces.len(), "total_bytes": turtle.len(),
                "part_number": part, "part_hash": crate::share::sha256(payload.as_bytes()),
                "payload": payload, "timestamp": "2026-09-02T00:00:00Z", "actor": "bobbin",
            })
        }).collect()
    }

    #[test]
    fn interrupted_upload_resumes_idempotently_then_promotes() {
        let mut store = Store::open_in_memory().unwrap();
        let parts = inputs(
            "bobbin-chunks:r",
            "<http://ex/a> <http://www.w3.org/2000/01/rdf-schema#label> \"A\" .\n",
            40,
        );
        assert!(
            !stage_snapshot_part(&mut store, &parts[0]).unwrap()["idempotent"]
                .as_bool()
                .unwrap()
        );
        assert!(
            stage_snapshot_part(&mut store, &parts[0]).unwrap()["idempotent"]
                .as_bool()
                .unwrap()
        );
        let err = promote_snapshot_upload(
            &mut store,
            &serde_json::json!({"upload_id": parts[0]["upload_id"]}),
        )
        .unwrap_err();
        assert!(err.to_string().contains("incomplete"));
        for part in parts.iter().skip(1) {
            stage_snapshot_part(&mut store, part).unwrap();
        }
        let result = promote_snapshot_upload(
            &mut store,
            &serde_json::json!({"upload_id": parts[0]["upload_id"]}),
        )
        .unwrap();
        assert_eq!(result["promoted"], true);
        assert_eq!(result["content_hash"], parts[0]["content_hash"]);
        let repeated = promote_snapshot_upload(
            &mut store,
            &serde_json::json!({"upload_id": parts[0]["upload_id"]}),
        )
        .unwrap();
        assert_eq!(repeated["promoted"], true);
        assert_eq!(repeated["content_hash"], parts[0]["content_hash"]);
    }

    #[test]
    fn conflicting_retry_fails_without_changing_active_snapshot() {
        let mut store = Store::open_in_memory().unwrap();
        crate::mcp::knot::tool_knot(
            &mut store,
            &serde_json::json!({
                "turtle": "<http://ex/old> <http://www.w3.org/2000/01/rdf-schema#label> \"old\" .",
                "replace_snapshot": true, "snapshot": "bobbin-chunks:r"
            }),
        )
        .unwrap();
        let parts = inputs(
            "bobbin-chunks:r",
            "<http://ex/new> <http://www.w3.org/2000/01/rdf-schema#label> \"new\" .\n",
            100,
        );
        stage_snapshot_part(&mut store, &parts[0]).unwrap();
        let mut bad = parts[0].clone();
        bad["payload"] = JsonValue::String("different".into());
        bad["part_hash"] = JsonValue::String(crate::share::sha256(b"different"));
        assert!(
            stage_snapshot_part(&mut store, &bad)
                .unwrap_err()
                .to_string()
                .contains("different bytes")
        );
        let old = store.lookup("http://ex/old").unwrap().unwrap();
        assert!(!store.entity_facts(old).unwrap().is_empty());
    }
}
