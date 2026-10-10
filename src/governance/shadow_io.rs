//! Shadow gate I/O: opening a store it is SAFE to replay, and rendering the
//! report (aegis-xfuch4.2).
//!
//! Every as-of ASK is a full historical query (the read model cannot serve
//! history), so replaying real traffic against the LIVE store would load the
//! store the gate protects. The shadow therefore runs on a quiescent COPY, and
//! "use a copy" is enforced, not advised: [`open_quiescent_copy`] refuses
//! the configured live store, and any file showing a live connection.
//!
//! `immutable=1` is what makes the read cheap and lock-free, and it is ALSO
//! what makes it dangerous on a live file: it ignores locks and the WAL, so it
//! can read torn state. It is used only after every refusal check passed.

use std::path::{Path, PathBuf};

use super::shadow::{DiffKind, Report, RuleStats};
use crate::error::{Error, Result};
use crate::store::Store;

/// Why a path was refused. Each names what to do instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The path is the configured live store.
    LiveStore(PathBuf),
    /// A `-wal`, `-shm` or `-journal` file sits beside it: some connection has
    /// it open, or it was copied mid-write and is not quiescent.
    ActiveSidecar(PathBuf),
    /// The path does not exist.
    Missing(PathBuf),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LiveStore(p) => write!(
                f,
                "{} is the configured live store. The shadow gate replays history \
                 with full as-of queries and reads with immutable=1, which ignores \
                 locks and the WAL; it runs only on a quiescent COPY. Take one with \
                 `sqlite3 <live> \".backup <copy>\"` (or `quipu backup`) and pass \
                 --db <copy>.",
                p.display()
            ),
            Self::ActiveSidecar(p) => write!(
                f,
                "{} exists, so the store is open by some connection or was copied \
                 mid-write. An immutable read of it could see torn state. Close \
                 every connection, or take a consistent copy with sqlite3 \".backup\", \
                 then retry.",
                p.display()
            ),
            Self::Missing(p) => write!(f, "{} does not exist", p.display()),
        }
    }
}

/// Check `path` is a quiescent copy, then open it read-only and immutable.
///
/// `live` is the store the configuration points at (the one a server would
/// write); a path that resolves to it is refused whatever else is true.
///
/// # Errors
/// [`Error::InvalidValue`] carrying a [`Refusal`] when the path is refused;
/// store errors opening an accepted path.
pub fn open_quiescent_copy(path: &Path, live: Option<&Path>) -> Result<Store> {
    if let Some(r) = refusal(path, live) {
        return Err(Error::InvalidValue(r.to_string()));
    }
    let abs = std::fs::canonicalize(path)
        .map_err(|e| Error::InvalidValue(format!("{}: {e}", path.display())))?;
    let uri = format!("file:{}?immutable=1", abs.display());
    Store::open_read_only(&uri)
}

/// The refusal for `path`, if any. Pure filesystem checks; opens nothing.
#[must_use]
pub fn refusal(path: &Path, live: Option<&Path>) -> Option<Refusal> {
    let Ok(abs) = std::fs::canonicalize(path) else {
        return Some(Refusal::Missing(path.to_path_buf()));
    };
    if let Some(live) = live
        && std::fs::canonicalize(live).is_ok_and(|l| l == abs)
    {
        return Some(Refusal::LiveStore(abs));
    }
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut side = abs.clone().into_os_string();
        side.push(suffix);
        let side = PathBuf::from(side);
        if side.exists() {
            return Some(Refusal::ActiveSidecar(side));
        }
    }
    None
}

fn rate(n: usize, d: usize) -> String {
    // Integer permille keeps this exact and free of float casts.
    match (n * 1000).checked_div(d) {
        None => "n/a".into(),
        Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
    }
}

fn stats_json(s: &RuleStats) -> serde_json::Value {
    serde_json::json!({
        "evaluations": s.evaluations,
        "satisfied": s.satisfied,
        "unsatisfied": s.unsatisfied,
        "unknown": s.unknown,
        "would_refuse": s.would_refuse,
        "would_escalate": s.would_escalate,
        "approved": s.approved,
        "would_warn": s.would_warn,
        "unevaluable": s.unevaluable,
    })
}

