//! Attachment config — the `[[quipu.attachments]]` table (quipu-at2).
//!
//! ATTACH composition has been complete and tested in the store since #75:
//! `Store::open_with_attachments` mounts read-only databases alongside the
//! local one, verifies their schema and term space, and registers the graphs
//! they contribute so `GRAPH ?g` ranges them. What it did NOT have was a way
//! in from outside the library — the only production caller was deep freeze's
//! auto-attach, so "here is a pack, query it alongside your store" had no
//! non-library path. This module is that path.
//!
//! The declaration is deliberately thin: an alias and a path. Everything that
//! decides whether the composition is *sound* — the `g` column, the term
//! space, the schema — is already checked by `attach::verify_attached_schema`
//! at open, and duplicating any of it here would be a second opinion that can
//! disagree with the first.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::store::attach::Attachment;

/// One declared attachment: a database to mount alongside the store.
#[derive(Debug, Clone, Deserialize)]
pub struct AttachmentConfig {
    /// The `SQLite` schema name the file mounts under, and the `graphs.source`
    /// value its contributed graphs carry. Must be a plain identifier;
    /// `attach::attach_all` refuses anything else rather than interpolating it
    /// into SQL.
    pub alias: String,

    /// Path to the database file. Relative paths resolve against the process's
    /// working directory, like every other path in this config.
    pub path: PathBuf,
}

impl AttachmentConfig {
    /// The minimal declaration.
    pub fn new(alias: impl Into<String>, path: impl Into<PathBuf>) -> Self {
        Self {
            alias: alias.into(),
            path: path.into(),
        }
    }
}

/// Resolve declared attachments into mountable [`Attachment`]s.
///
/// Mounts are **read-only, always**. A writable attachment is not offered as a
/// config knob: cross-database writes are permanently out
/// (`docs/design/multi-db-composition.md` §6), so a `read_only = false` key
/// would be an affordance for something the store refuses anyway.
///
/// # Errors
/// [`Error::Store`] naming the alias and path when a declared file does not
/// exist. This is the same refusal the frozen-pack registry makes for a
/// missing archive, and for the same reason: a declared layer silently absent
/// from the composition turns every query over it into a confident zero rows.
/// The remaining checks — alias validity, the `g` column, term-space
/// collisions — belong to `attach::attach_all` and run at open.
pub fn resolve_attachments(declared: &[AttachmentConfig]) -> Result<Vec<Attachment>> {
    let mut out = Vec::with_capacity(declared.len());
    for a in declared {
        if !a.path.exists() {
            return Err(Error::Store(format!(
                "[[quipu.attachments]] declares alias {:?} at {:?}, but no such file \
                 exists. Refusing to open with a declared layer silently absent — a \
                 query over its graphs would answer zero rows and look successful. \
                 Fix the path, or remove the declaration.",
                a.alias,
                a.path.display().to_string()
            )));
        }
        out.push(Attachment::read_only(&a.alias, &a.path.to_string_lossy()));
    }
    Ok(out)
}

/// Open a store with the `[[quipu.attachments]]` layers of `config` mounted.
///
/// This is the production caller ATTACH composition never had. Deep freeze's
/// archives are mounted by the store itself on top of these, so a store can
/// compose declared layers and its own frozen windows at once.
///
/// Named for what it actually does: it applies the ATTACHMENT half of the
/// config and nothing else. The other knobs (`base_ns`, `search`, `labels`, …)
/// are applied by the binaries after open, each with its own announcement, and
/// folding them in here would hide those behind a name that does not say so.
///
/// # Errors
/// Propagates [`resolve_attachments`]'s refusal for a missing file, and
/// `Store::open_with_attachments`'s for an invalid alias, a duplicate alias, or
/// a file quipu cannot compose (no `g` column, colliding term space).
pub fn open_with_configured_attachments(
    db_path: &str,
    config: &crate::config::QuipuConfig,
) -> Result<crate::store::Store> {
    let mounts = resolve_attachments(&config.attachments)?;
    crate::store::Store::open_with_attachments(db_path, &mounts)
}

/// One line per mounted attachment, for `quipu db attach --list`.
///
/// Reads what the store ACTUALLY has mounted rather than re-reading the
/// config: the config is the request, and the point of a visibility surface is
/// to show what the request became — including deep freeze's archives, which
/// no config declares.
pub fn describe_attachments(store: &crate::store::Store) -> Vec<String> {
    store
        .attachments()
        .iter()
        .map(|a| {
            let mode = if a.read_only { "ro" } else { "rw" };
            let exists = if Path::new(&a.path).exists() {
                ""
            } else {
                "  (MISSING)"
            };
            format!("{}\t{}\t{mode}{exists}", a.alias, a.path)
        })
        .collect()
}
