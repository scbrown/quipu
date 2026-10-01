//! What a committed write DID, by code path and graph (aegis-gwkd76).
//!
//! `quipu_facts_written_total` counts datums SUBMITTED to committed
//! transactions. That includes idempotent re-assertions, which change nothing,
//! so it cannot be compared with `quipu_graph_facts` (live ROOT-graph facts).
//! This splits every committed write, with no extra store read, into what the
//! staging step already knows: each staged datum is exactly one of an
//! effective assert, an effective retract, or a no-op.

use super::*;
use crate::write_kind::WriteKind;

/// One committed write's counts. `submitted` are the caller's datums;
/// `inferred` are datums OWL domain/range inference added to the same batch;
/// `asserted`/`retracted` are the staged datums that changed the current-fact
/// view; `superseded` are prior values a functional property closed, which are
/// not in the batch. So `noop = submitted + inferred - asserted - retracted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteCounts {
    pub submitted: u64,
    pub inferred: u64,
    pub asserted: u64,
    pub retracted: u64,
    pub superseded: u64,
    /// The write's graph is the root graph, the scope `quipu_graph_facts` counts.
    pub root: bool,
    pub kind: WriteKind,
}

impl WriteCounts {
    #[must_use]
    pub fn noop(&self) -> u64 {
        (self.submitted + self.inferred).saturating_sub(self.asserted + self.retracted)
    }
}

const OUTCOMES: [&str; 6] = [
    "submitted",
    "inferred",
    "asserted",
    "retracted",
    "superseded",
    "noop",
];

#[derive(Default)]
pub(super) struct WriteMetrics {
    counts: Mutex<BTreeMap<(WriteKind, bool, &'static str), u64>>,
}

impl WriteMetrics {
    pub(super) fn observe(&self, c: &WriteCounts) {
        let values = [
            c.submitted,
            c.inferred,
            c.asserted,
            c.retracted,
            c.superseded,
            c.noop(),
        ];
        let mut counts = self.counts.lock().unwrap();
        for (outcome, n) in OUTCOMES.iter().zip(values) {
            if n > 0 {
                *counts.entry((c.kind, c.root, outcome)).or_insert(0) += n;
            }
        }
    }

    pub(super) fn render(&self, out: &mut String) {
        out.push_str(
            "# HELP quipu_write_facts_total Datums of committed writes by code path (writer), graph \
             scope (root: the scope of the graph-facts gauge) and outcome: submitted by the caller, \
             inferred into the batch, asserted or retracted (changed the current-fact view), \
             superseded (prior functional values closed), noop (changed nothing). Net root-graph \
             growth = asserted - retracted - superseded with graph=\"root\".\n\
             # TYPE quipu_write_facts_total counter\n",
        );
        for ((kind, root, outcome), n) in self.counts.lock().unwrap().iter() {
            let graph = if *root { "root" } else { "named" };
            let _ = writeln!(
                out,
                "quipu_write_facts_total{{writer=\"{}\",graph=\"{graph}\",outcome=\"{outcome}\"}} {n}",
                kind.as_str()
            );
        }
    }
}
