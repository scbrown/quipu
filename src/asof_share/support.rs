//! The as-of cut's supporting pieces: the record form, the read-phase
//! ceilings, disk accounting and the per-graph digest.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{Duration, Instant};

use rusqlite::params;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{AbortReason, CutError, Limits, invalid};
use crate::{Result, Store};

/// One cut record in the `/changes` assert form. Fields are declared in
/// alphabetical order, so `serde_json` writes the same canonical line as a
/// `Value` with sorted keys, without building a map per record.
#[derive(Serialize)]
pub(crate) struct AssertRecord<'a> {
    pub(crate) actor: &'a Option<String>,
    pub(crate) attribute: &'a str,
    pub(crate) entity: &'a str,
    pub(crate) graph: &'a str,
    pub(crate) op: &'static str,
    pub(crate) source: &'a Option<String>,
    pub(crate) timestamp: &'a Option<String>,
    pub(crate) tx: i64,
    pub(crate) valid_from: &'a str,
    pub(crate) valid_to: &'a Option<String>,
    pub(crate) value: &'a serde_json::Value,
}

/// A transaction's `timestamp`, `actor` and `source`.
pub(super) type TxMeta = (Option<String>, Option<String>, Option<String>);

/// VM steps between two ceiling checks inside SQLite.
pub(super) const PROGRESS_STRIDE: i32 = 1000;

/// The read phase's ceilings, installed as the connection's progress handler
/// for as long as the cut reads, and removed on drop.
pub(crate) struct ReadPhaseGuard<'s> {
    store: &'s Store,
}

impl<'s> ReadPhaseGuard<'s> {
    pub(crate) fn install(
        store: &'s Store,
        started: Instant,
        limits: &Limits,
        wal: Option<PathBuf>,
        wal_start: u64,
        interrupted: Arc<AtomicU8>,
    ) -> Result<Self> {
        let (timeout, wal_max) = (limits.read_timeout, limits.wal_growth_max_bytes);
        // The WAL is a file stat: at most once a second, not every stride.
        let mut wal_checked = started;
        store.conn.progress_handler(
            PROGRESS_STRIDE,
            Some(move || {
                if started.elapsed() > timeout {
                    interrupted.store(1, Ordering::Release);
                    return true;
                }
                if wal_checked.elapsed() >= Duration::from_secs(1) {
                    wal_checked = Instant::now();
                    if wal_len(wal.as_deref()).saturating_sub(wal_start) > wal_max {
                        interrupted.store(2, Ordering::Release);
                        return true;
                    }
                }
                false
            }),
        )?;
        Ok(Self { store })
    }
}

impl Drop for ReadPhaseGuard<'_> {
    fn drop(&mut self) {
        let _ = self.store.conn.progress_handler(0, None::<fn() -> bool>);
    }
}

/// A frozen graph's mounted pack, as the registry records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FrozenPack {
    pub(super) alias: String,
    pub(super) path: String,
    pub(super) content_hash: String,
    pub(super) frozen_at: String,
}

/// The mounted (not thawed) pack of frozen graph `iri`, if it is frozen. A
/// store that never froze anything has no registry table.
pub(super) fn frozen_pack(store: &Store, iri: &str) -> Result<Option<FrozenPack>> {
    use rusqlite::OptionalExtension as _;
    let registry: bool = store.conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM main.sqlite_master \
                        WHERE type = 'table' AND name = 'frozen_packs')",
        [],
        |r| r.get(0),
    )?;
    if !registry {
        return Ok(None);
    }
    Ok(store
        .conn
        .query_row(
            "SELECT alias, path, content_hash, frozen_at FROM main.frozen_packs \
             WHERE graph_iri = ?1 AND thawed_at IS NULL ORDER BY id DESC LIMIT 1",
            params![iri],
            |r| {
                Ok(FrozenPack {
                    alias: r.get(0)?,
                    path: r.get(1)?,
                    content_hash: r.get(2)?,
                    frozen_at: r.get(3)?,
                })
            },
        )
        .optional()?)
}

pub(super) fn sorted_unique(
    iris: &[String],
    field: &str,
) -> std::result::Result<Vec<String>, CutError> {
    let set: BTreeSet<&String> = iris.iter().collect();
    if set.len() != iris.len() {
        return Err(invalid(format!("duplicate graph in {field}")));
    }
    Ok(set.into_iter().cloned().collect())
}

pub(super) fn check_ceilings(
    started: Instant,
    limits: &Limits,
    wal: Option<&Path>,
    wal_start: u64,
) -> std::result::Result<(), CutError> {
    if started.elapsed() > limits.read_timeout {
        return Err(CutError::Aborted(AbortReason::TimeCeiling));
    }
    if wal_len(wal).saturating_sub(wal_start) > limits.wal_growth_max_bytes {
        return Err(CutError::Aborted(AbortReason::WalCeiling));
    }
    Ok(())
}

