//! The project graph a repository commits under `.quipu/` (aegis-w3k75d.11).
//!
//! A repository carries its PROJECT's quipu named graph, tool-neutral, as three
//! committed paths:
//!
//! ```text
//! .quipu/project      one line: the project id; the graph IRI derives from it
//! .quipu/graph/       the share-manifest v1 bundle (manifest, export, shapes)
//! .quipu/.gitignore   written by quipu: an ALLOW-list for the two above
//! ```
//!
//! Everything else under `.quipu/` is local and must never be committed: the
//! store (`local.db`, `-wal`, `-shm`) and the host signing key
//! (`verifier.pk8`, a PRIVATE key, see `signing::default_key_path`). The ignore
//! file is an allow-list rather than a deny-list on purpose: whatever quipu
//! writes into `.quipu/` next is local by default, instead of committed by
//! default until somebody remembers to add a rule. It also needs no edit to the
//! repository's own `.gitignore`.
//!
//! Loading reuses the share import unchanged (hash verification, the
//! destination scrub, attestation, resolution, SHACL and quarantine) and then
//! promotes into the bundle's scope graph instead of ROOT, as a diff, so a
//! re-load after `git pull` changes only what changed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::store::{Datum, Store};
use crate::types::Op;

/// Directory, relative to a repository root, that holds everything quipu keeps.
pub const QUIPU_DIR: &str = ".quipu";
/// The committed bundle directory, relative to [`QUIPU_DIR`].
pub const GRAPH_DIR: &str = "graph";
/// The committed project id file, relative to [`QUIPU_DIR`].
pub const PROJECT_FILE: &str = "project";

