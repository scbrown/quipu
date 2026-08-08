//! The Census context, the Founding phase, and Σ.
//!
//! Phases 2–4 execute in `phase2.rs`/`phase3.rs`/`phase4.rs` (bead
//! quipu-y41); phases 5–6 remain `planned` in the manifest for quipu-krv /
//! quipu-tj0 / quipu-4mi.

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
const AEGIS: &str = "http://aegis.gastown.local/ontology/";
pub const A_TARGETS: &str = "http://aegis.gastown.local/ontology/targets";
pub const A_CLAIM: &str = "http://aegis.gastown.local/ontology/claim";
pub const ROOT_IRI: &str = "urn:quipu:graph:root";

/// Interned district graph ids and the Σ policy IRIs the probes cite.
pub struct CensusIris {
    pub district_g: [i64; 3],
}

pub struct Ctx {
    pub store: Store,
    pub rng: SplitMix64,
    pub arm: Arm,
    pub entries: Vec<ManifestEntry>,
    /// Path of the store's database file (unpack operates on paths).
    pub db_path: String,
    /// Per-write latencies (µs) for RQ1: writes touching no governed type.
    pub lat_ungoverned: Vec<u128>,
    /// Per-write latencies (µs) for RQ1: compliant governed writes.
    pub lat_governed: Vec<u128>,
    /// Logical minutes since the scenario epoch; every timestamp derives
    /// from this counter so no wall clock leaks into the run.
    minutes: u64,
}

impl Ctx {
    pub fn new(store: Store, rng: SplitMix64, arm: Arm, db_path: String) -> Self {
        Self {
            store,
            rng,
            arm,
            entries: Vec::new(),
            db_path,
            lat_ungoverned: Vec::new(),
            lat_governed: Vec::new(),
            minutes: 0,
        }
    }

    pub fn gated(&self) -> bool {
        self.arm == Arm::Gated
    }

    pub fn store_db_path(&self) -> &str {
        &self.db_path
    }

    /// Record a composition probe: `correct` is whether the observed
    /// behavior matched the lattice's contract (refusal where widening was
    /// attempted, admission where the composition was clean).
    pub fn probe_refusal(&mut self, id: &str, observed: &str, correct: bool) {
        let obs = format!("{observed} (contract_upheld={correct})");
        self.push_entry(id, 4, &catalogue::plants(id), None, &obs, "RQ4");
    }

    /// The next scenario timestamp: canonical UTC, strictly increasing.
    pub fn tick(&mut self) -> String {
        self.minutes += 1;
        let (d, rem) = (self.minutes / (24 * 60), self.minutes % (24 * 60));
        format!("2026-01-{:02}T{:02}:{:02}:00Z", d + 1, rem / 60, rem % 60)
    }

    /// Record an executed probe with no defect subject (setup, clean bulk).
    pub fn probe(&mut self, id: &str, phase: u8, plants: &str, observed: &str, scored_by: &str) {
        self.push_entry(id, phase, plants, None, observed, scored_by);
    }

    /// Record an executed defect probe: `subject` is what the RQ2 scorer
    /// asks the final store about, `landed` is what this arm observed.
    pub fn probe_defect(
        &mut self,
        id: &str,
        phase: u8,
        subject: &str,
        observed: &str,
        landed: bool,
        scored_by: &str,
    ) {
        let expected = catalogue_expectation(id);
        let obs = format!("{observed} (landed={landed})");
        self.push_entry(
            id,
            phase,
            &expected,
            Some(subject.to_string()),
            &obs,
            scored_by,
        );
    }

    fn push_entry(
        &mut self,
        id: &str,
        phase: u8,
        plants: &str,
        subject: Option<String>,
        observed: &str,
        scored_by: &str,
    ) {
        self.entries.push(ManifestEntry {
            id: id.to_string(),
            phase,
            plants: plants.to_string(),
            expected_gated: catalogue::expected_gated(id),
            expected_control: catalogue::expected_control(id),
            scored_by: scored_by.to_string(),
            status: "executed".to_string(),
            observed: Some(observed.to_string()),
            defect_subject: subject,
        });
    }
}

fn catalogue_expectation(id: &str) -> String {
    catalogue::plants(id)
}

pub fn run_all(ctx: &mut Ctx, out_dir: &str) {
    let iris = phase1_founding(ctx, out_dir);
    crate::phase2::run(ctx, &iris);
    crate::phase3::run(ctx, &iris);
    crate::phase4::run(ctx, &iris, out_dir);
    ctx.entries.extend(catalogue::phase5_amendment());
    ctx.entries.extend(catalogue::phase6_audit());
}

// ---------------------------------------------------------------------------
// Founding
// ---------------------------------------------------------------------------

