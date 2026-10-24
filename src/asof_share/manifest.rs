//! The as-of share's manifest types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The source log a follower ties the cut to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    /// Persistent source lineage.
    pub store_id: String,
    /// Current log epoch; a change means re-bootstrap, never splice.
    pub log_epoch: String,
}

/// What the cut holds for one graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphCompleteness {
    /// Live facts at `T`, one record each.
    pub records: usize,
    /// The applier's `share_graph_digest`: SHA-256 over the graph's records
    /// as canonical compact JSON lines, sorted and joined by `\n` (no
    /// trailing newline), with a `sha256:` prefix. Send order does not reach
    /// it.
    pub digest: String,
    /// The frozen pack the records were read from; always null until frozen
    /// graphs can be cut (aegis-s24.1.1).
    pub frozen: Option<serde_json::Value>,
}

/// What the cut can and cannot carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completeness {
    /// Records carry `valid_from` and `valid_to`.
    pub valid_time: bool,
    /// How byte values are carried.
    pub bytes: String,
    /// Every graph in the cut.
    pub graphs: BTreeMap<String, GraphCompleteness>,
}

/// Build that produced the cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Producer {
    /// Compiled version.
    pub version: String,
    /// Compiled source commit.
    pub git_sha: String,
    /// Whether the build had modified source.
    pub git_dirty: bool,
}

/// The cut's manifest. Its hash names the spool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// [`SCHEMA`].
    pub schema: String,
    /// Source identity read inside the cut.
    pub source: Source,
    /// Last source transaction included; the follower tails from it.
    pub watermark_tx: i64,
    /// Committed head when the cut was taken.
    pub source_head_tx: i64,
    /// Requested scope, sorted.
    pub scope: Vec<String>,
    /// Declared context graphs, sorted.
    pub context_graphs: Vec<String>,
    /// [`scope_digest`] of the scope and context lists.
    pub scope_digest: String,
    /// SHA-256 of the read rule the cut was served under ([`POLICY`]).
    pub policy_digest: String,
    /// SHA-256 of the shape sets in force at `T`, in the share encoding.
    pub shapes_hash: String,
    /// Per-graph record counts and digests.
    pub completeness: Completeness,
    /// Producing build.
    pub producer: Producer,
}
