//! Score only bounded SQLite candidates before top-K.
use super::structured_candidates::CAP;
use crate::vector::{VectorMatch, bytes_to_f32_slice, cosine_similarity};
use crate::{Error, Result, Store};
use std::collections::{HashMap, HashSet};
fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidValue(message.into())
}
pub(super) fn score_candidates(
    store: &Store,
    candidates: &HashSet<String>,
    query: &[f32],
    at: Option<&str>,
    limit: usize,
) -> Result<Vec<VectorMatch>> {
    let _progress = crate::time::request_deadline()
        .map(|d| crate::sparql::ProgressGuard::install(&store.conn, d))
        .transpose()?;
    let mut stmt = store.prepare("SELECT text,embedding,valid_from,valid_to FROM vectors WHERE entity_id=?1 AND ((?2 IS NULL AND valid_to IS NULL) OR (?2 IS NOT NULL AND valid_from<=?2 AND (valid_to IS NULL OR valid_to>?2)))")?;
    let mut best: HashMap<i64, VectorMatch> = HashMap::new();
    let mut total = 0;
    for entity in candidates {
        let Some(id) = store.lookup(entity)? else {
            continue;
        };
        let mut rows = stmt.query(rusqlite::params![id, at])?;
        while let Some(row) = rows.next()? {
            total += 1;
            if total > CAP * 8 {
                return Err(invalid(
                    "structured vector row cap exceeded; narrow the expression",
                ));
            }
            let blob: Vec<u8> = row.get(1)?;
            let vector = bytes_to_f32_slice(&blob);
            if !blob.len().is_multiple_of(4) || vector.iter().any(|n| !n.is_finite()) {
                return Err(invalid("invalid stored vector in structured candidate"));
            }
            if vector.len() != query.len() {
                return Err(invalid(
                    "embedding dimension mismatch in structured candidate",
                ));
            }
            let matched = VectorMatch {
                entity_id: id,
                text: row.get(0)?,
                score: cosine_similarity(query, &vector),
                valid_from: row.get(2)?,
                valid_to: row.get(3)?,
            };
            if best.get(&id).is_none_or(|old| matched.score > old.score) {
                best.insert(id, matched);
            }
        }
    }
    let mut out: Vec<_> = best.into_values().collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.entity_id.cmp(&b.entity_id))
    });
    out.truncate(limit);
    Ok(out)
}
