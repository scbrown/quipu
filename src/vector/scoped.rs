//! Bounded SQLite scoring of an explicit entity set, before top-K selection.
use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, params, params_from_iter, types::Value};

use super::{VectorMatch, bytes_to_f32_slice, cosine_similarity};
use crate::{Error, Result, Store};

const MAX_SCOPED_ROWS: usize = 1000;

impl Store {
    pub(crate) fn vector_search_scoped_sqlite(
        &self,
        query: &[f32],
        entities: &[i64],
        limit: usize,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        let started = crate::time::Stopwatch::start();
        let deadline = crate::time::request_deadline();
        let check = || {
            if deadline.is_some_and(|dl| dl.passed()) {
                Err(Error::QueryTimeout {
                    elapsed_ms: started.elapsed_ms(),
                    limit_ms: deadline
                        .map(|dl| dl.millis_from(&started))
                        .unwrap_or_default(),
                })
            } else {
                Ok(())
            }
        };
        check()?;
        let _progress = deadline
            .map(|dl| crate::sparql::ProgressGuard::install(&self.conn, dl))
            .transpose()?;
        if query.is_empty() || query.len() > 16384 || query.iter().any(|v| !v.is_finite()) {
            return Err(Error::InvalidValue(
                "scoped query must have 1..16384 finite dimensions".into(),
            ));
        }
        let entities: BTreeSet<_> = entities.iter().copied().collect();
        if entities.len() > MAX_SCOPED_ROWS {
            return Err(Error::InvalidValue(
                "narrow the SPARQL scope: more than 1000 entities".into(),
            ));
        }
        if !self.has_sqlite_vector_backend() {
            return Err(Error::InvalidValue(
                "exact SQLite scoring requires the SQLite vector backend".into(),
            ));
        }
        let entities: Vec<_> = entities.into_iter().collect();
        let mut scored = Vec::new();
        // The primary key starts with entity_id; no global vector scan or pool
        // widening. Keep each chunk below SQLite's conservative bind limit.
        for chunk in entities.chunks(100) {
            check()?;
            let placeholders = vec!["?"; chunk.len()].join(",");
            let validity = if valid_at.is_some() {
                "valid_from <= ? AND (valid_to IS NULL OR valid_to > ?)"
            } else {
                "valid_to IS NULL"
            };
            let sql = format!(
                "SELECT entity_id, embedding, valid_from FROM vectors \
                               WHERE entity_id IN ({placeholders}) AND {validity} LIMIT 1001"
            );
            let mut values: Vec<Value> = chunk.iter().copied().map(Value::Integer).collect();
            if let Some(at) = valid_at {
                values.extend([Value::Text(at.into()), Value::Text(at.into())]);
            }
            let mut statement = self.conn.prepare(&sql)?;
            let mut rows = statement.query(params_from_iter(values))?;
            while let Some(row) = rows.next()? {
                check()?;
                if scored.len() >= MAX_SCOPED_ROWS {
                    return Err(Error::InvalidValue(
                        "narrow the SPARQL scope: more than 1000 embedding rows".into(),
                    ));
                }
                let entity_id: i64 = row.get(0)?;
                let bytes = row
                    .get_ref(1)?
                    .as_blob()
                    .map_err(|e| Error::Store(format!("invalid scoped vector blob: {e}")))?;
                if bytes.len() != query.len() * 4 {
                    return Err(Error::Store(
                        "malformed or mismatched scoped embedding bytes".into(),
                    ));
                }
                let blob = bytes.to_vec();
                let embedding = bytes_to_f32_slice(&blob);
                if !embedding.is_empty() && embedding.len() != query.len() {
                    return Err(Error::Store(
                        "embedding dimension mismatch in scoped search".into(),
                    ));
                }
                if embedding.iter().any(|v| !v.is_finite()) {
                    return Err(Error::Store("non-finite scoped embedding".into()));
                }
                let since: String = row.get(2)?;
                scored.push((entity_id, since, cosine_similarity(query, &embedding)));
            }
        }
        check()?;
        scored.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        let mut matches = Vec::with_capacity(scored.len());
        for (entity_id, since, score) in scored {
            check()?;
            // Fetch the exact vector version scored, not an arbitrary current
            // text belonging to the same entity. A concurrently removed row is
            // skipped; SQL errors remain errors rather than a false empty read.
            let sql = if valid_at.is_some() {
                "SELECT text, valid_to FROM vectors WHERE entity_id=?1 AND valid_from=?2 \
                 AND valid_from<=?3 AND (valid_to IS NULL OR valid_to>?3)"
            } else {
                "SELECT text, valid_to FROM vectors WHERE entity_id=?1 AND valid_from=?2 AND valid_to IS NULL"
            };
            let mut statement = self.conn.prepare(sql)?;
            let row = if let Some(at) = valid_at {
                statement
                    .query_row(params![entity_id, since, at], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()?
            } else {
                statement
                    .query_row(params![entity_id, since], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?
            };
            if let Some((text, valid_to)) = row {
                matches.push(VectorMatch {
                    entity_id,
                    text,
                    score,
                    valid_from: since,
                    valid_to,
                });
            }
        }
        check()?;
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vector::KnowledgeVectorStore;

    #[test]
    fn scoped_search_preserves_exact_temporal_text() {
        let store = Store::open_in_memory().unwrap();
        let id = store.intern("http://example.org/temporal").unwrap();
        store
            .embed_entity(id, "old", &[1.0, 0.0], "2026-01-01")
            .unwrap();
        store.close_embedding(id, "2026-02-01").unwrap();
        store
            .embed_entity(id, "new", &[0.0, 1.0], "2026-02-01")
            .unwrap();
        let historical = store
            .vector_search_scoped_sqlite(&[1.0, 0.0], &[id], 1, Some("2026-01-15"))
            .unwrap();
        assert_eq!(historical[0].text, "old");
        assert_eq!(historical[0].valid_to.as_deref(), Some("2026-02-01"));
        let current = store
            .vector_search_scoped_sqlite(&[1.0, 0.0], &[id], 1, None)
            .unwrap();
        assert_eq!(current[0].text, "new");
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0, 0.0], &[id], 1, Some("2025-01-01"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn scoped_search_refuses_nonfinite_bytes_and_expired_budget() {
        let store = Store::open_in_memory().unwrap();
        let id = store.intern("http://example.org/bad-vector").unwrap();
        store
            .embed_entity(id, "bad", &[f32::NAN], "2026-01-01")
            .unwrap();
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0], &[id], 1, None)
                .is_err()
        );
        assert!(
            store
                .vector_search_scoped_sqlite(&[f32::INFINITY], &[id], 1, None)
                .is_err()
        );
        store
            .conn
            .execute("UPDATE vectors SET embedding=x'010203'", [])
            .unwrap();
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0], &[id], 1, None)
                .is_err()
        );
        let _guard =
            crate::time::set_request_deadline(Some(crate::time::Deadline::after_millis(0)));
        assert!(matches!(
            store.vector_search_scoped_sqlite(&[1.0], &[id], 1, None),
            Err(Error::QueryTimeout { .. })
        ));
    }

    #[test]
    fn scoped_search_embedding_row_cap_is_enforced() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .intern("http://example.org/multiple-versions")
            .unwrap();
        for version in 0..1000 {
            store
                .embed_entity(id, "positive", &[1.0], &format!("version-{version:04}"))
                .unwrap();
        }
        assert_eq!(
            store
                .vector_search_scoped_sqlite(&[1.0], &[id], 1, None)
                .unwrap()
                .len(),
            1
        );
        store
            .embed_entity(id, "overflow", &[1.0], "version-1000")
            .unwrap();
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0], &[id], 1, None)
                .is_err()
        );
    }

    #[test]
    fn scoped_search_bounds_and_empty_controls() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0], &[], 1, None)
                .unwrap()
                .is_empty()
        );
        let ids: Vec<i64> = (1..=1000).collect();
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0], &ids, 1, None)
                .unwrap()
                .is_empty()
        );
        let too_many: Vec<i64> = (1..=1001).collect();
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0], &too_many, 1, None)
                .is_err()
        );
        let id = store.intern("http://example.org/control").unwrap();
        store
            .embed_entity(id, "positive", &[1.0], "2026-01-01")
            .unwrap();
        assert_eq!(
            store
                .vector_search_scoped_sqlite(&[1.0], &[id, id], 1, None)
                .unwrap()
                .len(),
            1
        );
        assert!(
            store
                .vector_search_scoped_sqlite(&[1.0, 0.0], &[id], 1, None)
                .is_err()
        );
    }
}
