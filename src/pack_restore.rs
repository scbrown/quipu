//! The pack FORMAT GATE, and `quipu restore` — the reader for a `--full` pack.
//!
//! ## Why separate verbs rather than one smart `unpack`
//!
//! `pack` and `pack --full` produce two artifacts with two contracts, and the
//! two are loaded in OPPOSITE ways: a published pack MERGES into whatever store
//! you point it at, and a full pack IS a store, so loading one REPLACES the
//! destination. Settled with wu (aegis-9f899e, 2026-09-12) as two verbs rather
//! than one verb with a runtime predicate:
//!
//! | verb | does | refuses |
//! |---|---|---|
//! | `unpack`  | MERGES  | a full pack — "use `restore`" |
//! | `restore` | REPLACES | a published pack — "use `unpack`" |
//!
//! An emptiness predicate (`COUNT(facts) == 0` ⇒ replace, else merge) was
//! considered and REJECTED: too loose and it destroys a bookkeeping-only store;
//! too strict and it refuses a legitimate first load, and then somebody loosens
//! it — a loosening that arrives looking like a bugfix with a real complaint
//! behind it. Intent is DECLARED by the verb the operator typed; the manifest
//! alone decides whether that verb may proceed.
//!
//! ## Why the gate is FIRST, before the hash and before emptiness
//!
//! `--full` is a FOOTGUN for a merging consumer, and the failure lands on
//! diligence: "lossless" is exactly what a careful person reaches for when they
//! want everything. Before this module the mis-verb did not say so. Measured on
//! an intact pack (aegis-9f899e):
//!
//! ```text
//! quipu unpack full.qpack       -> exit 1, "unpack error: unknown graph: urn:quipu:whole-store"
//! quipu pack --verify full.qpack -> exit 1, "pack verify error: unknown graph: urn:quipu:whole-store"
//! ```
//!
//! Nothing was corrupted — the refusal is real — but it names a MISSING GRAPH
//! when the truth is an intact artifact and the wrong verb. An operator
//! restoring a backup reads that and goes looking for a graph. So the format
//! check runs before anything that could produce a more specific-sounding and
//! less true diagnosis: "I cannot tell what this artifact is" must never
//! degrade into "seems fine", and "wrong verb" must never render as "corrupt".

use std::path::Path;

use rusqlite::Connection;

use crate::error::{Error, Result};
use crate::pack::{Manifest, read_manifest};

/// A published pack: current facts, re-interned, scrubbed. Read by `unpack`.
pub const FORMAT_PUBLISHED: &str = "1";
/// An interop bundle of plain files. Export-only — nothing reads it back.
pub const FORMAT_TURTLE: &str = "1-turtle";
/// A frozen archive graph, produced by `quipu graph freeze`. Read by `attach`.
pub const FORMAT_FROZEN: &str = "2";
/// A lossless whole-store pack, produced by `pack --full`. Read by `restore`.
pub const FORMAT_FULL: &str = "quipu-pack-full/1";

/// The verbs that load a pack, for naming the right one in a refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// `quipu unpack` — merges a published pack into an existing store.
    Unpack,
    /// `quipu restore` — replaces a store with a full pack.
    Restore,
}

impl Verb {
    /// The pack format this verb reads.
    #[must_use]
    pub fn format(self) -> &'static str {
        match self {
            Self::Unpack => FORMAT_PUBLISHED,
            Self::Restore => FORMAT_FULL,
        }
    }

    /// How the verb appears on a command line.
    #[must_use]
    pub fn command(self) -> &'static str {
        match self {
            Self::Unpack => "quipu unpack",
            Self::Restore => "quipu restore",
        }
    }
}

/// Which verb reads `pack_format`, or `None` for a format nothing loads.
///
/// An unrecognised format returns `Err`, never `None`: "nothing reads this" and
/// "I have never heard of this" are different answers and only one of them is
/// safe to proceed past.
fn reader_for(pack_format: &str) -> Result<Option<Verb>> {
    match pack_format {
        FORMAT_PUBLISHED => Ok(Some(Verb::Unpack)),
        FORMAT_FULL => Ok(Some(Verb::Restore)),
        crate::pack_full_text::FORMAT_FULL_TEXT => Ok(Some(Verb::Restore)),
        FORMAT_TURTLE | FORMAT_FROZEN => Ok(None),
        other => {
            let text_format = crate::pack_full_text::FORMAT_FULL_TEXT;
            Err(Error::InvalidValue(format!(
                "pack_format {other:?} is not a format this build can read. Known \
             formats: {FORMAT_PUBLISHED:?} (published pack, `quipu unpack`), \
             {FORMAT_FULL:?} (full pack, `quipu restore`), {text_format:?} \
             (full TEXT pack, a DIRECTORY, `quipu restore`), {FORMAT_TURTLE:?} \
             (interop bundle, export-only), {FORMAT_FROZEN:?} (frozen archive, \
             `quipu db attach`). Refusing rather than guessing: a pack from a \
             NEWER quipu may carry tables this build would silently drop \
             (aegis-9f899e)."
            )))
        }
    }
}

