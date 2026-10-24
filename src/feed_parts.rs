//! Oversized-transaction parts: the one chunking scheme for feed frames.
//!
//! The change stream splits a transaction too large for one frame into
//! numbered parts (local-sync-mode.md §6, F6), and the as-of share sends a cut
//! the same way. Both build their parts here, so a follower checks one digest
//! scheme whichever surface the parts came from.
//!
//! Parts are built one record at a time and handed out as each one closes, so
//! a caller can write them to disk as they are made: memory holds one part,
//! never the whole transaction or cut.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Transport frame ceiling. A frame's data must fit with room for the SSE
/// `id:`/`event:` lines around it.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Whether a serialized frame body of `len` bytes fits the frame budget.
#[must_use]
pub fn fits_frame_len(len: usize) -> bool {
    len <= MAX_FRAME_BYTES.saturating_sub(64)
}

/// One record is larger than a frame can carry, so it cannot be split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordTooLarge;

/// The totals a `begin` frame and a `commit` barrier repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartsSummary {
    /// Number of parts.
    pub parts: usize,
    /// Number of records across every part.
    pub records: usize,
    /// Total bytes of every part's data.
    pub bytes: usize,
    /// SHA-256 over the concatenated data of every part, in order, exactly as
    /// sent.
    pub digest: String,
}

/// Builds the parts of one transaction (or one cut) record by record.
///
/// Each part is `{"source_tx", "index", "digest_before", "records"}`, where
/// `digest_before` is the running digest over the parts before it, so a
/// consumer can check the stream as it arrives without a part hashing itself.
pub struct PartBuilder {
    source_tx: i64,
    max_len: usize,
    hasher: Sha256,
    /// The waiting records, each already serialized as compact JSON.
    chunk: Vec<String>,
    size: usize,
    index: usize,
    records: usize,
    bytes: usize,
}

impl PartBuilder {
    /// A builder for the parts of `source_tx`, at the transport frame budget.
    #[must_use]
    pub fn new(source_tx: i64) -> Self {
        Self::with_max_len(source_tx, MAX_FRAME_BYTES.saturating_sub(64))
    }

    /// A builder whose parts are at most `max_len` bytes. Tests use a small
    /// budget to exercise splitting without megabyte fixtures.
    #[must_use]
    pub fn with_max_len(source_tx: i64, max_len: usize) -> Self {
        let mut builder = Self {
            source_tx,
            max_len,
            hasher: Sha256::new(),
            chunk: Vec::new(),
            size: 0,
            index: 0,
            records: 0,
            bytes: 0,
        };
        builder.size = builder.empty_len();
        builder
    }