pub fn assert_datum(store: &Store, s: &str, p: &str, v: Value, ts: &str) -> Datum {
    Datum {
        entity: store.intern(s).expect("intern subject"),
        attribute: store.intern(p).expect("intern predicate"),
        value: v,
        valid_from: ts.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

pub fn retract_datum(store: &Store, s: &str, p: &str, v: Value, ts: &str) -> Datum {
    Datum {
        op: Op::Retract,
        ..assert_datum(store, s, p, v, ts)
    }
}

pub fn type_ref(store: &Store, type_iri: &str) -> Value {
    Value::Ref(store.intern(type_iri).expect("intern type"))
}

pub fn has_facts(store: &Store, subject: &str) -> bool {
    let q = format!("ASK {{ GRAPH ?g {{ <{subject}> ?p ?o }} }}");
    matches!(
        quipu::sparql::query(store, &q),
        Ok(quipu::sparql::QueryResult::Ask(true))
    ) || matches!(
        quipu::sparql::query(store, &format!("ASK {{ <{subject}> ?p ?o }}")),
        Ok(quipu::sparql::QueryResult::Ask(true))
    )
}

/// Whether `subject` carries `predicate` = `value` (string) in any graph.
pub fn value_present(store: &Store, subject: &str, predicate: &str, value: &str) -> bool {
    for q in [
        format!("ASK {{ GRAPH ?g {{ <{subject}> <{predicate}> \"{value}\" }} }}"),
        format!("ASK {{ <{subject}> <{predicate}> \"{value}\" }}"),
    ] {
        if matches!(
            quipu::sparql::query(store, &q),
            Ok(quipu::sparql::QueryResult::Ask(true))
        ) {
            return true;
        }
    }
    false
}

/// Phase 1 — Founding: districts, recorders, authority, shapes, Σ.
///
/// The founding administrator is trusted: it writes with every gate off,
/// then — in the gated arm only — switches the gates on for phases 2+.
fn phase1_founding(ctx: &mut Ctx, out_dir: &str) -> CensusIris {
    let ts = ctx.tick();

    // Recorders, typed and labelled, in ROOT.
    let mut datums = Vec::new();
    for iri in RECORDERS {
        let name = iri.rsplit(':').next().expect("recorder local name");
        datums.push(assert_datum(
            &ctx.store,
            iri,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:Recorder"),
            &ts,
        ));
        datums.push(assert_datum(
            &ctx.store,
            iri,
            quipu::namespace::RDFS_LABEL,
            Value::Str(name.to_string()),
            &ts,
        ));
    }
    let tx = ctx
        .store
        .transact(&datums, &ts, Some("census:founding"), Some("census"))
        .expect("founding writes land");
    ctx.probe(
        "CEN-F1",
        1,
        "recorder identities typed and labelled in ROOT",
        &format!("tx {tx}, {} datums", datums.len()),
        "setup",
    );

    // District graphs: registered and labelled (fresh + ranked trust).
    let mut district_g = [0i64; 3];
    for (rank, iri) in DISTRICTS.iter().enumerate() {
        let ts = ctx.tick();
        district_g[rank] = ctx
            .store
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
        ctx.probe(
            "CEN-F2",
            1,
            "district graph labelled (freshness + ranked trust)",
            &format!("{iri} tx {tx}"),
            "setup",
        );
    }

    found_authority(ctx);
    found_sigma(ctx);
    found_shapes(ctx);

    // Arm split: the control arm keeps every gate off.
    if ctx.gated() {
        ctx.store.governance_config_mut().enforce_on_write = true;
        ctx.store.governance_config_mut().enforce_authority = true;
        ctx.store.shacl_config_mut().validate_on_write = true;
        let key = std::path::Path::new(out_dir).join("census-signing.pk8");
        let identity = quipu::signing::SigningIdentity::load(&key, "urn:census:verifier:keeper")
            .expect("signing identity loads");
        ctx.store
            .set_signing_identity(std::sync::Arc::new(identity));
        ctx.probe(
            "CEN-F5",
            1,
            "gates on: policy, authority, SHACL, signed verdicts",
            "gated arm configured",
            "setup",
        );
    } else {
        ctx.probe(
            "CEN-F5",
            1,
            "gates left off",
            "control arm configured",
            "setup",
        );
    }

    CensusIris { district_g }
}

/// Principals: amaru → {ROOT, north}; chaski → {ROOT, south, east};
/// scribe (delegate) → {ROOT, north, south}; keeper → wildcard.
fn found_authority(ctx: &mut Ctx) {
    let ts = ctx.tick();
    let grants: [(&str, &[&str]); 4] = [
        ("amaru", &[ROOT_IRI, DISTRICTS[0]]),
        ("chaski", &[ROOT_IRI, DISTRICTS[1], DISTRICTS[2]]),
        ("scribe", &[ROOT_IRI, DISTRICTS[0], DISTRICTS[1]]),
        ("keeper", &["*"]),
    ];
    let mut datums = Vec::new();
    for (id, graphs) in grants {
        let iri = format!("urn:census:principal:{id}");
        datums.push(assert_datum(
            &ctx.store,
            &iri,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, &format!("{AEGIS}Principal")),
            &ts,
        ));
        datums.push(assert_datum(
            &ctx.store,
            &iri,
            &format!("{AEGIS}principalId"),
            Value::Str(id.to_string()),
            &ts,
        ));
        for g in graphs {
            datums.push(assert_datum(
                &ctx.store,
                &iri,
                &format!("{AEGIS}authorityOver"),
                Value::Str((*g).to_string()),
                &ts,
            ));
        }
    }
    let tx = ctx
        .store
        .transact(&datums, &ts, Some("census:founding"), Some("census"))
        .expect("authority grants land");
    ctx.probe(
        "CEN-F3",
        1,
        "principal authority grants (delegation narrows; keeper holds wildcard)",
        &format!("tx {tx}"),
        "setup",
    );
}

/// Σ — the constraint set the gated arm enforces. Written identically in
/// BOTH arms: Σ is data; only enforcement differs between arms.
fn found_sigma(ctx: &mut Ctx) {
    let ts = ctx.tick();
    // Claims wrap their patterns in GRAPH ?g: census records land in
    // district graphs, and a plain BGP sees only the default graph — a
    // claim written without GRAPH would judge graph-scoped writes against
    // an empty view (measured here; the paper's §5 gets a sentence on it).
    let policies: [(&str, &str, &str, &str, Option<i64>); 4] = [
        (
            "urn:census:policy:tally-label",
            "urn:census:Tally",
            "ASK { GRAPH ?g { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l } }",
            "deny",
            None,
        ),
        (
            "urn:census:policy:single-placement",
            "urn:census:Resident",
            "ASK { FILTER NOT EXISTS { GRAPH ?g { \
             $target <urn:census:vocab:placedIn> ?a . \
             $target <urn:census:vocab:placedIn> ?b . FILTER(?a != ?b) } } }",
            "deny",
            None,
        ),
        // The closed-world vocabulary policy (bead quipu-64q): every
        // predicate on a Record must be declared. Open-world SHACL cannot
        // say this; an ASK claim over the post-state can. The declared
        // vocabulary lives in ROOT (default graph); the record lives in a
        // district graph.
        (
            "urn:census:policy:closed-vocabulary",
            "urn:census:Record",
            "ASK { FILTER NOT EXISTS { GRAPH ?g { $target ?p ?o } \
             FILTER NOT EXISTS { \
             ?p <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> \
             <urn:census:DeclaredPredicate> } } }",
            "deny",
            None,
        ),
        (
            "urn:census:policy:annex-approval",
            "urn:census:Annex",
            "ASK { GRAPH ?g { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l } }",
            "require-approval",
            Some(600),
        ),
    ];
    let mut datums = Vec::new();
    for (iri, target, claim, effect, window) in policies {
        datums.push(assert_datum(
            &ctx.store,
            iri,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, &format!("{AEGIS}Policy")),
            &ts,
        ));
        datums.push(assert_datum(
            &ctx.store,
            iri,
            A_TARGETS,
            Value::Str(target.to_string()),
            &ts,
        ));
        datums.push(assert_datum(
            &ctx.store,
            iri,
            A_CLAIM,
            Value::Str(claim.to_string()),
            &ts,
        ));
        datums.push(assert_datum(
            &ctx.store,
            iri,
            &format!("{AEGIS}boundary"),
            Value::Str("action".to_string()),
            &ts,
        ));
        datums.push(assert_datum(
            &ctx.store,
            iri,
            &format!("{AEGIS}effect"),
            Value::Str(effect.to_string()),
            &ts,
        ));
        if let Some(w) = window {
            datums.push(assert_datum(
                &ctx.store,
                iri,
                &format!("{AEGIS}reversibilityWindowSeconds"),
                Value::Int(w),
                &ts,
            ));
        }
    }
    // The closed world itself: the declared census vocabulary. rdf:type and
    // rdfs:label are declared too — the policed set is everything a Record
    // may carry.
    for p in [
        quipu::namespace::RDF_TYPE,
        quipu::namespace::RDFS_LABEL,
        "urn:census:vocab:recordedBy",
        "urn:census:vocab:households",
        "urn:census:vocab:notes",
    ] {
        datums.push(assert_datum(
            &ctx.store,
            p,
            quipu::namespace::RDF_TYPE,
            type_ref(&ctx.store, "urn:census:DeclaredPredicate"),
            &ts,
        ));
    }
    let tx = ctx
        .store
        .transact(&datums, &ts, Some("census:founding"), Some("census"))
        .expect("sigma lands");
    ctx.probe("CEN-F4", 1, "Sigma: 4 policies (label, single-placement, closed-vocabulary, escalation) + declared vocabulary", &format!("tx {tx}"), "setup");
}

/// The SHACL shape CEN-U1 exercises on the episode ingress path.
fn found_shapes(ctx: &mut Ctx) {
    let ts = ctx.tick();
    let shape = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix census: <urn:census:> .
@prefix vocab: <urn:census:vocab:> .

census:RecordShape a sh:NodeShape ;
    sh:targetClass census:Record ;
    sh:property [
        sh:path vocab:recordedBy ;
        sh:minCount 1 ;
        sh:message "every census record must carry vocab:recordedBy" ;
    ] .
"#;
    ctx.store
        .load_shapes("census-record", shape, &ts)
        .expect("shapes load");
    ctx.probe(
        "CEN-F6",
        1,
        "SHACL shape: census:Record requires vocab:recordedBy",
        "loaded",
        "setup",
    );
}