/// Refuse unless this manifest is a format `verb` reads.
///
/// This is the gate every load path calls FIRST — see the module docs for why
/// its position is part of the guard and not an implementation detail.
///
/// # Errors
/// [`Error::InvalidValue`] for an unknown format, and
/// [`Error::PolicyDenied`] for a known format belonging to another verb — with
/// that verb named, because the operator's next action is to retype the
/// command and a refusal that does not name the replacement sends them
/// debugging the artifact instead.
pub fn require_format(manifest: &Manifest, verb: Verb) -> Result<()> {
    let reader = reader_for(&manifest.pack_format)?;
    if reader == Some(verb) {
        return Ok(());
    }
    let remedy = match reader {
        Some(other) => format!(
            "It is read by `{} <pack>`, which {}.",
            other.command(),
            match other {
                Verb::Unpack => "MERGES it into the destination store",
                Verb::Restore => "REPLACES the destination store with it",
            }
        ),
        None => match manifest.pack_format.as_str() {
            FORMAT_TURTLE => "It is an interop bundle: a directory of plain files for something \
                 that is not quipu to read. Nothing loads it back."
                .to_string(),
            _ => "It is a frozen archive graph; compose it with `quipu db attach`.".to_string(),
        },
    };
    Err(Error::PolicyDenied(format!(
        "{} refuses this pack: it is pack_format {:?}, and {} reads {:?}. {remedy} \
         The pack itself is intact — this is the wrong verb for it, not a damaged \
         artifact (aegis-9f899e).",
        verb.command(),
        manifest.pack_format,
        verb.command(),
        verb.format(),
    )))
}

/// What a [`restore`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    /// Destination store path that now holds the pack's contents.
    pub destination: String,
    /// The pack's `content_hash`, re-verified before the write.
    pub content_hash: String,
    /// Live fact rows the destination held before being replaced.
    pub replaced_facts: i64,
    /// Table count in the restored store. `i64` because that is what
    /// `COUNT(*)` is; narrowing it would be a cast with nothing to gain.
    pub tables: i64,
}

/// Replace `destination` with the whole-store contents of a `--full` pack.
///
/// Order is load-bearing: FORMAT, then HASH, then the destination's emptiness.
/// The destructive step happens only after all three.
///
/// # Errors
/// - [`Error::PolicyDenied`] if `pack_path` is not a full pack (see
///   [`require_format`]), or if `destination` already holds facts and `force`
///   is not set.
/// - [`Error::InvalidValue`] if the pack's recomputed hash does not match its
///   manifest.
/// - [`Error::Store`] for filesystem and SQLite failures.
pub fn restore(pack_path: &str, destination: &str, force: bool) -> Result<RestoreReport> {
    // A text pack is a DIRECTORY, so every sqlite read below would fail on it
    // with a diagnosis about the file rather than about the format. Dispatch
    // before that can happen — "I cannot tell what this artifact is" must never
    // degrade into a more specific-sounding and less true message.
    if crate::pack_full_text::is_text_pack(pack_path) {
        return restore_text(pack_path, destination, force);
    }

    // 1. FORMAT. Before the hash, so a published pack handed to `restore` is
    //    told which verb to use rather than being hashed by the wrong function
    //    and reported as a mismatch — which is corruption's message, not this.
    let manifest = read_manifest(pack_path)?;
    require_format(&manifest, Verb::Restore)?;

    // 2. HASH, with the full pack's OWN hash function.
    let (claimed, recomputed, matches) = crate::pack_full::verify_full(pack_path)?;
    if !matches {
        return Err(Error::InvalidValue(format!(
            "pack: HASH MISMATCH: manifest {claimed}, recomputed {recomputed}. \
             This pack's contents do not match what it claims to be; restoring \
             it would install content nobody signed off on."
        )));
    }

    // 3. EMPTINESS of the DESTINATION — a safety check on what is about to be
    //    destroyed, NOT a way to choose between merging and replacing. That
    //    choice is the verb, and it has already been made by the operator.
    let replaced_facts = live_fact_count(destination)?;
    if replaced_facts > 0 && !force {
        return Err(Error::PolicyDenied(format!(
            "quipu restore refuses to replace {destination}: it holds {replaced_facts} \
             live fact(s), and a restore REPLACES the whole store rather than \
             merging into it. Move it aside, point --db at a fresh path, or pass \
             --force if destroying it is what you mean. To ADD this pack's \
             contents to a populated store you want a published pack and \
             `quipu unpack`, not a full pack (aegis-9f899e)."
        )));
    }

    // A full pack IS a store, so the restore is a copy. VACUUM INTO rather than
    // a filesystem copy for the reason `pack --full` gives on the way out: the
    // pack is opened in WAL mode, and copying the file alone silently drops
    // whatever is still in the -wal.
    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{destination}{suffix}");
        if Path::new(&p).exists() {
            std::fs::remove_file(&p)
                .map_err(|e| Error::Store(format!("quipu restore: cannot replace {p}: {e}")))?;
        }
    }
    let conn = Connection::open(pack_path)?;
    conn.execute(
        &format!("VACUUM INTO '{}'", destination.replace('\'', "''")),
        [],
    )?;
    drop(conn);

    let restored = Connection::open(destination)?;
    let tables: i64 = restored.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [],
        |r| r.get(0),
    )?;

    Ok(RestoreReport {
        destination: destination.to_string(),
        content_hash: manifest.content_hash,
        replaced_facts,
        tables,
    })
}