    /// The part's JSON text, byte for byte what `serde_json` writes for
    /// `{"source_tx", "index", "digest_before", "records"}`: keys sorted, compact,
    /// each record exactly as given. `before` is lowercase hex and needs no
    /// escaping. Writing it directly spares re-serializing every record.
    fn part_data(&self, chunk: &[String], before: &str) -> String {
        let records: usize = chunk.iter().map(|r| r.len() + 1).sum();
        let mut out = String::with_capacity(records + 128);
        out.push_str("{\"digest_before\":\"");
        out.push_str(before);
        out.push_str("\",\"index\":");
        out.push_str(&self.index.to_string());
        out.push_str(",\"records\":[");
        for (i, record) in chunk.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(record);
        }
        out.push_str("],\"source_tx\":");
        out.push_str(&self.source_tx.to_string());
        out.push('}');
        out
    }

    /// The empty part, sized with a placeholder digest of the final length,
    /// so the real part can never be larger than the one that was measured.
    fn empty_len(&self) -> usize {
        self.part_data(&[], &"0".repeat(64)).len()
    }

    /// Add one record. Returns the part this record closed, if the record did
    /// not fit beside the records already waiting.
    ///
    /// # Errors
    ///
    /// [`RecordTooLarge`] when the record alone exceeds a part.
    pub fn push(&mut self, record: &Value) -> Result<Option<String>, RecordTooLarge> {
        self.push_line(record.to_string())
    }

    /// [`Self::push`] for a record already serialized as compact JSON, so a
    /// caller that also needs the line (for a digest) serializes it once.
    ///
    /// # Errors
    ///
    /// [`RecordTooLarge`] when the record alone exceeds a part.
    pub fn push_line(&mut self, record: String) -> Result<Option<String>, RecordTooLarge> {
        // Sizes add exactly: an array serializes as `[`, the items joined by
        // `,`, and `]`, so a part is its empty frame plus each record plus a
        // comma.
        let text = record.len();
        let mut closed = None;
        if !self.chunk.is_empty() && self.size + text + 1 > self.max_len {
            closed = Some(self.close());
        }
        let len = text + usize::from(!self.chunk.is_empty());
        if self.size + len > self.max_len {
            return Err(RecordTooLarge);
        }
        self.size += len;
        self.records += 1;
        self.chunk.push(record);
        Ok(closed)
    }

    fn close(&mut self) -> String {
        let before = hex::encode(self.hasher.clone().finalize());
        let chunk = std::mem::take(&mut self.chunk);
        let data = self.part_data(&chunk, &before);
        self.hasher.update(data.as_bytes());
        self.bytes += data.len();
        self.index += 1;
        self.size = self.empty_len();
        data
    }

    /// Close the last part and return it with the totals.
    #[must_use]
    pub fn finish(mut self) -> (Option<String>, PartsSummary) {
        let last = (!self.chunk.is_empty()).then(|| self.close());
        let summary = PartsSummary {
            parts: self.index,
            records: self.records,
            bytes: self.bytes,
            digest: hex::encode(self.hasher.finalize()),
        };
        (last, summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The former construction: parts built from `Value`s by `serde_json`.
    fn reference_parts(source_tx: i64, max_len: usize, records: &[Value]) -> Vec<String> {
        let part = |index: usize, chunk: &[Value], before: &str| {
            json!({"source_tx": source_tx, "index": index, "digest_before": before, "records": chunk})
                .to_string()
        };
        let empty = |index: usize| part(index, &[], &"0".repeat(64)).len();
        let (mut chunks, mut chunk): (Vec<Vec<Value>>, Vec<Value>) = (Vec::new(), Vec::new());
        let mut size = empty(0);
        for record in records {
            let text = record.to_string().len();
            if !chunk.is_empty() && size + text + 1 > max_len {
                chunks.push(std::mem::take(&mut chunk));
                size = empty(chunks.len());
            }
            size += text + usize::from(!chunk.is_empty());
            chunk.push(record.clone());
        }
        if !chunk.is_empty() {
            chunks.push(chunk);
        }
        let mut hasher = Sha256::new();
        chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                let before = hex::encode(hasher.clone().finalize());
                let data = part(index, chunk, &before);
                hasher.update(data.as_bytes());
                data
            })
            .collect()
    }

    /// Writing the part text directly is byte-identical to `serde_json`'s
    /// output, for escapes, Unicode, nesting and every split point.
    #[test]
    fn direct_parts_equal_the_serde_json_parts() {
        let records: Vec<Value> = (0..40)
            .map(|i| {
                json!({
                    "entity": format!("urn:e:{i}"),
                    "value": {"base64": "AJ/u", "bytes_len": 3},
                    "note": format!("quote \" backslash \\ tab \t é ✓ {i}"),
                    "valid_to": null,
                    "tx": i,
                })
            })
            .collect();
        for max_len in [300, 500, 1_000, 10_000] {
            let mut builder = PartBuilder::with_max_len(7, max_len);
            let mut parts = Vec::new();
            for record in &records {
                parts.extend(builder.push(record).unwrap());
            }
            let (last, summary) = builder.finish();
            parts.extend(last);
            let want = reference_parts(7, max_len, &records);
            assert_eq!(parts, want, "max_len {max_len}");
            assert_eq!(summary.parts, want.len());
        }
    }
}
