//! Verified, idempotent materialization of repository knowledge packs.

use std::path::Path;

use crate::error::{Error, Result};
use crate::pack::{read_manifest, verify};
use crate::store::Store;

/// What [`unpack`] materialized and installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnpackReport {
    /// `loaded` for new content, `unchanged` for an already-loaded hash.
    pub outcome: String,
    /// Destination graph IRI.
    pub graph: String,
    /// Imported fact count.
    pub facts: usize,
    /// Versioned shape definitions installed.
    pub shapes: usize,
    /// Versioned stored queries installed.
    pub queries: usize,
    /// Entity embeddings restored from the pack.
    pub vectors: usize,
    /// Repository commit represented by this pack.
    pub repository_sha: Option<String>,
    /// Consumer HEAD supplied by the loader.
    pub head_sha: Option<String>,
}

/// Expectations and checkout state supplied while loading a repository pack.
#[derive(Debug, Clone, Default)]
pub struct LoadOptions<'a> {
    /// Optional destination graph override.
    pub into: Option<&'a str>,
    /// Repository identity the caller expects the manifest to carry.
    pub expect_repository: Option<&'a str>,
    /// Current checkout commit, used to report the incremental ingest range.
    pub head_sha: Option<&'a str>,
}

/// Materialize a pack using the backward-compatible non-repository API.
pub fn unpack(
    pack_path: &str,
    destination: &str,
    into: Option<&str>,
    timestamp: &str,
) -> Result<UnpackReport> {
    unpack_verified(
        pack_path,
        destination,
        &LoadOptions {
            into,
            ..Default::default()
        },
        timestamp,
    )
}

/// Verify and incrementally materialize a pack. Repeated loads are no-ops.
pub fn unpack_verified(
    pack_path: &str,
    destination: &str,
    opts: &LoadOptions<'_>,
    timestamp: &str,
) -> Result<UnpackReport> {
    let manifest = read_manifest(pack_path)?;
    let (stored, recomputed, matches) = verify(pack_path)?;
    if !matches {
        return Err(Error::InvalidValue(format!(
            "pack: HASH MISMATCH: manifest {stored}, recomputed {recomputed}"
        )));
    }
    let provenance: serde_json::Value = serde_json::from_str(&manifest.producer)
        .map_err(|e| Error::InvalidValue(format!("pack: invalid producer manifest: {e}")))?;
    let repository = provenance.get("repository").and_then(|v| v.as_str());
    let repository_sha = provenance
        .get("repository_sha")
        .and_then(|v| v.as_str())
        .map(String::from);
    if let Some(expected) = opts.expect_repository
        && repository != Some(expected)
    {
        return Err(Error::InvalidValue(format!(
            "pack: repository mismatch: expected {expected:?}, manifest has {repository:?}"
        )));
    }
    let graph = opts.into.unwrap_or(&manifest.source_graph).to_string();
    let dest = Store::open(destination)?;
    dest.conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS pack_loads (
        content_hash TEXT PRIMARY KEY, repository TEXT, repository_sha TEXT, loaded_at TEXT NOT NULL
    );",
    )?;
    let loaded: bool = dest.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pack_loads WHERE content_hash=?1)",
        rusqlite::params![manifest.content_hash],
        |r| r.get(0),
    )?;
    drop(dest);
    if loaded {
        return Ok(UnpackReport {
            outcome: "unchanged".into(),
            graph,
            facts: 0,
            shapes: 0,
            queries: 0,
            vectors: 0,
            repository_sha,
            head_sha: opts.head_sha.map(String::from),
        });
    }

    let source = Store::open(pack_path)?;
    let shapes = source.list_shapes()?;
    let queries = source.query_list()?;
    drop(source);
    let imported =
        crate::store::import::import_graph(Path::new(destination), Path::new(pack_path), &graph)?;
    let dest = Store::open(destination)?;
    for (name, turtle, _) in &shapes {
        dest.load_shapes(name, turtle, timestamp)?;
    }
    for query in &queries {
        dest.query_load(query, timestamp)?;
    }
    dest.conn.execute(
        "INSERT INTO pack_loads (content_hash, repository, repository_sha, loaded_at) VALUES (?1,?2,?3,?4)",
        rusqlite::params![manifest.content_hash, repository, repository_sha, timestamp],
    )?;
    Ok(UnpackReport {
        outcome: "loaded".into(),
        graph,
        facts: imported.facts,
        shapes: shapes.len(),
        queries: queries.len(),
        vectors: imported.vectors,
        repository_sha,
        head_sha: opts.head_sha.map(String::from),
    })
}
