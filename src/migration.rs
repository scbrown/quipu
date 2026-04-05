//! Migrate vector data between backends (SQLite → LanceDB).
//!
//! The migration reads all rows from the SQLite `vectors` table and bulk-inserts
//! them into a `LanceDB` store in batches. Temporal metadata (`valid_from`,
//! `valid_to`) is preserved.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use rusqlite::params;

use crate::error::{Error, Result};
use crate::vector::bytes_to_f32_slice;
use crate::vector_lance::LanceVectorStore;

/// Embedding dimension (must match `LanceVectorStore::EMBEDDING_DIM`).
const EMBEDDING_DIM: i32 = 384;

/// Number of rows per bulk-insert batch.
const BATCH_SIZE: usize = 1000;

/// Result of a migration run.
#[derive(Debug)]
pub struct MigrationReport {
    /// Total rows read from the source.
    pub total: usize,
    /// Rows successfully migrated.
    pub migrated: usize,
    /// Rows skipped (e.g. wrong embedding dimension).
    pub skipped: usize,
}

/// Arrow schema matching `LanceVectorStore::schema()`.
fn lance_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("entity_id", DataType::Int64, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                EMBEDDING_DIM,
            ),
            false,
        ),
        Field::new("valid_from", DataType::Utf8, false),
        Field::new("valid_to", DataType::Utf8, true),
        Field::new("entity_type", DataType::Utf8, true),
        Field::new("source_episode", DataType::Utf8, true),
    ]))
}

/// A row read from the SQLite vectors table.
struct VectorRow {
    entity_id: i64,
    text: String,
    embedding: Vec<f32>,
    valid_from: String,
    valid_to: Option<String>,
}

/// Read all vector rows from the SQLite database at `db_path`.
fn read_sqlite_vectors(db_path: &str) -> Result<Vec<VectorRow>> {
    let conn = rusqlite::Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT entity_id, text, embedding, valid_from, valid_to FROM vectors ORDER BY entity_id, valid_from",
    )?;
    let mut rows_out = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let entity_id: i64 = row.get(0)?;
        let text: String = row.get(1)?;
        let blob: Vec<u8> = row.get(2)?;
        let valid_from: String = row.get(3)?;
        let valid_to: Option<String> = row.get(4)?;
        let embedding = bytes_to_f32_slice(&blob);
        rows_out.push(VectorRow {
            entity_id,
            text,
            embedding,
            valid_from,
            valid_to,
        });
    }
    Ok(rows_out)
}

/// Build a `RecordBatch` from a slice of `VectorRow`.
fn rows_to_batch(rows: &[VectorRow]) -> Result<RecordBatch> {
    let n = rows.len();
    let mut ids = Vec::with_capacity(n);
    let mut texts = Vec::with_capacity(n);
    let mut all_floats = Vec::with_capacity(n * EMBEDDING_DIM as usize);
    let mut valid_froms = Vec::with_capacity(n);
    let mut valid_tos: Vec<Option<&str>> = Vec::with_capacity(n);
    let entity_types: Vec<Option<&str>> = vec![None; n];
    let source_episodes: Vec<Option<&str>> = vec![None; n];

    for row in rows {
        ids.push(row.entity_id);
        texts.push(row.text.as_str());
        all_floats.extend_from_slice(&row.embedding);
        valid_froms.push(row.valid_from.as_str());
        valid_tos.push(row.valid_to.as_deref());
    }

    let id_arr = Int64Array::from(ids);
    let text_arr = StringArray::from(texts);
    let vf_arr = StringArray::from(valid_froms);
    let vt_arr = StringArray::from(valid_tos);
    let et_arr = StringArray::from(entity_types);
    let se_arr = StringArray::from(source_episodes);
    let values = Float32Array::from(all_floats);
    let vector_arr = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        EMBEDDING_DIM,
        Arc::new(values) as ArrayRef,
        None,
    )
    .map_err(|e| Error::Store(format!("Arrow FixedSizeList: {e}")))?;

    RecordBatch::try_new(
        lance_schema(),
        vec![
            Arc::new(id_arr) as ArrayRef,
            Arc::new(text_arr),
            Arc::new(vector_arr),
            Arc::new(vf_arr),
            Arc::new(vt_arr),
            Arc::new(et_arr),
            Arc::new(se_arr),
        ],
    )
    .map_err(|e| Error::Store(format!("Arrow RecordBatch: {e}")))
}

