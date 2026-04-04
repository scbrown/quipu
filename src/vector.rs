//! Vector search over entity embeddings, stored in `SQLite`.
//!
//! Embeddings are stored as f32 blobs in a `vectors` table alongside temporal
//! metadata. Search uses brute-force cosine similarity — fast enough for the
//! <1M fact target. The embedding function is caller-provided (trait), so
//! Bobbin can supply its ONNX pipeline when Quipu is used as a subsystem.

use rusqlite::params;

use crate::error::Result;
use crate::store::Store;

/// Schema for the vectors table, created alongside the fact log.
pub(crate) const VECTORS_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS vectors (
    entity_id  INTEGER NOT NULL,
    text       TEXT    NOT NULL,
    embedding  BLOB    NOT NULL,
    valid_from TEXT    NOT NULL,
    valid_to   TEXT,
    PRIMARY KEY (entity_id, valid_from)
);
CREATE INDEX IF NOT EXISTS idx_vectors_valid ON vectors(valid_to);
"#;

/// A vector search result.
#[derive(Debug, Clone)]
pub struct VectorMatch {
    pub entity_id: i64,
    pub text: String,
    pub score: f64,
    pub valid_from: String,
    pub valid_to: Option<String>,
}

impl Store {
    /// Store an embedding for an entity.
    pub fn embed_entity(
        &self,
        entity_id: i64,
        text: &str,
        embedding: &[f32],
        valid_from: &str,
    ) -> Result<()> {
        let blob = f32_slice_to_bytes(embedding);
        self.conn.execute(
            "INSERT OR REPLACE INTO vectors (entity_id, text, embedding, valid_from, valid_to) \
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![entity_id, text, blob, valid_from],
        )?;
        Ok(())
    }

    /// Close an entity's embedding (set `valid_to`) when the entity is retracted.
    pub fn close_embedding(&self, entity_id: i64, valid_to: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE vectors SET valid_to = ?1 WHERE entity_id = ?2 AND valid_to IS NULL",
            params![valid_to, entity_id],
        )?;
        Ok(())
    }

    /// Search for similar entities by cosine similarity.
    ///
    /// Only returns current embeddings (`valid_to` IS NULL) unless `valid_at` is set.
    pub fn vector_search(
        &self,
        query_embedding: &[f32],
        limit: usize,
        valid_at: Option<&str>,
    ) -> Result<Vec<VectorMatch>> {
        let sql = if valid_at.is_some() {
            "SELECT entity_id, text, embedding, valid_from, valid_to FROM vectors \
             WHERE valid_from <= ?1 AND (valid_to IS NULL OR valid_to > ?1)"
        } else {
            "SELECT entity_id, text, embedding, valid_from, valid_to FROM vectors \
             WHERE valid_to IS NULL"
        };

        let mut stmt = self.conn.prepare(sql)?;
        let mut matches = Vec::new();

        let rows = if let Some(vt) = valid_at {
            stmt.query(params![vt])?
        } else {
            stmt.query([])?
        };

        let mut rows = rows;
        while let Some(row) = rows.next()? {
            let entity_id: i64 = row.get(0)?;
            let text: String = row.get(1)?;
            let blob: Vec<u8> = row.get(2)?;
            let valid_from: String = row.get(3)?;
            let valid_to: Option<String> = row.get(4)?;

            let stored = bytes_to_f32_slice(&blob);
            let score = cosine_similarity(query_embedding, &stored);

            matches.push(VectorMatch {
                entity_id,
                text,
                score,
                valid_from,
                valid_to,
            });
        }

        // Sort by score descending, take top N.
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(limit);
        Ok(matches)
    }

    /// Return the number of current embeddings.
    pub fn vector_count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM vectors WHERE valid_to IS NULL",
            [],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(0))
    }
}

// ── Embedding math ─────────────────────────────────────────────────

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 { 0.0 } else { dot / denom }
}

fn f32_slice_to_bytes(data: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(data.len() * 4);
    for f in data {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    bytes
}

fn bytes_to_f32_slice(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_embedding(seed: f32, dim: usize) -> Vec<f32> {
        (0..dim).map(|i| (seed + i as f32 * 0.1).sin()).collect()
    }

    #[test]
    fn embed_and_search() {
        let store = Store::open_in_memory().unwrap();

        let e1 = store.intern("http://example.org/alice").unwrap();
        let e2 = store.intern("http://example.org/bob").unwrap();
        let e3 = store.intern("http://example.org/carol").unwrap();

        let emb1 = make_embedding(1.0, 8);
        let emb2 = make_embedding(1.1, 8); // similar to emb1
        let emb3 = make_embedding(5.0, 8); // different

        store
            .embed_entity(e1, "Alice the engineer", &emb1, "2026-01-01")
            .unwrap();
        store
            .embed_entity(e2, "Bob the developer", &emb2, "2026-01-01")
            .unwrap();
        store
            .embed_entity(e3, "Carol the manager", &emb3, "2026-01-01")
            .unwrap();

        assert_eq!(store.vector_count().unwrap(), 3);

        // Search with emb1 — Alice should be top match, Bob close second.
        let results = store.vector_search(&emb1, 3, None).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].entity_id, e1); // Alice = exact match
        assert_eq!(results[1].entity_id, e2); // Bob = similar
        assert!(results[0].score > results[1].score);
        assert!(results[1].score > results[2].score);
    }

    #[test]
    fn temporal_vector_search() {
        let store = Store::open_in_memory().unwrap();

        let e1 = store.intern("http://example.org/svc").unwrap();
        let emb_old = make_embedding(1.0, 8);
        let emb_new = make_embedding(2.0, 8);

        // Old embedding, valid until March.
        store
            .embed_entity(e1, "old description", &emb_old, "2026-01-01")
            .unwrap();
        store.close_embedding(e1, "2026-03-01").unwrap();

        // New embedding, current.
        store
            .embed_entity(e1, "new description", &emb_new, "2026-03-01")
            .unwrap();

        // Current search: only new embedding.
        let results = store.vector_search(&emb_old, 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "new description");

        // Time-travel search to February: only old embedding.
        let results = store
            .vector_search(&emb_old, 10, Some("2026-02-01"))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "old description");

        // Time-travel to April: only new embedding.
        let results = store
            .vector_search(&emb_new, 10, Some("2026-04-01"))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "new description");
    }

    #[test]
    fn cosine_similarity_self() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-10);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-10);
    }

    #[test]
    fn embedding_round_trip() {
        let original: Vec<f32> = vec![1.0, -2.5, 3.25, 0.0, f32::MAX, f32::MIN];
        let bytes = f32_slice_to_bytes(&original);
        let decoded = bytes_to_f32_slice(&bytes);
        assert_eq!(original, decoded);
    }

    #[test]
    fn limit_results() {
        let store = Store::open_in_memory().unwrap();

        for i in 0..20 {
            let eid = store.intern(&format!("http://example.org/e{i}")).unwrap();
            let emb = make_embedding(i as f32, 8);
            store
                .embed_entity(eid, &format!("entity {i}"), &emb, "2026-01-01")
                .unwrap();
        }

        let query = make_embedding(0.0, 8);
        let results = store.vector_search(&query, 5, None).unwrap();
        assert_eq!(results.len(), 5);
    }
}