/// The main database's WAL file, or `None` for an in-memory store.
pub(super) fn wal_path(store: &Store) -> Result<Option<PathBuf>> {
    let file: String = store.conn.query_row(
        "SELECT file FROM pragma_database_list WHERE name = 'main'",
        [],
        |r| r.get(0),
    )?;
    Ok((!file.is_empty()).then(|| PathBuf::from(format!("{file}-wal"))))
}

pub(super) fn wal_len(wal: Option<&Path>) -> u64 {
    wal.and_then(|path| std::fs::metadata(path).ok())
        .map_or(0, |meta| meta.len())
}

/// Every byte under the spool directory: finished spools still being sent,
/// a partial spool and sorted digest runs.
pub(super) fn spool_usage(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            total += spool_usage(&entry.path())?;
        } else {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Free bytes on the filesystem holding `dir`, for an unprivileged writer.
#[cfg(unix)]
pub(super) fn free_bytes(dir: &Path) -> Option<u64> {
    let stat = rustix::fs::statvfs(dir).ok()?;
    Some(stat.f_bavail.saturating_mul(stat.f_frsize))
}

#[cfg(not(unix))]
pub(super) fn free_bytes(_dir: &Path) -> Option<u64> {
    None
}

/// The applier's per-graph share digest (`applier::share_graph_digest`):
/// SHA-256 over the graph's canonical record lines, sorted, joined by `\n`.
///
/// The applier sorts in memory. A producer cutting millions of records cannot,
/// so each graph's lines are held in a run. When all graphs' runs together
/// pass [`Limits::digest_run_bytes`], the largest is sorted and spilled to a
/// temporary directory beside the spool; at the end the runs are merged into
/// the hash. The bytes hashed are the same.
pub(super) struct GraphDigest {
    dir: tempfile::TempDir,
    run: Vec<String>,
    run_len: usize,
    runs: Vec<PathBuf>,
}

impl GraphDigest {
    pub(super) fn new(spool_dir: &Path) -> std::io::Result<Self> {
        Ok(Self {
            dir: tempfile::Builder::new()
                .prefix("digest-runs-")
                .tempdir_in(spool_dir)?,
            run: Vec::new(),
            run_len: 0,
            runs: Vec::new(),
        })
    }

    /// Hold one line in the current run; the caller decides when to spill.
    pub(super) fn push(&mut self, line: String) {
        self.run_len += line.len();
        self.run.push(line);
    }

    /// Bytes held in memory, not yet spilled.
    pub(super) fn pending(&self) -> usize {
        self.run_len
    }

    pub(super) fn spill(&mut self) -> std::io::Result<u64> {
        let bytes = (self.run_len + self.run.len()) as u64;
        self.run.sort_unstable();
        let path = self.dir.path().join(format!("run-{}", self.runs.len()));
        let mut file = std::io::BufWriter::new(std::fs::File::create(&path)?);
        for line in self.run.drain(..) {
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
        }
        file.flush()?;
        self.runs.push(path);
        self.run_len = 0;
        Ok(bytes)
    }

    /// Lowercase hex of the digest. The run directory is removed on drop.
    pub(super) fn finish(mut self) -> std::io::Result<String> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        use std::io::BufRead;
        let mut hasher = Sha256::new();
        let mut first = true;
        let mut emit = |line: &str| {
            if !first {
                hasher.update(b"\n");
            }
            first = false;
            hasher.update(line.as_bytes());
        };
        if self.runs.is_empty() {
            self.run.sort_unstable();
            for line in &self.run {
                emit(line);
            }
        } else {
            if !self.run.is_empty() {
                self.spill()?;
            }
            let mut readers = self
                .runs
                .iter()
                .map(|path| std::fs::File::open(path).map(|f| std::io::BufReader::new(f).lines()))
                .collect::<std::io::Result<Vec<_>>>()?;
            let mut heap = BinaryHeap::new();
            for (index, reader) in readers.iter_mut().enumerate() {
                if let Some(line) = reader.next().transpose()? {
                    heap.push(Reverse((line, index)));
                }
            }
            while let Some(Reverse((line, index))) = heap.pop() {
                emit(&line);
                if let Some(next) = readers[index].next().transpose()? {
                    heap.push(Reverse((next, index)));
                }
            }
        }
        Ok(hex::encode(hasher.finalize()))
    }
}

/// The shape sets in force at `tx`, in the share's encoding.
pub(super) fn shapes_hash(store: &Store, tx: i64) -> Result<String> {
    let sets = store.list_shapes_as_of(&crate::store::AsOf {
        tx: Some(tx),
        valid_at: None,
    })?;
    let sets: BTreeMap<&str, &str> = sets
        .iter()
        .map(|(name, turtle, _)| (name.as_str(), turtle.as_str()))
        .collect();
    let mut out = String::new();
    for (name, turtle) in sets {
        out.push_str(&format!("# --- {name} ---\n{}\n", turtle.trim_end()));
    }
    Ok(crate::share::sha256(out.as_bytes()))
}
