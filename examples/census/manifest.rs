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

/// One stub per RQ so the output layout is fixed from the first run; the
/// phase implementations (quipu-y41 onward) replace `pending` with numbers.
pub fn write_metric_stubs(out: &str) {
    let dir = format!("{out}/metrics");
    std::fs::create_dir_all(&dir).expect("create metrics directory");
    let rqs = [
        (
            "rq1",
            "gate cost: per-write latency, gated vs control; ungoverned overhead",
        ),
        (
            "rq2",
            "strictness value: planted defects in final graph, gated vs control",
        ),
        (
            "rq3",
            "audit: in-store T |= Sigma vs external checker, vs ground truth",
        ),
        (
            "rq4",
            "composition: widening probes refused, clean probes admitted",
        ),
        (
            "rq5",
            "replay: as-of fidelity across the amendment boundary",
        ),
    ];
    for (rq, title) in rqs {
        let body = serde_json::json!({
            "rq": rq,
            "title": title,
            "status": "pending",
            "pending_on": "quipu-y41 (phases 1-4), quipu-tj0 (RQ5), quipu-4mi (RQ3 external arm)",
        });
        let path = format!("{dir}/{rq}.json");
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&body).expect("stub serializes"),
        )
        .expect("write metric stub");
    }
}