/// Migrate all vectors from the SQLite database at `db_path` into the given
/// `LanceDB` store.
///
/// Returns a report of migrated/skipped counts. If `dry_run` is true, reads
/// and validates but does not write.
pub fn migrate_vectors(
    db_path: &str,
    lance: &mut LanceVectorStore,
    dry_run: bool,
) -> Result<MigrationReport> {
    let rows = read_sqlite_vectors(db_path)?;
    let total = rows.len();

    // Partition into valid (correct dim) and skipped.
    let expected_dim = EMBEDDING_DIM as usize;
    let (valid, skipped): (Vec<_>, Vec<_>) = rows
        .into_iter()
        .partition(|r| r.embedding.len() == expected_dim);
    let skipped_count = skipped.len();

    if dry_run {
        return Ok(MigrationReport {
            total,
            migrated: valid.len(),
            skipped: skipped_count,
        });
    }

    if valid.is_empty() {
        return Ok(MigrationReport {
            total,
            migrated: 0,
            skipped: skipped_count,
        });
    }

    // Bulk insert in batches. First batch creates the table if needed.
    let mut migrated = 0;
    let mut first = true;
    for chunk in valid.chunks(BATCH_SIZE) {
        let batch = rows_to_batch(chunk)?;
        if first {
            lance.ensure_table_from_batch(batch)?;
            first = false;
        } else {
            lance.add_record_batch(batch)?;
        }
        migrated += chunk.len();
    }

    Ok(MigrationReport {
        total,
        migrated,
        skipped: skipped_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use crate::vector::KnowledgeVectorStore;

    fn make_embedding(seed: f32) -> Vec<f32> {
        (0..384).map(|i| (seed + i as f32 * 0.01).sin()).collect()
    }

    #[test]
    fn read_sqlite_vectors_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db_str = db_path.to_str().unwrap();

        // Populate a SQLite store with vectors.
        let store = Store::open(db_str).unwrap();
        let emb1 = make_embedding(1.0);
        let emb2 = make_embedding(2.0);
        store.embed_entity(1, "alice", &emb1, "2026-01-01").unwrap();
        store.embed_entity(2, "bob", &emb2, "2026-01-01").unwrap();
        store.close_embedding(1, "2026-03-01").unwrap();
        drop(store);

        let rows = read_sqlite_vectors(db_str).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].entity_id, 1);
        assert_eq!(rows[0].text, "alice");
        assert_eq!(rows[0].embedding.len(), 384);
        assert_eq!(rows[0].valid_to, Some("2026-03-01".to_string()));
        assert_eq!(rows[1].entity_id, 2);
        assert!(rows[1].valid_to.is_none());
    }

    #[tokio::test]
    async fn migrate_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db_str = db_path.to_str().unwrap();

        let store = Store::open(db_str).unwrap();
        for i in 0..5 {
            let emb = make_embedding(i as f32);
            store
                .embed_entity(i, &format!("entity {i}"), &emb, "2026-01-01")
                .unwrap();
        }
        drop(store);

        let mut lance = LanceVectorStore::open_in_memory().await.unwrap();
        let report = migrate_vectors(db_str, &mut lance, true).unwrap();
        assert_eq!(report.total, 5);
        assert_eq!(report.migrated, 5);
        assert_eq!(report.skipped, 0);

        // Dry run should not have written anything.
        assert_eq!(lance.vector_count().unwrap(), 0);
    }

    #[tokio::test]
    async fn migrate_full() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db_str = db_path.to_str().unwrap();

        let store = Store::open(db_str).unwrap();
        let emb1 = make_embedding(1.0);
        let emb2 = make_embedding(2.0);
        store.embed_entity(1, "alice", &emb1, "2026-01-01").unwrap();
        store.embed_entity(2, "bob", &emb2, "2026-01-01").unwrap();
        // Close alice's embedding to test temporal preservation.
        store.close_embedding(1, "2026-03-01").unwrap();
        drop(store);

        let mut lance = LanceVectorStore::open_in_memory().await.unwrap();
        let report = migrate_vectors(db_str, &mut lance, false).unwrap();
        assert_eq!(report.total, 2);
        assert_eq!(report.migrated, 2);
        assert_eq!(report.skipped, 0);

        // Both vectors should be in LanceDB. Only bob is current.
        // (alice has valid_to set so she's closed)
        let count = lance.vector_count().unwrap();
        assert_eq!(count, 1); // Only current (valid_to IS NULL) vectors.
    }
}
