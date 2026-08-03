//! Reading T — hank's trace spool, as records the passes can check.
//!
//! The schema is hank's `crate::trace` plus the attribution fields of
//! `crate::attribution`, and it is deliberately deserialised **permissively**:
//! every field is optional, and an unknown field is ignored rather than fatal.
//! A checker that refused to read a spool because hank had added a field would
//! stop auditing at exactly the moment the two repos drifted, which is when an
//! audit is worth most.
//!
//! What is *not* permissive: a line that will not parse at all is counted, not
//! skipped. Silently dropping it would let the report claim conformance over a
//! window it had only partly read.

use serde::Deserialize;

/// One constraint evaluation within a record — SARC's `E_i`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Evaluation {
    /// The constraint's id, as the trace cites it.
    #[serde(default)]
    pub id: String,
    /// The class the runtime believed it had.
    #[serde(default)]
    pub class: Option<String>,
    /// The point the runtime believed it was evaluated at.
    #[serde(default)]
    pub verification_point: Option<String>,
    /// The layer that ACTUALLY evaluated it, as the runtime recorded it — not
    /// the layer the policy claimed. The difference is the whole of the I6
    /// check.
    #[serde(default)]
    pub hosted_at: Option<String>,
    /// `satisfied` | `unsatisfied` | `unknown`.
    #[serde(default)]
    pub outcome: Option<String>,
    /// `blocked` | `warned` | `logged` | `escalated` | `no-action`.
    #[serde(default)]
    pub response: Option<String>,
}

/// One line of the trace spool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TraceRecord {
    /// The event kind — `guard`, `audit`, and others the passes ignore.
    #[serde(default)]
    pub kind: Option<String>,
    /// The enforcement point, on records that name one.
    #[serde(default)]
    pub point: Option<String>,
    /// The gate result — `allow` | `deny` | `notify`.
    #[serde(default)]
    pub result: Option<String>,
    /// The enforcement mode the decision was made under.
    #[serde(default)]
    pub mode: Option<String>,
    /// The file the action was about, when the runtime was configured to record
    /// it. The only per-record subject the spool carries, and what lets replay
    /// ask whether a refusal was ever escaped.
    #[serde(default)]
    pub path: Option<String>,
    /// The constraints evaluated, with their outcomes and responses.
    #[serde(default)]
    pub constraints: Vec<Evaluation>,
    /// The declared principal-and-agent chain, caller-first.
    #[serde(default)]
    pub principal_chain: Vec<String>,
    /// The agent that decided the action, when declared.
    #[serde(default)]
    pub planner: Option<String>,
    /// The identity of the process that ran it.
    #[serde(default)]
    pub executor: Option<String>,
    /// The tool it ran through.
    #[serde(default)]
    pub tool: Option<String>,
    /// Set when the declared chain's tail disagreed with the executor.
    #[serde(default)]
    pub attribution_conflict: bool,
}

impl TraceRecord {
    /// Whether this record describes a constraint evaluation at all.
    ///
    /// The spool carries `command` and `fail_open` lines too, and holding an
    /// attribution requirement against a line that evaluated nothing would
    /// report an absence that is correct.
    #[must_use]
    pub fn is_enforcement(&self) -> bool {
        !self.constraints.is_empty()
    }

    /// Whether the action was refused.
    #[must_use]
    pub fn refused(&self) -> bool {
        self.result.as_deref() == Some("deny")
    }
}

/// Parse a JSONL trace, returning the records and the count of unreadable lines.
///
/// Blank lines are not "unreadable" — they carry no claim and a trailing newline
/// is not a defect. Anything else that fails to parse is counted.
#[must_use]
pub fn parse_trace(jsonl: &str) -> (Vec<TraceRecord>, usize) {
    let mut records = Vec::new();
    let mut unreadable = 0usize;
    for line in jsonl.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TraceRecord>(line) {
            Ok(record) => records.push(record),
            Err(_) => unreadable += 1,
        }
    }
    (records, unreadable)
}