/// The ignore file quipu writes at `.quipu/.gitignore`.
///
/// `*` first, then re-include exactly what is committed. `graph/` needs the
/// directory AND its contents re-included, because git will not descend into
/// an ignored directory to find a negated file.
pub const GITIGNORE: &str = "\
# Written by quipu (aegis-w3k75d.11). Commit ONLY the project graph.
# Everything else here is local: the store (local.db*) and the host signing
# key (verifier.pk8, a PRIVATE key). An allow-list, so anything quipu writes
# here later is local by default.
*
!.gitignore
!project
!graph/
!graph/**
";

/// The named-graph IRI for a project id.
pub fn project_iri(id: &str) -> String {
    format!("urn:quipu:project:{id}")
}

/// A project id is one path-and-IRI-safe token: letters, digits, `-`, `_`, `.`.
pub fn validate_project_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidValue(format!(
            "project id {id:?} must be 1-128 characters of letters, digits, '-', '_' or '.', \
             not starting with '.'"
        )))
    }
}

/// `<root>/.quipu`.
pub fn quipu_dir(root: &Path) -> PathBuf {
    root.join(QUIPU_DIR)
}

/// `<root>/.quipu/graph`.
pub fn graph_dir(root: &Path) -> PathBuf {
    quipu_dir(root).join(GRAPH_DIR)
}

/// Read the committed project id, if `.quipu/project` exists.
pub fn read_project_id(root: &Path) -> Result<Option<String>> {
    let path = quipu_dir(root).join(PROJECT_FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let id = text.trim().to_string();
            validate_project_id(&id)?;
            Ok(Some(id))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Store(format!("read {}: {e}", path.display()))),
    }
}

/// Create `.quipu/`, write the ignore file, and settle the project id.
///
/// `id` given and none committed: write it. `id` given and a DIFFERENT one
/// committed: refuse, because silently re-pointing a repository at another
/// graph would orphan the committed one. Neither: refuse and say how to name
/// it. Returns the id in force.
pub fn scaffold(root: &Path, id: Option<&str>) -> Result<String> {
    let dir = quipu_dir(root);
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::Store(format!("create {}: {e}", dir.display())))?;
    std::fs::write(dir.join(".gitignore"), GITIGNORE)
        .map_err(|e| Error::Store(format!("write {}/.gitignore: {e}", dir.display())))?;
    let committed = read_project_id(root)?;
    let id = match (id, committed) {
        (Some(given), Some(existing)) if given != existing => {
            return Err(Error::InvalidValue(format!(
                "this repository's project id is {existing:?} (.quipu/project); refusing to \
                 re-point it at {given:?}. Edit .quipu/project deliberately to change it."
            )));
        }
        (_, Some(existing)) => existing,
        (Some(given), None) => {
            validate_project_id(given)?;
            std::fs::write(dir.join(PROJECT_FILE), format!("{given}\n"))
                .map_err(|e| Error::Store(format!("write .quipu/project: {e}")))?;
            given.to_string()
        }
        (None, None) => {
            return Err(Error::InvalidValue(
                "no project id: name it once with `quipu share --project <id>`; it is \
                 committed in .quipu/project"
                    .into(),
            ));
        }
    };
    Ok(id)
}

/// Where `share --project` writes before swapping into [`GRAPH_DIR`]. Ignored
/// by the allow-list, so an interrupted share never stages a half bundle.
pub const NEXT_DIR: &str = "graph.next";

/// Replace `.quipu/graph` with the freshly written `.quipu/graph.next`.
///
/// `share` refuses an existing destination, which is right for an arbitrary
/// `--output` and wrong for the committed project bundle, whose normal life is
/// to be re-shared after every change. Writing beside it and swapping keeps the
/// old bundle intact until the new one is complete.
pub fn install_next_bundle(root: &Path) -> Result<()> {
    let dir = quipu_dir(root);
    let next = dir.join(NEXT_DIR);
    let live = dir.join(GRAPH_DIR);
    let prev = dir.join("graph.prev");
    let io = |what: &str, e: std::io::Error| Error::Store(format!("{what}: {e}"));
    if prev.exists() {
        std::fs::remove_dir_all(&prev).map_err(|e| io("remove graph.prev", e))?;
    }
    if live.exists() {
        std::fs::rename(&live, &prev).map_err(|e| io("move graph aside", e))?;
    }
    if let Err(e) = std::fs::rename(&next, &live) {
        // Put the previous bundle back rather than leave no bundle at all.
        let _ = std::fs::rename(&prev, &live);
        return Err(io("install graph.next", e));
    }
    if prev.exists() {
        std::fs::remove_dir_all(&prev).map_err(|e| io("remove graph.prev", e))?;
    }
    Ok(())
}

/// Clear a stale `.quipu/graph.next` left by an interrupted share.
pub fn clear_next_bundle(root: &Path) -> Result<PathBuf> {
    let next = quipu_dir(root).join(NEXT_DIR);
    if next.exists() {
        std::fs::remove_dir_all(&next)
            .map_err(|e| Error::Store(format!("remove stale graph.next: {e}")))?;
    }
    Ok(next)
}

/// What a project load changed.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProjectLoadResult {
    pub graph: String,
    pub share_id: String,
    pub tx_id: Option<i64>,
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
}

/// Promote an imported (staged, committed, not quarantined) share into the
/// named graph `target`, as a diff against what `target` holds now.
///
/// The staging graph is kept: it is the unmodified evidence of what the bundle
/// supplied, exactly as for a ROOT promotion.
pub fn promote_into_graph(
    store: &mut Store,
    share_id: &str,
    staging_iri: &str,
    target: &str,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<ProjectLoadResult> {
    let staging = store
        .lookup(staging_iri)?
        .filter(|g| store.graph_class(*g).ok().flatten().as_deref() == Some("committed"))
        .ok_or_else(|| {
            Error::InvalidValue(format!(
                "no eligible staged import for {share_id} (quarantined or missing)"
            ))
        })?;
    let wanted = store.current_facts_in_graph(staging)?;
    let g = store.graph_create(target)?;
    let have = store.current_facts_in_graph(g)?;
    let key = |f: &crate::types::Fact| (f.entity, f.attribute, f.value.to_bytes());
    let wanted_keys: HashSet<_> = wanted.iter().map(key).collect();
    let have_keys: HashSet<_> = have.iter().map(key).collect();
    let datum = |f: &crate::types::Fact, op| Datum {
        entity: f.entity,
        attribute: f.attribute,
        value: f.value.clone(),
        valid_from: timestamp.to_string(),
        valid_to: None,
        op,
    };
    let mut changes: Vec<Datum> = have
        .iter()
        .filter(|f| !wanted_keys.contains(&key(f)))
        .map(|f| datum(f, Op::Retract))
        .collect();
    let removed = changes.len();
    changes.extend(
        wanted
            .iter()
            .filter(|f| !have_keys.contains(&key(f)))
            .map(|f| datum(f, Op::Assert)),
    );
    let added = changes.len() - removed;
    let unchanged = wanted.len() - added;
    let tx_id = if changes.is_empty() {
        None
    } else {
        let source = format!("quipu:project-load:{share_id}");
        Some(store.transact_to_graph(&changes, timestamp, actor, Some(&source), g)?)
    };
    Ok(ProjectLoadResult {
        graph: target.to_string(),
        share_id: share_id.to_string(),
        tx_id,
        added,
        removed,
        unchanged,
    })
}

#[cfg(test)]
#[path = "project_graph_tests.rs"]
mod tests;
