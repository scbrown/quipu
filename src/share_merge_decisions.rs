//! Resolve a share merge that cannot auto-merge (aegis-yavo9c).
//!
//! `quipu merge` holds every conflicting slot at its base value and writes
//! nothing. This module lets an operator finish such a merge, with the same
//! propose / decide / apply split as `quipu align`:
//!
//! 1. [`emit`] writes a decisions file: one row per [`DecisionRecord`], with the
//!    provenance of each side, bound to the exact ROOT and incoming share it
//!    was computed from. Nothing is written to the store.
//! 2. [`propose`] fills each row's `proposal` from mechanical evidence. It never
//!    fills `decision` and never writes the store: an agent proposes, the
//!    operator decides.
//! 3. [`apply`] commits the clean merge plus every decided slot in ONE
//!    transaction whose source names the reviewer and the decisions file's
//!    hash. It refuses, writing nothing, on an undecided row, a stale file
//!    (ROOT or the incoming share moved, or the conflicts are no longer the
//!    ones decided), a decided value count above the slot's `sh:maxCount`, or a
//!    value that is not an RDF term.

#![cfg(not(target_arch = "wasm32"))]

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::share::ShareManifest;
use crate::share::sha256;
use crate::share_merge::{
    DecisionRecord, Graph, LoadedShare, MergeResult, commit, locate_base, merge_graphs,
    parse_graph, read_share, root_graph, share_from_parts,
};
use crate::store::Store;

/// Schema of the decisions file.
pub const DECISIONS_SCHEMA: &str = "https://github.com/scbrown/quipu/merge-decisions/v1";

/// A decisions file: the conflicts of one merge, bound to its inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionFile {
    pub schema: String,
    pub incoming_share: String,
    pub base_share: String,
    /// ROOT's graph hash when the file was emitted; [`apply`] refuses if ROOT
    /// has changed since.
    pub local_graph_hash: String,
    pub incoming_graph_hash: String,
    pub rows: Vec<DecisionRow>,
}

/// One conflicting slot and, once decided, its resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRow {
    pub id: String,
    #[serde(flatten)]
    pub record: DecisionRecord,
    pub provenance: SideProvenance,
    /// Filled by [`propose`]; advisory only, never applied.
    pub proposal: Option<Proposal>,
    /// Filled by the operator; [`apply`] refuses while any row lacks one.
    pub decision: Option<Decision>,
}

/// Where each side's values came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SideProvenance {
    /// ROOT's current facts for the slot.
    pub ours: Vec<LocalFact>,
    pub theirs: ShareOrigin,
    pub base: ShareOrigin,
}

/// One current ROOT fact and the transaction that wrote it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalFact {
    pub value: String,
    pub valid_from: String,
    pub tx: i64,
    pub actor: Option<String>,
    pub source: Option<String>,
}

/// A share's identity, as the share itself states it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShareOrigin {
    pub share_id: String,
    pub created_at: String,
    pub store_id: String,
    pub attested: bool,
}

/// Which side a resolution takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Ours,
    Theirs,
    Base,
}

/// An operator's resolution: one side's values, or explicit values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Decision {
    Choose { choose: Side },
    Values { values: Vec<String> },
}

/// A proposed resolution and the evidence for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    pub choose: Side,
    pub evidence: Vec<String>,
}

/// Result of [`apply`]: the merge result plus each applied choice.
#[derive(Debug, Clone, Serialize)]
pub struct DecidedMerge {
    #[serde(flatten)]
    pub merge: MergeResult,
    pub reviewer: String,
    pub decisions_sha256: String,
    pub applied: Vec<AppliedRow>,
}

/// What one decided row wrote.
#[derive(Debug, Clone, Serialize)]
pub struct AppliedRow {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub values: Vec<String>,
}

struct Inputs {
    incoming: LoadedShare,
    base: LoadedShare,
    ours: Graph,
    ours_hash: String,
}

/// A share delivered in a request body rather than a directory, in the same
/// shape `/import` takes (aegis-yavo9c).
#[derive(Debug, Clone, Deserialize)]
pub struct InlineShare {
    pub manifest: ShareManifest,
    pub export_ntriples: String,
    pub shapes_turtle: String,
}

/// The incoming share and its base: read from a directory (the base found
/// through `parent_share`), or both inline.
pub struct SharePair {
    incoming: LoadedShare,
    base: LoadedShare,
}

impl SharePair {
    /// Read `incoming_dir` and locate its base beside it.
    ///
    /// # Errors
    /// Share read or verification failures, or no unique base.
    pub fn from_dir(incoming_dir: &Path) -> Result<Self> {
        let incoming = read_share(incoming_dir)?;
        let base = locate_base(&incoming)?;
        Ok(Self { incoming, base })
    }

