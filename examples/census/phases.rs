//! The six Census phases.
//!
//! Phase 1 (Founding) executes against the live store; phases 2–6 register
//! their probes from the catalogue as `planned` — implementing them is bead
//! quipu-y41 (phases 2–4), quipu-tj0 (5), and quipu-4mi (6's external arm).

use quipu::store::labels::GraphLabel;
use quipu::{Datum, Op, Store, Value, lattice};

use crate::catalogue;
use crate::manifest::{Arm, ManifestEntry};
use crate::rng::SplitMix64;

/// The scripted cast and places — fixed, not sampled, so probe ids stay
/// stable across seeds; the rng varies volumes and orderings only.
pub const DISTRICTS: [&str; 3] = [
    "urn:census:graph:district-north",
    "urn:census:graph:district-south",
    "urn:census:graph:district-east",
];
pub const RECORDERS: [&str; 3] = [
    "urn:census:recorder:amaru",
    "urn:census:recorder:chaski",
    "urn:census:recorder:quilla",
];
const TRUST_CHAIN: &str = "urn:census:chain:main";

pub struct Ctx {
    pub store: Store,
    /// Unused until the phase implementations (quipu-y41) plant
    /// volume-varied probes; carried from day one so the seed discipline
    /// is part of the harness contract, not a retrofit.
    #[allow(dead_code)]
    pub rng: SplitMix64,
    /// Read by the phase implementations to decide whether gates are on.
    #[allow(dead_code)]
    pub arm: Arm,
    pub entries: Vec<ManifestEntry>,
    /// Logical minutes since the scenario epoch; every timestamp derives
    /// from this counter so no wall clock leaks into the run.
    minutes: u64,
}

impl Ctx {
    pub fn new(store: Store, rng: SplitMix64, arm: Arm) -> Self {
        Self {
            store,
            rng,
            arm,
            entries: Vec::new(),
            minutes: 0,
        }
    }

    /// The next scenario timestamp: canonical UTC, strictly increasing.
    pub fn tick(&mut self) -> String {
        self.minutes += 1;
        let (d, rem) = (self.minutes / (24 * 60), self.minutes % (24 * 60));
        format!("2026-01-{:02}T{:02}:{:02}:00Z", d + 1, rem / 60, rem % 60)
    }

    fn executed(&mut self, id: &str, phase: u8, plants: &str, expected: &str, observed: String) {
        self.entries.push(ManifestEntry {
            id: id.to_string(),
            phase,
            plants: plants.to_string(),
            expected_gated: expected.to_string(),
            expected_control: expected.to_string(),
            scored_by: "setup".to_string(),
            status: "executed".to_string(),
            observed: Some(observed),
        });
    }
}

pub fn run_all(ctx: &mut Ctx) {
    phase1_founding(ctx);
    ctx.entries.extend(catalogue::phase2_recording());
    ctx.entries.extend(catalogue::phase3_correction());
    ctx.entries.extend(catalogue::phase4_composition());
    ctx.entries.extend(catalogue::phase5_amendment());
    ctx.entries.extend(catalogue::phase6_audit());
}

/// Phase 1 — Founding: districts, recorders, trust declarations.
///
/// Real writes, executed in both arms identically (founding is the scripted
/// administrator, not a probed writer). Registers each district graph by
/// declaring its label, and each recorder as a typed, labelled entity in
/// ROOT.
fn phase1_founding(ctx: &mut Ctx) {
    let ts = ctx.tick();
    let rdf_type = ctx
        .store
        .intern(quipu::namespace::RDF_TYPE)
        .expect("intern rdf:type");
    let rdfs_label = ctx
        .store
        .intern(quipu::namespace::RDFS_LABEL)
        .expect("intern rdfs:label");
    let recorder_class = ctx
        .store
        .intern("urn:census:Recorder")
        .expect("intern Recorder");

    let mut datums = Vec::new();
    for iri in RECORDERS {
        let e = ctx.store.intern(iri).expect("intern recorder");
        let name = iri
            .rsplit(':')
            .next()
            .expect("recorder iri has a local name");
        datums.push(Datum {
            entity: e,
            attribute: rdf_type,
            value: Value::Ref(recorder_class),
            valid_from: ts.clone(),
            valid_to: None,
            op: Op::Assert,
        });
        datums.push(Datum {
            entity: e,
            attribute: rdfs_label,
            value: Value::Str(name.to_string()),
            valid_from: ts.clone(),
            valid_to: None,
            op: Op::Assert,
        });
    }
    let tx = ctx
        .store
        .transact(&datums, &ts, Some("census:founding"), Some("census"))
        .expect("founding writes land");
    ctx.executed(
        "CEN-F1",
        1,
        "recorder identities typed and labelled in ROOT",
        "facts land",
        format!("tx {tx}, {} datums", datums.len()),
    );

    // Declare each district's label: fresh, trusted within the single
    // declared chain. Ranks differ so later phases have a precedence
    // gradient to promote across.
    for (rank, iri) in DISTRICTS.iter().enumerate() {
        let ts = ctx.tick();
        ctx.store
            .graph_create(iri)
            .expect("district graph registers");
        let label = GraphLabel {
            freshness: Some(lattice::Freshness::Fresh),
            durability: None,
            trust: Some(lattice::Trust {
                iri: format!("urn:census:trust:declared-{rank}"),
                chain: TRUST_CHAIN.to_string(),
                rank: rank as i64,
            }),
            policy: None,
        };
        let tx = ctx
            .store
            .set_graph_label(iri, &label, &ts, Some("census:founding"))
            .expect("district label lands");
        ctx.executed(
            "CEN-F2",
            1,
            "district graph labelled (freshness + trust in the declared chain)",
            "label lands in the meta-graph",
            format!("{iri} tx {tx}"),
        );
    }
}
