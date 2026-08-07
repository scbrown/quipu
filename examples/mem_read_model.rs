//! In-memory read model prototype — the measurements behind
//! `docs/design/in-memory-read-model.md` §4.
//!
//! Builds three permutation indexes plus a two-way term dictionary over the
//! store's current ROOT facts, then answers the same 2-hop join that
//! `examples/scale_bench.rs` puts through `eval_bgp`. The point of the pairing
//! is the contrast: the SQL nested-loop join is quadratic and times out past
//! ~2,500 episodes, while the hash join here is linear and sub-millisecond.
//!
//! This is a PROTOTYPE, not a proposal to merge as-is. It answers only the
//! subset `current_facts()` covers — currently-valid asserted facts in ROOT.
//! Time travel, named graphs, overlays and attached databases are all out of
//! scope and are exactly what §5 of the design doc says must fall back to SQL.
//!
//! Run it against a store built by `scale_bench`:
//!
//! ```bash
//! cargo run --release --example scale_bench -- 10000 /tmp/q.db
//! cargo run --release --example mem_read_model -- /tmp/q.db
//! ```
//!
//! Build timings are page-cache sensitive — run twice and read the second.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use quipu::Store;
use quipu::types::Value;

/// Namespace `scale_bench` ingests under.
const NS: &str = "http://gastown.example/";

/// Resident set size in KiB, for the memory-per-fact figure.
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok()))
        })
        .unwrap_or(0)
}

/// The three access patterns `facts`' SQL indexes serve, held in memory, plus
/// the term dictionary whose per-row round-trips dominate the current binding
/// path.
struct ReadModel {
    /// entity -> (attr, value). Serves `<s> ?p ?o`.
    spo: HashMap<i64, Vec<(i64, Value)>>,
    /// attr -> (entity, value). Serves `?s <p> ?o`.
    pso: HashMap<i64, Vec<(i64, Value)>>,
    /// (attr, value-bytes) -> entities. Serves `?s <p> <o>` and `?s a <T>`.
    pos: HashMap<(i64, Vec<u8>), Vec<i64>>,
    iri_to_id: HashMap<String, i64>,
    terms: usize,
    facts: usize,
}

impl ReadModel {
    fn build(store: &Store) -> quipu::Result<Self> {
        let facts = store.current_facts()?;
        let facts_len = facts.len();

        let mut spo: HashMap<i64, Vec<(i64, Value)>> = HashMap::new();
        let mut pso: HashMap<i64, Vec<(i64, Value)>> = HashMap::new();
        let mut pos: HashMap<(i64, Vec<u8>), Vec<i64>> = HashMap::new();
        let mut iri_to_id: HashMap<String, i64> = HashMap::new();
        let mut seen: HashSet<i64> = HashSet::new();

        // One resolve() per DISTINCT term. Still one SQL statement each, which
        // is ~2/3 of build cost — a single `SELECT id, iri FROM terms` sweep is
        // Phase 1 of the design doc's plan.
        let mut intern = |store: &Store, id: i64, map: &mut HashMap<String, i64>| {
            if seen.insert(id)
                && let Ok(iri) = store.resolve(id)
            {
                map.insert(iri, id);
            }
        };

        for fact in facts {
            spo.entry(fact.entity)
                .or_default()
                .push((fact.attribute, fact.value.clone()));
            pso.entry(fact.attribute)
                .or_default()
                .push((fact.entity, fact.value.clone()));
            pos.entry((fact.attribute, fact.value.to_bytes()))
                .or_default()
                .push(fact.entity);

            intern(store, fact.entity, &mut iri_to_id);
            intern(store, fact.attribute, &mut iri_to_id);
            if let Value::Ref(target) = &fact.value {
                intern(store, *target, &mut iri_to_id);
            }
        }

        let terms = seen.len();
        Ok(Self {
            spo,
            pso,
            pos,
            iri_to_id,
            terms,
            facts: facts_len,
        })
    }

    fn id(&self, iri: &str) -> Option<i64> {
        self.iri_to_id.get(iri).copied()
    }

    /// `?d <pred> ?s . ?s a <type_iri>` as a hash join: build a set from the
    /// type side, probe it with the edge side. This is the operation `eval_bgp`
    /// currently performs as one SQL statement per accumulated row.
    fn two_hop(&self, pred: &str, type_iri: &str) -> Vec<(i64, i64)> {
        let (Some(pred_id), Some(rdf_type), Some(type_id)) = (
            self.id(pred),
            self.id(quipu::namespace::RDF_TYPE),
            self.id(type_iri),
        ) else {
            return Vec::new();
        };

        let typed: HashSet<i64> = self
            .pos
            .get(&(rdf_type, Value::Ref(type_id).to_bytes()))
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default();

        self.pso
            .get(&pred_id)
            .map(|edges| {
                edges
                    .iter()
                    .filter_map(|(subject, value)| match value {
                        Value::Ref(target) if typed.contains(target) => Some((*subject, *target)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "scale_bench.db".to_string());

    let baseline_rss = rss_kb();
    let store = Store::open(&path).expect("open store");

    let started = Instant::now();
    let model = ReadModel::build(&store).expect("build read model");
    let build_ms = started.elapsed().as_secs_f64() * 1000.0;
    let resident = rss_kb().saturating_sub(baseline_rss);

    println!("facts indexed  : {}", model.facts);
    println!("terms          : {}", model.terms);
    println!("build          : {build_ms:>9.1}ms");
    println!("resident       : {:>9.1} MB", resident as f64 / 1024.0);
    println!(
        "bytes/fact     : {:>9.0}",
        (resident as f64 * 1024.0) / model.facts.max(1) as f64
    );

    let pred = format!("{NS}targets");
    let ty = format!("{NS}Service");

    // Warm once so the reported figure is the join, not first-touch paging.
    let _ = model.two_hop(&pred, &ty);
    let started = Instant::now();
    let rows = model.two_hop(&pred, &ty);
    println!(
        "2-hop join     : {:>9.3}ms ({} rows, unlimited)",
        started.elapsed().as_secs_f64() * 1000.0,
        rows.len()
    );

    if let Some(subject) = model.id(&format!("{NS}service-1")) {
        let started = Instant::now();
        let n = model.spo.get(&subject).map_or(0, Vec::len);
        println!(
            "point lookup   : {:>9.4}ms ({n} facts)",
            started.elapsed().as_secs_f64() * 1000.0
        );
    }
}