    /// Verify two inline shares and that `base` is `incoming`'s parent.
    ///
    /// # Errors
    /// A hash or envelope mismatch on either share, or a base that is not the
    /// incoming share's declared `parent_share`.
    pub fn inline(incoming: InlineShare, base: InlineShare) -> Result<Self> {
        let load = |what: &str, s: InlineShare| {
            share_from_parts(
                Path::new(what),
                s.manifest,
                &s.export_ntriples,
                s.shapes_turtle,
            )
        };
        let incoming = load("inline incoming share", incoming)?;
        let base = load("inline base share", base)?;
        if incoming.manifest.parent_share.as_deref() != Some(base.manifest.share_id.as_str()) {
            return Err(Error::InvalidValue(format!(
                "the base share {} is not the incoming share's parent_share ({:?})",
                base.manifest.share_id, incoming.manifest.parent_share
            )));
        }
        Ok(Self { incoming, base })
    }
}

fn inputs(store: &Store, pair: SharePair) -> Result<Inputs> {
    let (ours, ours_hash) = root_graph(store)?;
    Ok(Inputs {
        incoming: pair.incoming,
        base: pair.base,
        ours,
        ours_hash,
    })
}

fn origin(share: &LoadedShare) -> ShareOrigin {
    ShareOrigin {
        share_id: share.manifest.share_id.clone(),
        created_at: share.manifest.created_at.clone(),
        store_id: share.manifest.store_id.clone(),
        attested: share.manifest.attestation.is_some(),
    }
}

fn local_facts(store: &Store, record: &DecisionRecord) -> Result<Vec<LocalFact>> {
    let Some(subject) = record
        .subject
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
    else {
        return Ok(Vec::new());
    };
    let (Some(entity), Some(attribute)) =
        (store.lookup(subject)?, store.lookup(&record.predicate)?)
    else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for fact in store.entity_facts(entity)? {
        if fact.attribute != attribute {
            continue;
        }
        let tx = store.get_transaction(fact.tx)?;
        out.push(LocalFact {
            value: crate::rdf::value_to_term(store, &fact.value)?.to_string(),
            valid_from: fact.valid_from,
            tx: fact.tx,
            actor: tx.as_ref().and_then(|t| t.actor.clone()),
            source: tx.and_then(|t| t.source),
        });
    }
    out.sort_by(|a, b| a.value.cmp(&b.value));
    Ok(out)
}

/// Compute the decisions file for merging `incoming_dir` into ROOT.
///
/// # Errors
/// Share read, base lookup, or store read failures.
pub fn emit(store: &Store, incoming_dir: &Path) -> Result<DecisionFile> {
    emit_pair(store, SharePair::from_dir(incoming_dir)?)
}

/// [`emit`] for an already-loaded share pair.
///
/// # Errors
/// Store read failures.
pub fn emit_pair(store: &Store, pair: SharePair) -> Result<DecisionFile> {
    let i = inputs(store, pair)?;
    let (_, conflicts) = merge_graphs(
        &i.base.graph,
        &i.ours,
        &i.incoming.graph,
        &i.incoming.shapes,
    )?;
    let mut rows = Vec::new();
    for (n, record) in conflicts.into_iter().enumerate() {
        rows.push(DecisionRow {
            id: format!("d{}", n + 1),
            provenance: SideProvenance {
                ours: local_facts(store, &record)?,
                theirs: origin(&i.incoming),
                base: origin(&i.base),
            },
            record,
            proposal: None,
            decision: None,
        });
    }
    Ok(DecisionFile {
        schema: DECISIONS_SCHEMA.into(),
        incoming_share: i.incoming.manifest.share_id,
        base_share: i.base.manifest.share_id,
        local_graph_hash: i.ours_hash,
        incoming_graph_hash: i.incoming.manifest.graph_hash,
        rows,
    })
}

/// Fill each row's `proposal` from mechanical evidence. Never decides.
#[must_use]
pub fn propose(mut file: DecisionFile) -> DecisionFile {
    for row in &mut file.rows {
        row.proposal = Some(proposal_for(row));
    }
    file
}

fn proposal_for(row: &DecisionRow) -> Proposal {
    let r = &row.record;
    let mut evidence = Vec::new();
    if r.ours == r.base {
        evidence.push("ours is unchanged from base; only theirs changed this slot".into());
        return Proposal {
            choose: Side::Theirs,
            evidence,
        };
    }
    if r.theirs == r.base {
        evidence.push("theirs is unchanged from base; only ours changed this slot".into());
        return Proposal {
            choose: Side::Ours,
            evidence,
        };
    }
    let ours_time = row
        .provenance
        .ours
        .iter()
        .map(|f| f.valid_from.as_str())
        .max();
    let theirs_time = row.provenance.theirs.created_at.as_str();
    if row.provenance.theirs.attested {
        evidence.push("the incoming share is attested".into());
    }
    let choose = match ours_time {
        // Both sides are RFC 3339 UTC in practice; a lexical comparison of
        // mixed offsets would be wrong, so say which strings were compared.
        Some(ours) if ours > theirs_time => {
            evidence.push(format!(
                "ours is newer: local valid_from {ours} > incoming share created_at {theirs_time}"
            ));
            Side::Ours
        }
        Some(ours) => {
            evidence.push(format!(
                "theirs is newer: incoming share created_at {theirs_time} >= local valid_from {ours}"
            ));
            Side::Theirs
        }
        None => {
            evidence.push(
                "ours deleted the slot and theirs replaced it; proposing the replacement".into(),
            );
            Side::Theirs
        }
    };
    evidence.push("mechanical proposal: the operator decides".into());
    Proposal { choose, evidence }
}

