//! Seed binary — generates test-fixtures/test-store.db from static assets.
//!
//! Run with: `cargo run --bin seed-fixtures --features shacl`
//! Or via:   `just seed`

use quipu::episode::{Episode, ingest_episode};
use quipu::namespace::DEFAULT_BASE_NS;
use quipu::rdf::ingest_rdf;
use quipu::store::{Datum, Store};
use quipu::types::{Op, Value};

use oxrdfio::RdfFormat;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const DB_PATH: &str = "test-fixtures/test-store.db";
const SHAPES_PATH: &str = "test-fixtures/test-shapes.ttl";
const EPISODES_PATH: &str = "test-fixtures/test-episodes.json";
const INFRA_PATH: &str = "test-fixtures/seed-infra.ttl";
const AGENTS_PATH: &str = "test-fixtures/seed-agents.ttl";
const KNOWLEDGE_PATH: &str = "test-fixtures/seed-knowledge.ttl";

fn main() {
    if let Err(e) = run() {
        eprintln!("seed-fixtures failed: {e}");
        std::process::exit(1);
    }
}

fn run() -> quipu::Result<()> {
    // 1. Delete existing DB.
    if Path::new(DB_PATH).exists() {
        fs::remove_file(DB_PATH).expect("failed to remove existing DB");
        println!("  removed existing {DB_PATH}");
    }

    // 2. Open file-backed store.
    let mut store = Store::open(DB_PATH)?;
    println!("  opened {DB_PATH}");

    // 3. Infrastructure subgraph (~30 entities).
    let (tx, n) = ingest_ttl(
        &mut store,
        INFRA_PATH,
        "2026-01-15T10:00:00Z",
        "fixture:infra",
    )?;
    println!("  infra:      tx={tx}  triples={n}");

    // 4. Agent platform subgraph (~15 entities).
    let (tx, n) = ingest_ttl(
        &mut store,
        AGENTS_PATH,
        "2026-02-01T10:00:00Z",
        "fixture:agents",
    )?;
    println!("  agents:     tx={tx}  triples={n}");

    // 5. Knowledge subgraph (~10 entities).
    let (tx, n) = ingest_ttl(
        &mut store,
        KNOWLEDGE_PATH,
        "2026-02-15T10:00:00Z",
        "fixture:knowledge",
    )?;
    println!("  knowledge:  tx={tx}  triples={n}");

    // 6. Temporal mutations.
    temporal_mutations(&mut store)?;

    // 7. Ingest episodes from JSON.
    let episodes_json = fs::read_to_string(EPISODES_PATH)
        .unwrap_or_else(|e| panic!("failed to read {EPISODES_PATH}: {e}"));
    let episodes: Vec<Episode> = serde_json::from_str(&episodes_json)
        .unwrap_or_else(|e| panic!("failed to parse {EPISODES_PATH}: {e}"));

    let timestamps = [
        "2026-01-20T10:00:00Z",
        "2026-03-27T09:00:00Z",
        "2026-03-29T14:00:00Z",
        "2026-02-05T10:00:00Z",
        "2026-04-01T12:00:00Z",
        "2026-04-05T16:00:00Z",
        "2026-04-10T09:00:00Z",
    ];
    for (i, ep) in episodes.iter().enumerate() {
        let ts = timestamps.get(i).copied().unwrap_or("2026-04-12T00:00:00Z");
        let (tx, n) = ingest_episode(&mut store, ep, ts, DEFAULT_BASE_NS)?;
        println!("  episode[{}]: tx={tx}  triples={n}  name={}", i, ep.name);
    }

    // 8. Load SHACL shapes.
    let shapes = fs::read_to_string(SHAPES_PATH)
        .unwrap_or_else(|e| panic!("failed to read {SHAPES_PATH}: {e}"));
    store.load_shapes("test-fixtures", &shapes, "2026-04-12T00:00:00Z")?;
    println!("  shapes:     loaded from {SHAPES_PATH}");

    // 9. Layout seed metadata.
    let meta_entity = store.intern(&format!("{DEFAULT_BASE_NS}__layout_metadata"))?;
    let seed_attr = store.intern(&format!("{DEFAULT_BASE_NS}layoutSeed"))?;
    store.transact(
        &[Datum {
            entity: meta_entity,
            attribute: seed_attr,
            value: Value::Int(42),
            valid_from: "2026-04-12T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        }],
        "2026-04-12T00:00:00Z",
        Some("seed"),
        Some("fixture:metadata"),
    )?;
    println!("  metadata:   layoutSeed=42");

    // 10. Verification.
    verify(&store)?;

    println!("\nseed-fixtures complete: {DB_PATH}");
    Ok(())
}

// ── Temporal mutations ────────────────────────────────────────────

fn temporal_mutations(store: &mut Store) -> quipu::Result<()> {
    let ns = DEFAULT_BASE_NS;
    let status_attr = store.intern(&format!("{ns}status"))?;

    // koror: online (01-15) -> down (03-27T09:00)
    let koror = store
        .lookup(&format!("{ns}koror"))?
        .expect("koror not found");
    store.retract_entity(
        koror,
        Some(status_attr),
        "2026-03-27T09:00:00Z",
        Some("seed"),
    )?;
    store.transact(
        &[status_datum(
            koror,
            status_attr,
            "down",
            "2026-03-27T09:00:00Z",
        )],
        "2026-03-27T09:00:00Z",
        Some("seed"),
        Some("fixture:temporal"),
    )?;

    // koror: down -> online (03-29T14:00)
    store.retract_entity(
        koror,
        Some(status_attr),
        "2026-03-29T14:00:00Z",
        Some("seed"),
    )?;
    store.transact(
        &[status_datum(
            koror,
            status_attr,
            "online",
            "2026-03-29T14:00:00Z",
        )],
        "2026-03-29T14:00:00Z",
        Some("seed"),
        Some("fixture:temporal"),
    )?;

    // postgresql: running -> crashed (03-27T09:15)
    let pg = store
        .lookup(&format!("{ns}postgresql"))?
        .expect("postgresql not found");
    store.retract_entity(pg, Some(status_attr), "2026-03-27T09:15:00Z", Some("seed"))?;
    store.transact(
        &[status_datum(
            pg,
            status_attr,
            "crashed",
            "2026-03-27T09:15:00Z",
        )],
        "2026-03-27T09:15:00Z",
        Some("seed"),
        Some("fixture:temporal"),
    )?;

    // postgresql: crashed -> running (03-27T11:00)
    store.retract_entity(pg, Some(status_attr), "2026-03-27T11:00:00Z", Some("seed"))?;
    store.transact(
        &[status_datum(
            pg,
            status_attr,
            "running",
            "2026-03-27T11:00:00Z",
        )],
        "2026-03-27T11:00:00Z",
        Some("seed"),
        Some("fixture:temporal"),
    )?;

    // traefik: healthy -> degraded (03-27T09:05)
    let traefik = store
        .lookup(&format!("{ns}traefik"))?
        .expect("traefik not found");
    store.retract_entity(
        traefik,
        Some(status_attr),
        "2026-03-27T09:05:00Z",
        Some("seed"),
    )?;
    store.transact(
        &[status_datum(
            traefik,
            status_attr,
            "degraded",
            "2026-03-27T09:05:00Z",
        )],
        "2026-03-27T09:05:00Z",
        Some("seed"),
        Some("fixture:temporal"),
    )?;

    // traefik: degraded -> healthy (03-29T14:30)
    store.retract_entity(
        traefik,
        Some(status_attr),
        "2026-03-29T14:30:00Z",
        Some("seed"),
    )?;
    store.transact(
        &[status_datum(
            traefik,
            status_attr,
            "healthy",
            "2026-03-29T14:30:00Z",
        )],
        "2026-03-29T14:30:00Z",
        Some("seed"),
        Some("fixture:temporal"),
    )?;

    println!("  temporal:   6 mutations (koror, postgresql, traefik)");
    Ok(())
}

fn status_datum(entity: i64, attribute: i64, value: &str, valid_from: &str) -> Datum {
    Datum {
        entity,
        attribute,
        value: Value::Str(value.into()),
        valid_from: valid_from.into(),
        valid_to: None,
        op: Op::Assert,
    }
}

// ── Verification ──────────────────────────────────────────────────

fn verify(store: &Store) -> quipu::Result<()> {
    let ns = DEFAULT_BASE_NS;

    // Count entities via SPARQL.
    let result = quipu::sparql_query(
        store,
        "SELECT (COUNT(DISTINCT ?s) AS ?c) WHERE { ?s ?p ?o }",
    )?;
    let entity_count = count_from_row(result.rows().first(), "c");
    assert!(
        entity_count >= 50,
        "expected 50+ entities, found {entity_count}"
    );
    println!("\n  verify: {entity_count} distinct subjects (>= 50)");

    // koror edge count.
    let koror_id = store
        .lookup(&format!("{ns}koror"))?
        .expect("koror not found");
    let koror_facts = store.entity_facts(koror_id)?;
    let outgoing: i64 = koror_facts
        .iter()
        .filter(|f| matches!(f.value, Value::Ref(_)))
        .count()
        .try_into()
        .unwrap_or(0);

    // Count incoming edges via SPARQL.
    let incoming_q = format!("SELECT (COUNT(?s) AS ?c) WHERE {{ ?s ?p <{ns}koror> }}");
    let incoming_result = quipu::sparql_query(store, &incoming_q)?;
    let incoming = count_from_row(incoming_result.rows().first(), "c");
    let total_edges = outgoing + incoming;
    assert!(
        total_edges >= 8,
        "koror expected 8+ edges, found {total_edges} ({outgoing} out + {incoming} in)"
    );
    println!("  verify: koror has {total_edges} edges ({outgoing} out + {incoming} in) (>= 8)");

    // koror status history.
    let status_attr = store.lookup(&format!("{ns}status"))?.expect("status attr");
    let koror_history = store.attribute_history(koror_id, status_attr)?;
    assert!(
        koror_history.len() >= 5,
        "koror status history expected 5+ entries, found {}",
        koror_history.len()
    );
    println!(
        "  verify: koror status history has {} entries (>= 5)",
        koror_history.len()
    );

    // Episode transactions: check that episode provenance entities exist.
    let ep_q = "SELECT (COUNT(?e) AS ?c) WHERE { ?e a <http://www.w3.org/ns/prov#Activity> }";
    let ep_result = quipu::sparql_query(store, ep_q)?;
    let ep_count = count_from_row(ep_result.rows().first(), "c");
    assert!(
        ep_count >= 7,
        "expected 7+ episode transactions, found {ep_count}"
    );
    println!("  verify: {ep_count} episode transactions (>= 7)");

    // Shapes loaded.
    let shapes = store.list_shapes()?;
    assert!(!shapes.is_empty(), "shapes not loaded");
    println!("  verify: shapes loaded ({})", shapes.len());

    Ok(())
}

fn count_from_row(row: Option<&HashMap<String, Value>>, var: &str) -> i64 {
    row.and_then(|r| r.get(var))
        .and_then(|v| match v {
            Value::Int(n) => Some(*n),
            _ => None,
        })
        .unwrap_or(0)
}

fn ingest_ttl(
    store: &mut Store,
    path: &str,
    timestamp: &str,
    source: &str,
) -> quipu::Result<(i64, usize)> {
    let turtle = fs::read_to_string(path).unwrap_or_else(|e| panic!("failed to read {path}: {e}"));
    ingest_rdf(
        store,
        turtle.as_bytes(),
        RdfFormat::Turtle,
        None,
        timestamp,
        Some("seed"),
        Some(source),
    )
}
