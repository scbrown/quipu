//! The defect catalogue, transcribed from `docs/design/paper-principles.md`
//! §4 — the single source of planted/expected text for every probe. If the
//! design doc's catalogue and this file disagree, that is a bug in whichever
//! one changed without the other.

use crate::manifest::ManifestEntry;

/// `(id, phase, plants, expected_gated, expected_control, scored_by)`
pub const PROBES: &[(&str, u8, &str, &str, &str, &str)] = &[
    // Phase 1 — founding (setup, executed identically in both arms).
    (
        "CEN-F1",
        1,
        "recorder identities typed and labelled in ROOT",
        "facts land",
        "facts land",
        "setup",
    ),
    (
        "CEN-F2",
        1,
        "district graph labelled (freshness + ranked trust)",
        "label lands in the meta-graph",
        "label lands in the meta-graph",
        "setup",
    ),
    (
        "CEN-F3",
        1,
        "principal authority grants",
        "facts land",
        "facts land",
        "setup",
    ),
    (
        "CEN-F4",
        1,
        "Sigma and the declared vocabulary",
        "facts land",
        "facts land",
        "setup",
    ),
    (
        "CEN-F5",
        1,
        "arm configuration",
        "gates on",
        "gates off",
        "setup",
    ),
    (
        "CEN-F6",
        1,
        "SHACL record shape loaded",
        "loaded",
        "loaded",
        "setup",
    ),
    // Phase 2 — recording.
    (
        "CEN-U1",
        2,
        "episode record missing required provenance property",
        "refused; SHACL names the missing property",
        "lands silently",
        "RQ2",
    ),
    (
        "CEN-A1",
        2,
        "write into a district the writer has no authority over",
        "refused; empty-intersection refusal",
        "lands silently",
        "RQ2",
    ),
    (
        "CEN-A2",
        2,
        "delegated writer exceeding the delegator's grant",
        "refused; intersection narrows, never widens",
        "lands silently",
        "RQ2",
    ),
    (
        "CEN-P1",
        2,
        "write violating a policy claim on post-state",
        "refused; deny cites the policy",
        "lands silently",
        "RQ2",
    ),
    (
        "CEN-P2",
        2,
        "write valid against pre-state, invalid only in post-state (second placement)",
        "refused - separates post-state from pre-state gating",
        "lands silently",
        "RQ2",
    ),
    (
        "CEN-V1",
        2,
        "fact using a fabricated predicate in the policed namespace",
        "refused by the closed-world vocabulary policy",
        "lands silently",
        "RQ2",
    ),
    (
        "CEN-N1",
        2,
        "clean writes touching no governed type",
        "land; overhead ~ 0",
        "land",
        "RQ1",
    ),
    (
        "CEN-N2",
        2,
        "clean writes touching governed types",
        "land; latency = enforcement cost",
        "land",
        "RQ1",
    ),
    // Phase 3 — correction.
    (
        "CEN-E1",
        3,
        "write requiring escalation; scripted human approves",
        "first attempt refused with DecisionRequest; retry after Decision lands",
        "lands on first attempt",
        "RQ3",
    ),
    (
        "CEN-E2",
        3,
        "escalation the scripted human rejects",
        "retry still refused; rejection outranks approval",
        "lands on first attempt",
        "RQ3",
    ),
    (
        "CEN-R1",
        3,
        "retraction plus supersession of a placement",
        "old value closed; successor asserted; gate passes the combined post-state",
        "same",
        "RQ5",
    ),
    (
        "CEN-R2",
        3,
        "trust-plane promotion",
        "fact moves graphs bitemporally",
        "same",
        "RQ5",
    ),
    // Phase 4 — composition (read-time; identical in both arms).
    (
        "CEN-C1",
        4,
        "composed dataset including one undeclared graph",
        "fold Coverage = Partial; floored query refused",
        "same",
        "RQ4",
    ),
    (
        "CEN-C2",
        4,
        "trust pair from two different declared chains",
        "comparison error naming both chains",
        "same",
        "RQ4",
    ),
    (
        "CEN-C3",
        4,
        "label expired before query time",
        "label absent from fold; coverage degrades",
        "same",
        "RQ4",
    ),
    (
        "CEN-C4",
        4,
        "one no-export graph in an otherwise clean set",
        "composed obligations include no-export (join, not meet)",
        "same",
        "RQ4",
    ),
    (
        "CEN-C5",
        4,
        "clean compositions with full declarations",
        "pass; zero false refusals",
        "same",
        "RQ4",
    ),
    (
        "CEN-C6",
        4,
        "overlay rebind against a base it is not bound to",
        "refused; bind-once",
        "same",
        "RQ4",
    ),
    (
        "CEN-C7",
        4,
        "provincial pack imported into the census store",
        "facts re-interned; content hash stable",
        "same",
        "RQ4",
    ),
    // Phase 5 — amendment.
    (
        "CEN-M0",
        5,
        "the amendment itself: claim v1 superseded by v2; shape reloaded as v2",
        "both versioned; v1 stays answerable as-of",
        "same",
        "setup",
    ),
    (
        "CEN-M1",
        5,
        "post-amendment write valid under old Sigma only",
        "refused under amended Sigma",
        "lands silently",
        "RQ5",
    ),
    (
        "CEN-M2",
        5,
        "replay of a phase-2 verdict after the amendment",
        "evaluates under the OLD rules; fails while shapes are latest-only (forces GS6)",
        "n/a (no verdicts to replay)",
        "RQ5",
    ),
    // Phase 6 — audit (planned: quipu-4mi and the in-store audit run).
    (
        "CEN-G1",
        6,
        "tool class left ungoverned, no reason declared",
        "audit reports Violation",
        "same",
        "RQ3",
    ),
    (
        "CEN-G2",
        6,
        "tool class ungoverned with ungovernedReason",
        "audit reports Incompleteness, not violation",
        "same",
        "RQ3",
    ),
    (
        "CEN-T1",
        6,
        "trace record with no attribution",
        "counted incomplete; not placed at the attribution root",
        "same",
        "RQ3",
    ),
    (
        "CEN-X1",
        6,
        "full Sigma/T export to the external SARC checker",
        "verdict-for-verdict agreement on the shared decidable subset",
        "same",
        "RQ3",
    ),
];

fn row(
    id: &str,
) -> &'static (
    &'static str,
    u8,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
) {
    PROBES
        .iter()
        .find(|r| r.0 == id)
        .unwrap_or_else(|| panic!("probe {id} is not in the catalogue"))
}

pub fn plants(id: &str) -> String {
    row(id).2.to_string()
}

pub fn expected_gated(id: &str) -> String {
    row(id).3.to_string()
}

pub fn expected_control(id: &str) -> String {
    row(id).4.to_string()
}

/// Planned entries for exactly these probe ids (later beads' work).
pub fn planned_only(ids: &[&str]) -> Vec<ManifestEntry> {
    PROBES
        .iter()
        .filter(|r| ids.contains(&r.0))
        .map(|(id, phase, plants, eg, ec, rq)| ManifestEntry {
            id: (*id).to_string(),
            phase: *phase,
            plants: (*plants).to_string(),
            expected_gated: (*eg).to_string(),
            expected_control: (*ec).to_string(),
            scored_by: (*rq).to_string(),
            status: "planned".to_string(),
            observed: None,
            defect_subject: None,
        })
        .collect()
}