/// `quipu restore <dir>` for a TEXT pack (aegis-9f899e).
///
/// Same contract as the binary path and in the same ORDER, which is the part
/// that matters: format gate first, then the hash, then the destination
/// emptiness check, and only then a write. The hash here is not a formality —
/// it is the whole acceptance criterion for this format. A text pack is
/// replayed rather than copied, so "did every row survive the round trip"
/// cannot be assumed the way it can for a `VACUUM INTO`, and a dump missing a
/// file, a table, or a single row would otherwise install a quietly smaller
/// store that looks perfectly healthy.
fn restore_text(dir: &str, destination: &str, force: bool) -> Result<RestoreReport> {
    let manifest = crate::pack_full_text::read_manifest_dir(dir)?;
    require_format(&manifest, Verb::Restore)?;

    let build_path = format!("{destination}.rebuilding");
    let rebuilt = crate::pack_full_text::rebuild(dir, &build_path);
    let cleanup = || {
        for suffix in ["", "-wal", "-shm"] {
            let p = format!("{build_path}{suffix}");
            if Path::new(&p).exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
    };
    let (manifest, recomputed) = match rebuilt {
        Ok(v) => v,
        Err(e) => {
            cleanup();
            return Err(e);
        }
    };
    if recomputed != manifest.content_hash {
        cleanup();
        return Err(Error::InvalidValue(format!(
            "quipu restore: HASH MISMATCH after reconstructing {dir}: manifest {}, \
             rebuilt {recomputed}. The text pack does not reconstruct the store it \
             claims to; a file, a table, or a row is missing or altered. Nothing \
             was written (aegis-9f899e).",
            manifest.content_hash
        )));
    }

    let replaced_facts = match live_fact_count(destination) {
        Ok(n) => n,
        Err(e) => {
            cleanup();
            return Err(e);
        }
    };
    if replaced_facts > 0 && !force {
        cleanup();
        return Err(Error::PolicyDenied(format!(
            "quipu restore refuses to replace {destination}: it holds {replaced_facts} \
             live fact(s), and a restore REPLACES the whole store rather than \
             merging into it. Move it aside, point --db at a fresh path, or pass \
             --force if destroying it is what you mean (aegis-9f899e)."
        )));
    }

    for suffix in ["", "-wal", "-shm"] {
        let p = format!("{destination}{suffix}");
        if Path::new(&p).exists() {
            std::fs::remove_file(&p)
                .map_err(|e| Error::Store(format!("quipu restore: cannot replace {p}: {e}")))?;
        }
    }
    let conn = Connection::open(&build_path)?;
    conn.execute(
        &format!("VACUUM INTO '{}'", destination.replace('\'', "''")),
        [],
    )?;
    drop(conn);
    cleanup();

    let restored = Connection::open(destination)?;
    let tables: i64 = restored.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        [],
        |r| r.get(0),
    )?;
    Ok(RestoreReport {
        destination: destination.to_string(),
        content_hash: manifest.content_hash,
        replaced_facts,
        tables,
    })
}

/// Live fact rows in a store, or `0` for a path that is not a store yet.
///
/// A missing file, and a file with no `facts` table, both mean "nothing to
/// destroy". Reporting an error for either would make `restore` unusable for
/// its main case — restoring into a fresh path.
fn live_fact_count(destination: &str) -> Result<i64> {
    if !Path::new(destination).exists() {
        return Ok(0);
    }
    let conn = Connection::open(destination)?;
    let has_facts: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='facts')",
        [],
        |r| r.get(0),
    )?;
    if !has_facts {
        return Ok(0);
    }
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM facts WHERE valid_to IS NULL",
        [],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
#[path = "pack_restore_tests.rs"]
mod tests;
