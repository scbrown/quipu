//! The Census ground-truth manifest.
//!
//! The injector emits one entry per probe with its planted ground truth;
//! every RQ scorer reads THIS file, never the script — the separation that
//! keeps scoring honest (`docs/design/paper-principles.md` §4, "Totals
//! discipline").

use serde::Serialize;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    Gated,
    Control,
}

impl Arm {
    pub fn as_str(self) -> &'static str {
        match self {
            Arm::Gated => "gated",
            Arm::Control => "control",
        }
    }
}

#[derive(Serialize)]
pub struct RunInfo {
    pub seed: u64,
    pub arm: String,
    pub harness: String,
}

#[derive(Serialize)]
pub struct ManifestEntry {
    /// Probe id from the defect catalogue (e.g. `CEN-P2`), suffixed when a
    /// probe repeats (`CEN-N1.3`).
    pub id: String,
    /// Census phase 1-6.
    pub phase: u8,
    /// What the injector planted, in one sentence.
    pub plants: String,
    /// Ground truth for the gated arm.
    pub expected_gated: String,
    /// Ground truth for the control arm.
    pub expected_control: String,
    /// Which research question scores this probe (e.g. `RQ2`).
    pub scored_by: String,
    /// `executed` (skeleton ran it) or `planned` (quipu-y41 and later).
    pub status: String,
    /// Observed outcome when executed; absent while planned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<String>,
    /// The subject IRI the RQ2 scorer asks the final store about — the
    /// scorer reads THIS, never the phase scripts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defect_subject: Option<String>,
}

#[derive(Serialize)]
pub struct Manifest {
    pub run: RunInfo,
    pub entries: Vec<ManifestEntry>,
}

impl Manifest {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifest serializes")
    }
}
