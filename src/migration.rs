//! Migration from SQLite vectors to LanceDB.
//!
//! Reads all rows from the SQLite `vectors` table, converts them to Arrow
//! `RecordBatch`es, and bulk-inserts into a LanceDB table. Supports dry-run
//! mode for previewing migration counts without writing.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use lancedb::query::ExecutableQuery;
use rusqlite::params;

use crate::error::{Error, Result};
use crate::store::Store;
use crate::vector::bytes_to_f32_slice;

/// Embedding dimension (must match `LanceVectorStore::EMBEDDING_DIM`).
const EMBEDDING_DIM: i32 = 384;

/// Default batch size for bulk inserts.
const DEFAULT_BATCH_SIZE: usize = 1000;

/// LanceDB table name (must match `LanceVectorStore::TABLE_NAME`).
const TABLE_NAME: &str = "vectors";

/// Result of a migration run.
#[derive(Debug)]
pub struct MigrateResult {
    /// Number of rows successfully migrated.
    pub migrated: usize,
    /// Number of rows skipped (dimension mismatch, etc.).
    pub skipped: usize,
}

/// Migrate all vectors from the SQLite store to a LanceDB database.
///
/// # Arguments
/// * `store` - The SQLite-backed store to read vectors from.
/// * `lance_path` - URI/path for the LanceDB database directory.
/// * `dry_run` - If true, report counts without writing to LanceDB.
/// * `batch_size` - Number of rows per insert batch (0 = default 1000).
pub fn migrate_sqlite_to_lancedb(
    store: &Store,
    lance_path: &str,
    dry_run: bool,
    batch_size: usize,
) -> Result<MigrateResult> {
    let (rows, skipped) = read_sqlite_vectors(store)?;

    if dry_run {
        return Ok(MigrateResult {
            migrated: rows.len(),
            skipped,
        });
    }

    if rows.is_empty() {
        // Create an empty table so subsequent open() calls find it.
        create_empty_table(lance_path)?;
        return Ok(MigrateResult {
            migrated: 0,
            skipped,
        });
    }

    let batch_sz = if batch_size == 0 {
        DEFAULT_BATCH_SIZE
    } else {
        batch_size
    };

    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| Error::Store("No Tokio runtime available for migration".into()))?;

    let migrated = tokio::task::block_in_place(|| {
        handle.block_on(async {
            let conn = lancedb::connect(lance_path)
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB connect: {e}")))?;

            // Check if table already exists.
            let tables = conn
                .table_names()
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB list tables: {e}")))?;
            if tables.contains(&TABLE_NAME.to_string()) {
                return Err(Error::Store(format!(
                    "LanceDB table '{TABLE_NAME}' already exists at {lance_path}. \
                     Remove the directory to re-migrate."
                )));
            }

            let mut table: Option<lancedb::Table> = None;
            let mut total = 0;

            for chunk in rows.chunks(batch_sz) {
                let batch = build_batch(chunk)?;
                match &table {
                    Some(t) => {
                        t.add(vec![batch])
                            .execute()
                            .await
                            .map_err(|e| Error::Store(format!("LanceDB add: {e}")))?;
                    }
                    None => {
                        let t = conn
                            .create_table(TABLE_NAME, vec![batch])
                            .execute()
                            .await
                            .map_err(|e| Error::Store(format!("LanceDB create table: {e}")))?;
                        table = Some(t);
                    }
                }
                total += chunk.len();
            }

            Ok(total)
        })
    })?;

    Ok(MigrateResult { migrated, skipped })
}

/// Create an empty LanceDB table with the vectors schema.
fn create_empty_table(lance_path: &str) -> Result<()> {
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| Error::Store("No Tokio runtime available".into()))?;

    tokio::task::block_in_place(|| {
        handle.block_on(async {
            let conn = lancedb::connect(lance_path)
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB connect: {e}")))?;

            let tables = conn
                .table_names()
                .execute()
                .await
                .map_err(|e| Error::Store(format!("LanceDB list tables: {e}")))?;
            if !tables.contains(&TABLE_NAME.to_string()) {
                conn.create_empty_table(TABLE_NAME, lance_schema())
                    .execute()
                    .await
                    .map_err(|e| Error::Store(format!("LanceDB create empty table: {e}")))?;
            }
            Ok(())
        })
    })
}

/// A single row read from the SQLite vectors table.
struct VectorRow {
    entity_id: i64,
    text: String,
    embedding: Vec<f32>,
    valid_from: String,
    valid_to: Option<String>,
}

/// Read all vectors from the SQLite store.
///
/// Returns `(valid_rows, skipped_count)`. Rows with dimension != `EMBEDDING_DIM`
/// are skipped.
fn read_sqlite_vectors(store: &Store) -> Result<(Vec<VectorRow>, usize)> {
    let mut stmt = store
        .conn
        .prepare("SELECT entity_id, text, embedding, valid_from, valid_to FROM vectors")?;
    let mut rows = Vec::new();
    let mut skipped = 0;
    let mut result = stmt.query(params![])?;

    while let Some(row) = result.next()? {
        let blob: Vec<u8> = row.get(2)?;
        let embedding = bytes_to_f32_slice(&blob);

        if embedding.len() != EMBEDDING_DIM as usize {
            skipped += 1;
            continue;
        }

        rows.push(VectorRow {
            entity_id: row.get(0)?,
            text: row.get(1)?,
            embedding,
            valid_from: row.get(3)?,
            valid_to: row.get(4)?,
        });
    }

    Ok((rows, skipped))
}

