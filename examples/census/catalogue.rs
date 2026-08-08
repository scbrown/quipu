//! The defect catalogue, transcribed from `docs/design/paper-principles.md`
//! §4 — one function per phase returning the probes the injector must plant.
//!
//! This file is the single source the phase implementations consume; if the
//! design doc's catalogue and this file disagree, that is a bug in whichever
//! one changed without the other.

use crate::manifest::ManifestEntry;

fn planned(
    id: &str,
    phase: u8,
    plants: &str,
    expected_gated: &str,
    expected_control: &str,
    scored_by: &str,
) -> ManifestEntry {
    ManifestEntry {
        id: id.to_string(),
        phase,
        plants: plants.to_string(),
        expected_gated: expected_gated.to_string(),
        expected_control: expected_control.to_string(),
        scored_by: scored_by.to_string(),
        status: "planned".to_string(),
        observed: None,
    }
}

pub fn phase2_recording() -> Vec<ManifestEntry> {
    vec![
        planned(
            "CEN-U1",
            2,
            "fact missing a required provenance tag",
            "refused; SHACL feedback names the missing property; signed deny verdict persists",
            "lands silently",
            "RQ2",
        ),
        planned(
            "CEN-A1",
            2,
            "write into a district the writer has no authority over",
            "refused; empty-intersection refusal; deny verdict",
            "lands silently",
            "RQ2",
        ),
        planned(
            "CEN-A2",
            2,
            "delegated writer exceeding the delegator's grant",
            "refused; intersection narrows, never widens",
            "lands silently",
            "RQ2",
        ),
        planned(
            "CEN-P1",
            2,
            "write violating a policy claim on post-state",
            "refused; deny verdict cites the policy IRI",
            "lands silently",
            "RQ2",
        ),
        planned(
            "CEN-P2",
            2,
            "write valid against pre-state, invalid only in post-state",
            "refused — separates post-state gating from pre-state gating",
            "lands silently",
            "RQ2",
        ),
        planned(
            "CEN-V1",
            2,
            "fact using a fabricated predicate in a policed namespace",
            "refused by the closed-world vocabulary policy in Sigma (quipu-64q)",
            "lands silently",
            "RQ2",
        ),
        planned(
            "CEN-N1",
            2,
            "clean writes touching no governed type (repeated k times)",
            "land; per-write gate overhead ~ 0",
            "land",
            "RQ1",
        ),
        planned(
            "CEN-N2",
            2,
            "clean writes touching governed types (repeated k times)",
            "land; per-write latency is the enforcement cost",
            "land",
            "RQ1",
        ),
    ]
}

pub fn phase3_correction() -> Vec<ManifestEntry> {
    vec![
        planned(
            "CEN-E1",
            3,
            "write requiring escalation; scripted human approves",
            "first attempt refused with DecisionRequest; retry after Decision lands; both verdicts persist",
            "lands silently on first attempt",
            "RQ3",
        ),
        planned(
            "CEN-E2",
            3,
            "escalation the scripted human rejects",
            "retry still refused; rejection outranks approval",
            "lands silently",
            "RQ3",
        ),
        planned(
            "CEN-R1",
            3,
            "retraction plus supersession of a phase-2 fact",
            "old fact closed via valid_to; successor asserted; history intact",
            "same (retraction is not gated)",
            "RQ5",
        ),
        planned(
            "CEN-R2",
            3,
            "trust-plane promotion",
            "fact moves graphs bitemporally and reversibly; rank facts unchanged",
            "same",
            "RQ5",
        ),
    ]
}

pub fn phase4_composition() -> Vec<ManifestEntry> {
    vec![
        planned(
            "CEN-C1",
            4,
            "composed dataset including one undeclared graph",
            "fold Coverage = Partial; floored query refused; report shows partial",
            "same (labels are read-time; arms differ only at the write gate)",
            "RQ4",
        ),
        planned(
            "CEN-C2",
            4,
            "trust pair from two different declared chains",
            "comparison error naming both chains; no silent ordering",
            "same",
            "RQ4",
        ),
        planned(
            "CEN-C3",
            4,
            "label expired before query time",
            "label absent from fold; coverage degrades",
            "same",
            "RQ4",
        ),
        planned(
            "CEN-C4",
            4,
            "one no-export graph in an otherwise clean set",
            "composed obligations include no-export (join, not meet)",
            "same",
            "RQ4",
        ),
        planned(
            "CEN-C5",
            4,
            "clean compositions with full declarations (repeated n times)",
            "pass; zero false refusals",
            "same",
            "RQ4",
        ),
        planned(
            "CEN-C6",
            4,
            "overlay attempting facts against a base it is not bound to",
            "refused; bind-once",
            "refused (bind-once is structural, not a gate)",
            "RQ4",
        ),
        planned(
            "CEN-C7",
            4,
            "provincial pack imported into the census store",
            "facts re-interned; content hash identical across both stores",
            "same",
            "RQ4",
        ),
    ]
}

pub fn phase5_amendment() -> Vec<ManifestEntry> {
    vec![
        planned(
            "CEN-M1",
            5,
            "post-amendment write valid under old Sigma only",
            "refused under amended Sigma",
            "lands silently",
            "RQ5",
        ),
        planned(
            "CEN-M2",
            5,
            "replay of a phase-2 verdict after the amendment",
            "evaluates under the OLD rules; fails while shapes are latest-only (forces GS6, quipu-krv)",
            "n/a (no verdicts to replay)",
            "RQ5",
        ),
    ]
}

pub fn phase6_audit() -> Vec<ManifestEntry> {
    vec![
        planned(
            "CEN-G1",
            6,
            "tool class left ungoverned, no reason declared",
            "audit reports Violation",
            "audit reports Violation (audit runs on both arms)",
            "RQ3",
        ),
        planned(
            "CEN-G2",
            6,
            "tool class ungoverned with ungovernedReason",
            "audit reports Incompleteness, not violation",
            "same",
            "RQ3",
        ),
        planned(
            "CEN-T1",
            6,
            "trace record with no attribution",
            "counted incomplete; not placed at the attribution root",
            "same",
            "RQ3",
        ),
        planned(
            "CEN-X1",
            6,
            "full Sigma/T export to the external SARC checker",
            "verdict-for-verdict agreement on the shared decidable subset (quipu-4mi)",
            "same",
            "RQ3",
        ),
    ]
}