impl Report {
    /// Machine-readable form. Every limit rides with the numbers it bounds.
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        let side = |m: &std::collections::BTreeMap<(String, String), RuleStats>| {
            m.iter()
                .map(|((rule, writer), s)| {
                    let mut v = stats_json(s);
                    v["rule"] = rule.clone().into();
                    v["writer"] = writer.clone().into();
                    v
                })
                .collect::<Vec<_>>()
        };
        let j = &self.verdict_join;
        serde_json::json!({
            "window": {"from_tx": self.from_tx, "to_tx": self.to_tx},
            "transactions": self.transactions,
            "judged": self.judged,
            "bypass_skipped": self.bypass_skipped,
            "unparsed_time": self.unparsed_time,
            "truncated_at": self.truncated_at,
            "policy_boundaries": self.policy_boundaries,
            "baseline_refuses_committed": self.baseline_refuses_committed,
            "refusals_not_replayable": self.refusals_not_replayable,
            "verdict_join": {
                "judged": j.judged, "matched": j.matched,
                "outcome_mismatch": j.outcome_mismatch, "unmatched": j.unmatched,
                "via_gated_tx": j.via_gated_tx, "match_rate": rate(j.matched, j.judged),
            },
            "diffs": self.diffs.iter().map(|d| serde_json::json!({
                "tx": d.tx, "writer": d.writer, "kind": d.kind.as_str(),
                "refused_by": d.refused_by.iter()
                    .map(|(p, t)| serde_json::json!({"rule": p, "target": t}))
                    .collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "candidate": side(&self.candidate),
            "baseline": side(&self.baseline),
            "unevaluable_samples": self.unevaluable_samples,
        })
    }

    /// The operator-facing summary: limits FIRST, then the diff counts, then
    /// the per rule x writer table for the candidate side.
    #[must_use]
    pub fn render(&self) -> String {
        let mut out = String::new();
        let new_ref = self
            .diffs
            .iter()
            .filter(|d| d.kind == DiffKind::NewRefusal)
            .count();
        let now_admit = self.diffs.len() - new_ref;
        let j = &self.verdict_join;
        out.push_str(&format!(
            "shadow gate over tx {}..={}: {} transaction(s), {} judged, {} gate-bypass \
             bookkeeping skipped.\n",
            self.from_tx, self.to_tx, self.transactions, self.judged, self.bypass_skipped
        ));
        if let Some(at) = self.truncated_at {
            out.push_str(&format!(
                "TRUNCATED by --max-txs: stopped before tx {at}. Numbers cover a prefix only.\n"
            ));
        }
        out.push_str(&format!(
            "NOT REPLAYABLE: {} refused write(s) in the window (rolled back, no delta \
             kept). They are counted, not judged.\n",
            self.refusals_not_replayable
        ));
        if j.matched + j.outcome_mismatch == 0 {
            // No recorded verdict joined at all: fidelity is UNMEASURED, which
            // is not the same as a 0% match (no signing identity, enforcement
            // off, or verdicts pruned).
            out.push_str(&format!(
                "fidelity: UNMEASURED: none of {} baseline judgement(s) joined a \
                 recorded verdict (no signing identity, enforcement off, or none kept).\n",
                j.judged
            ));
        } else {
            out.push_str(&format!(
                "fidelity: recorded-verdict match {} ({} matched, {} outcome mismatch, {} \
                 unmatched of {}; {} via aegis:gatedTx, the rest by the next-tx convention).\n",
                rate(j.matched, j.judged),
                j.matched,
                j.outcome_mismatch,
                j.unmatched,
                j.judged,
                j.via_gated_tx,
            ));
        }
        out.push_str(&format!(
            "baseline would refuse {} committed transaction(s): enforcement off then, an \
             approval, or a reconstruction gap.\n",
            self.baseline_refuses_committed
        ));
        if !self.policy_boundaries.is_empty() {
            out.push_str(&format!(
                "policy boundaries (governing set changed after): {:?}. A diff is judged \
                 against the set in force at its own tx, never attributed across a change.\n",
                self.policy_boundaries
            ));
        }
        out.push_str(&format!(
            "\nDIFF: {new_ref} new refusal(s), {now_admit} would-now-admit.\n"
        ));
        for d in &self.diffs {
            let by = d
                .refused_by
                .iter()
                .map(|(p, t)| format!("{p} on {t}"))
                .collect::<Vec<_>>()
                .join("; ");
            out.push_str(&format!(
                "  tx {} [{}] {}: {}\n",
                d.tx,
                d.writer,
                d.kind.as_str(),
                by
            ));
        }
        out.push_str("\ncandidate side, per rule x writer:\n");
        out.push_str(
            "  rule | writer | evals | sat | unsat | unknown | refuse | escalate | approved | warn | unevaluable\n",
        );
        for ((rule, writer), s) in &self.candidate {
            out.push_str(&format!(
                "  {rule} | {writer} | {} | {} | {} | {} | {} | {} | {} | {} | {}\n",
                s.evaluations,
                s.satisfied,
                s.unsatisfied,
                s.unknown,
                s.would_refuse,
                s.would_escalate,
                s.approved,
                s.would_warn,
                s.unevaluable
            ));
        }
        for sample in &self.unevaluable_samples {
            out.push_str(&format!("  unevaluable: {sample}\n"));
        }
        out
    }
}

#[cfg(test)]
#[path = "shadow_io_tests.rs"]
mod tests;