/// Build a multi-row Arrow `RecordBatch` from a slice of vector rows.
fn build_batch(rows: &[VectorRow]) -> Result<RecordBatch> {
    let n = rows.len();

    let entity_ids = Int64Array::from(rows.iter().map(|r| r.entity_id).collect::<Vec<_>>());
    let texts = StringArray::from(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>());
    let valid_froms = StringArray::from(
        rows.iter()
            .map(|r| r.valid_from.as_str())
            .collect::<Vec<_>>(),
    );
    let valid_tos = StringArray::from(
        rows.iter()
            .map(|r| r.valid_to.as_deref())
            .collect::<Vec<Option<&str>>>(),
    );
    let entity_types = StringArray::from(vec![None::<&str>; n]);
    let source_episodes = StringArray::from(vec![None::<&str>; n]);

    // Flatten all embeddings into a single contiguous array for FixedSizeList.
    let all_values: Vec<f32> = rows
        .iter()
        .flat_map(|r| r.embedding.iter().copied())
        .collect();
    let values = Float32Array::from(all_values);
    let vector = FixedSizeListArray::try_new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        EMBEDDING_DIM,
        Arc::new(values) as ArrayRef,
        None,
    )
    .map_err(|e| Error::Store(format!("Arrow FixedSizeList: {e}")))?;

    RecordBatch::try_new(
        lance_schema(),
        vec![
            Arc::new(entity_ids) as ArrayRef,
            Arc::new(texts),
            Arc::new(vector),
            Arc::new(valid_froms),
            Arc::new(valid_tos),
            Arc::new(entity_types),
            Arc::new(source_episodes),
        ],
    )
    .map_err(|e| Error::Store(format!("Arrow RecordBatch: {e}")))
}

/// Arrow schema for the LanceDB vectors table (matches `LanceVectorStore::schema()`).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_empty_store() {
        let store = Store::open_in_memory().unwrap();
        let (rows, skipped) = read_sqlite_vectors(&store).unwrap();
        assert!(rows.is_empty());
        assert_eq!(skipped, 0);
    }

    #[test]
    fn read_with_dimension_mismatch() {
        let store = Store::open_in_memory().unwrap();

        // Insert a vector with wrong dimension (8 instead of 384).
        let emb: Vec<f32> = (0..8).map(|i| i as f32 * 0.1).collect();
        let blob = crate::vector::f32_slice_to_bytes(&emb);
        store
            .conn
            .execute(
                "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
                 VALUES (1, 'test', ?1, '2026-01-01')",
                params![blob],
            )
            .unwrap();

        let (rows, skipped) = read_sqlite_vectors(&store).unwrap();
        assert!(rows.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn read_matching_vectors() {
        let store = Store::open_in_memory().unwrap();

        // Insert a vector with correct dimension (384).
        let emb: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin()).collect();
        let blob = crate::vector::f32_slice_to_bytes(&emb);
        store
            .conn
            .execute(
                "INSERT INTO vectors (entity_id, text, embedding, valid_from, valid_to) \
                 VALUES (1, 'alice', ?1, '2026-01-01', NULL)",
                params![blob],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO vectors (entity_id, text, embedding, valid_from, valid_to) \
                 VALUES (2, 'bob', ?1, '2026-01-01', '2026-03-01')",
                params![blob],
            )
            .unwrap();

        let (rows, skipped) = read_sqlite_vectors(&store).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(skipped, 0);
        assert_eq!(rows[0].entity_id, 1);
        assert_eq!(rows[0].text, "alice");
        assert!(rows[0].valid_to.is_none());
        assert_eq!(rows[1].entity_id, 2);
        assert_eq!(rows[1].valid_to.as_deref(), Some("2026-03-01"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn migrate_dry_run() {
        let store = Store::open_in_memory().unwrap();

        let emb: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin()).collect();
        let blob = crate::vector::f32_slice_to_bytes(&emb);
        store
            .conn
            .execute(
                "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
                 VALUES (1, 'test', ?1, '2026-01-01')",
                params![blob],
            )
            .unwrap();

        let result = migrate_sqlite_to_lancedb(&store, "memory://test", true, 0).unwrap();
        assert_eq!(result.migrated, 1);
        assert_eq!(result.skipped, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn migrate_to_lancedb() {
        let store = Store::open_in_memory().unwrap();

        // Insert two vectors.
        let emb: Vec<f32> = (0..384).map(|i| (i as f32 * 0.01).sin()).collect();
        let blob = crate::vector::f32_slice_to_bytes(&emb);
        store
            .conn
            .execute(
                "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
                 VALUES (1, 'alice', ?1, '2026-01-01')",
                params![blob],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO vectors (entity_id, text, embedding, valid_from) \
                 VALUES (2, 'bob', ?1, '2026-02-01')",
                params![blob],
            )
            .unwrap();

        let result = migrate_sqlite_to_lancedb(&store, "memory://migrate-test", false, 1).unwrap();
        assert_eq!(result.migrated, 2);
        assert_eq!(result.skipped, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn migrate_empty_creates_table() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let lance_path = dir.path().join("vectors-db");
        let lance_uri = lance_path.to_string_lossy().to_string();

        let result = migrate_sqlite_to_lancedb(&store, &lance_uri, false, 0).unwrap();
        assert_eq!(result.migrated, 0);

        // Verify table was created on disk.
        let conn = lancedb::connect(&lance_uri).execute().await.unwrap();
        let tables = conn.table_names().execute().await.unwrap();
        assert!(tables.contains(&"vectors".to_string()));
    }
}