fn chosen(row: &DecisionRow) -> Result<Vec<String>> {
    let r = &row.record;
    Ok(match &row.decision {
        None => {
            return Err(Error::InvalidValue(format!(
                "row {} ({} {}) is undecided; nothing was written",
                row.id, r.subject, r.predicate
            )));
        }
        Some(Decision::Choose { choose: Side::Ours }) => r.ours.clone(),
        Some(Decision::Choose {
            choose: Side::Theirs,
        }) => r.theirs.clone(),
        Some(Decision::Choose { choose: Side::Base }) => r.base.clone(),
        Some(Decision::Values { values }) => values.clone(),
    })
}

/// Commit the clean merge plus every decided slot, atomically.
///
/// # Errors
/// [`Error::InvalidValue`] on an undecided row, a stale or foreign decisions
/// file, too many values for a slot, or a value that is not an RDF term; in
/// every such case nothing is written. Store failures otherwise.
pub fn apply(
    store: &mut Store,
    incoming_dir: &Path,
    file: &DecisionFile,
    file_bytes: &[u8],
    reviewer: &str,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<DecidedMerge> {
    let pair = SharePair::from_dir(incoming_dir)?;
    apply_pair(store, pair, file, file_bytes, reviewer, timestamp, actor)
}

/// [`apply`] for an already-loaded share pair.
///
/// # Errors
/// As [`apply`].
pub fn apply_pair(
    store: &mut Store,
    pair: SharePair,
    file: &DecisionFile,
    file_bytes: &[u8],
    reviewer: &str,
    timestamp: &str,
    actor: Option<&str>,
) -> Result<DecidedMerge> {
    if reviewer.trim().is_empty() {
        return Err(Error::InvalidValue("a reviewer is required".into()));
    }
    if file.schema != DECISIONS_SCHEMA {
        return Err(Error::InvalidValue(format!(
            "not a merge decisions file: schema {:?}",
            file.schema
        )));
    }
    let i = inputs(store, pair)?;
    let stale = |what: &str| {
        Error::InvalidValue(format!(
            "stale decisions: {what} changed since they were emitted (--emit-decisions / quipu_merge_decisions); emit again and re-decide. Nothing was written"
        ))
    };
    if file.incoming_share != i.incoming.manifest.share_id {
        return Err(stale("the incoming share"));
    }
    if file.local_graph_hash != i.ours_hash {
        return Err(stale("ROOT"));
    }
    let (mut merged, conflicts) = merge_graphs(
        &i.base.graph,
        &i.ours,
        &i.incoming.graph,
        &i.incoming.shapes,
    )?;
    // Same conflicts, in any row order: an operator may sort the file.
    let same = conflicts.len() == file.rows.len()
        && conflicts
            .iter()
            .all(|c| file.rows.iter().any(|row| &row.record == c));
    if !same {
        return Err(stale("the set of conflicts"));
    }
    let mut applied = Vec::new();
    for row in &file.rows {
        let r = &row.record;
        let values = chosen(row)?;
        if values.len() > r.max_count {
            return Err(Error::InvalidValue(format!(
                "row {} decides {} values for {} {}, above sh:maxCount {}; nothing was written",
                row.id,
                values.len(),
                r.subject,
                r.predicate,
                r.max_count
            )));
        }
        merged.retain(|t| {
            !(t.subject.to_string() == r.subject && t.predicate.as_str() == r.predicate)
        });
        for value in &values {
            let line = format!("{} <{}> {} .", r.subject, r.predicate, value);
            let parsed = parse_graph(&line, &format!("row {} value {value}", row.id))
                .map_err(|e| Error::InvalidValue(format!("{e}; nothing was written")))?;
            merged.extend(parsed);
        }
        applied.push(AppliedRow {
            id: row.id.clone(),
            subject: r.subject.clone(),
            predicate: r.predicate.clone(),
            values,
        });
    }
    let decisions_sha256 = sha256(file_bytes);
    let parents = [i.ours_hash, i.incoming.manifest.share_id.clone()];
    let source = format!(
        "share-merge:parents={},{};decisions={decisions_sha256};reviewer={reviewer}",
        parents[0], parents[1]
    );
    let merge = commit(store, &i.ours, &merged, parents, &source, timestamp, actor)?;
    Ok(DecidedMerge {
        merge,
        reviewer: reviewer.into(),
        decisions_sha256,
        applied,
    })
}

#[cfg(test)]
#[path = "share_merge_decisions_tests.rs"]
mod tests;
