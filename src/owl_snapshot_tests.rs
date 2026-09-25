use super::*;

const TS: &str = "2026-01-01T00:00:00Z";
const LATER: &str = "2026-01-02T00:00:00Z";
const ONT: &str = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> . \
    <urn:Child> rdfs:subClassOf <urn:Parent> .";

fn seed(count: usize) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.load_ontology("test", ONT, TS).unwrap();
    let attr = store
        .intern("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
        .unwrap();
    let child = store.intern("urn:Child").unwrap();
    let data: Vec<_> = (0..count)
        .map(|n| Datum {
            entity: store.intern(&format!("urn:entity:{n}")).unwrap(),
            attribute: attr,
            value: Value::Ref(child),
            valid_from: TS.into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect();
    store.transact(&data, TS, None, None).unwrap();
    store
}

fn snapshot(store: &Store) -> Snapshot {
    let mut s = Snapshot::capture(store, Store::open_in_memory().unwrap(), TS).unwrap();
    s.derive().unwrap();
    s
}

#[test]
fn snapshot_remaps_ids_and_applies_in_bounded_batches() {
    let mut live = seed(APPLY_BATCH + 1);
    let root_before = live.current_facts().unwrap();
    let mut plan = snapshot(&live);
    assert_eq!(plan.remaining(), APPLY_BATCH + 1);
    // Allocate terms after capture: scratch ids must not be reused in live.
    live.intern("urn:concurrent-term").unwrap();
    assert_eq!(plan.apply_batch(&mut live).unwrap(), APPLY_BATCH);
    assert_eq!(plan.remaining(), 1);
    assert_eq!(plan.apply_batch(&mut live).unwrap(), 1);
    plan.finish(&mut live).unwrap();
    assert_eq!(live.current_facts().unwrap().len(), root_before.len());
    let companion = live.lookup(ROOT_INFERRED_GRAPH_IRI).unwrap().unwrap();
    let parent = live.lookup("urn:Parent").unwrap().unwrap();
    let inferred = live.current_facts_in_graph(companion).unwrap();
    assert_eq!(
        inferred
            .iter()
            .filter(|f| f.value == Value::Ref(parent))
            .count(),
        APPLY_BATCH + 1
    );
}

#[test]
fn partial_apply_never_publishes_freshness_after_a_retraction() {
    let mut live = seed(APPLY_BATCH + 1);
    let mut plan = snapshot(&live);
    assert!(plan.finish(&mut live).is_err());
    assert_eq!(plan.apply_batch(&mut live).unwrap(), APPLY_BATCH);
    let f = live.current_facts().unwrap().remove(0);
    live.transact(
        &[Datum {
            entity: f.entity,
            attribute: f.attribute,
            value: f.value,
            valid_from: LATER.into(),
            valid_to: None,
            op: Op::Retract,
        }],
        LATER,
        None,
        None,
    )
    .unwrap();
    assert!(plan.apply_batch(&mut live).is_err());
    assert!(plan.finish(&mut live).is_err());
    let companion = live.lookup(ROOT_INFERRED_GRAPH_IRI).unwrap().unwrap();
    let freshness = live.lookup(DERIVED_AS_OF_TX).unwrap();
    assert!(
        !live
            .current_facts_in_graph(companion)
            .unwrap()
            .iter()
            .any(|f| Some(f.attribute) == freshness)
    );
}

#[test]
fn freshness_is_the_snapshot_head_not_concurrent_additions() {
    let mut live = seed(1);
    let mut plan = snapshot(&live);
    let head = plan.premise_head;
    plan.apply_batch(&mut live).unwrap();
    plan.finish(&mut live).unwrap();
    assert!(live.transaction_head().unwrap() > head);
    let companion = live.lookup(ROOT_INFERRED_GRAPH_IRI).unwrap().unwrap();
    let freshness = live.lookup(DERIVED_AS_OF_TX).unwrap().unwrap();
    let values: Vec<_> = live
        .current_facts_in_graph(companion)
        .unwrap()
        .into_iter()
        .filter(|f| f.entity == companion && f.attribute == freshness)
        .map(|f| f.value)
        .collect();
    assert_eq!(values, vec![Value::Int(head)]);
}

#[test]
fn validation_uses_retraction_index_instead_of_scanning_history() {
    let live = seed(1);
    let mut stmt = live
        .conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT 1 FROM facts \
        INDEXED BY idx_retracted_tx WHERE retracted_tx > 1 AND g=0",
        )
        .unwrap();
    let details: Vec<String> = stmt
        .query_map([], |r| r.get(3))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert!(
        details
            .iter()
            .any(|s| s.contains("SEARCH facts USING INDEX idx_retracted_tx")),
        "{details:?}"
    );
}

#[test]
fn concurrent_additions_do_not_starve_snapshot_publication() {
    let mut live = seed(1);
    let mut plan = snapshot(&live);
    let mut extra = live.current_facts().unwrap()[0].clone();
    extra.entity = live.intern("urn:new-premise").unwrap();
    live.transact(
        &[Datum {
            entity: extra.entity,
            attribute: extra.attribute,
            value: extra.value,
            valid_from: LATER.into(),
            valid_to: None,
            op: Op::Assert,
        }],
        LATER,
        None,
        None,
    )
    .unwrap();
    assert!(live.transaction_head().unwrap() > plan.premise_head);
    assert_eq!(plan.apply_batch(&mut live).unwrap(), 1);
}

#[test]
fn retracted_premise_prevents_publication() {
    let mut live = seed(1);
    let mut plan = snapshot(&live);
    let f = live.current_facts().unwrap().remove(0);
    live.transact(
        &[Datum {
            entity: f.entity,
            attribute: f.attribute,
            value: f.value,
            valid_from: LATER.into(),
            valid_to: None,
            op: Op::Retract,
        }],
        LATER,
        None,
        None,
    )
    .unwrap();
    assert!(
        plan.apply_batch(&mut live)
            .unwrap_err()
            .to_string()
            .contains("retracted")
    );
    assert_eq!(plan.remaining(), 1);
}

#[test]
fn ontology_change_and_store_swap_prevent_publication() {
    let mut live = seed(1);
    let mut plan = snapshot(&live);
    assert!(plan.apply_batch(&mut seed(1)).is_err());
    live.remove_ontology("test").unwrap();
    assert!(plan.apply_batch(&mut live).is_err());
}

#[test]
fn unrelated_named_graphs_are_not_premises() {
    let mut live = seed(1);
    let graph = live.intern("urn:private").unwrap();
    let f = live.current_facts().unwrap().remove(0);
    live.transact_to_graph(
        &[Datum {
            entity: live.intern("urn:private-entity").unwrap(),
            attribute: f.attribute,
            value: f.value,
            valid_from: TS.into(),
            valid_to: None,
            op: Op::Assert,
        }],
        TS,
        None,
        None,
        graph,
    )
    .unwrap();
    assert_eq!(snapshot(&live).remaining(), 1);
}

#[test]
fn capture_works_on_an_actual_read_only_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.db");
    let source = seed(1);
    std::fs::write(&path, source.serialize_db().unwrap()).unwrap();
    let reader = Store::open_read_only(path.to_str().unwrap()).unwrap();
    assert_eq!(snapshot(&reader).remaining(), 1);
}

#[test]
fn inference_budget_rejects_before_live_publication() {
    let mut scratch = seed(2);
    scratch.ensure_owl_cache().unwrap();
    let ontology = scratch.owl_cache.as_deref().unwrap().clone();
    assert!(
        ontology
            .materialize_limited(&mut scratch, TS, 1)
            .unwrap_err()
            .to_string()
            .contains("budget exceeded")
    );
}

#[test]
#[ignore = "explicit release-mode scale rehearsal"]
fn scheduled_snapshot_scale_rehearsal() {
    let n: usize = std::env::var("OWL_BENCH_FACTS")
        .unwrap_or_else(|_| "700000".into())
        .parse()
        .unwrap();
    let derived = n.min(25_000);
    let mut source = seed(derived);
    let attr = source.intern("urn:inert-attribute").unwrap();
    let value = Value::Int(1).to_bytes();
    {
        let tx = source.conn.unchecked_transaction().unwrap();
        source
            .conn
            .execute(
                "INSERT INTO transactions(timestamp) VALUES (?1)",
                params![TS],
            )
            .unwrap();
        let head = source.conn.last_insert_rowid();
        let mut insert = source
            .conn
            .prepare("INSERT INTO facts(e,a,v,g,tx,valid_from,op) VALUES (?1,?2,?3,0,?4,?5,1)")
            .unwrap();
        for i in derived..n {
            let e = source.intern(&format!("urn:inert:{i}")).unwrap();
            insert.execute(params![e, attr, value, head, TS]).unwrap();
        }
        drop(insert);
        tx.commit().unwrap();
    }
    let started = std::time::Instant::now();
    let mut plan = Snapshot::capture(&source, Store::open_in_memory().unwrap(), TS).unwrap();
    let captured = started.elapsed();
    plan.derive().unwrap();
    let derivation = started.elapsed() - captured;
    let mut worst = std::time::Duration::ZERO;
    let mut batches = 0;
    while plan.remaining() > 0 {
        let t = std::time::Instant::now();
        assert!(plan.apply_batch(&mut source).unwrap() <= APPLY_BATCH);
        worst = worst.max(t.elapsed());
        batches += 1;
    }
    plan.finish(&mut source).unwrap();
    assert_eq!(plan.report.total, derived);
    eprintln!(
        "OWL_SCALE facts={n} derived={derived} capture_ms={} derive_ms={} batches={batches} max_apply_ms={} total_ms={}",
        captured.as_millis(),
        derivation.as_millis(),
        worst.as_millis(),
        started.elapsed().as_millis()
    );
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status
            .lines()
            .filter(|s| s.starts_with("VmHWM:") || s.starts_with("VmRSS:"))
        {
            eprintln!("OWL_SCALE {line}");
        }
    }
}

#[test]
#[ignore = "requires an explicitly supplied disposable corpus copy"]
fn scheduled_snapshot_corpus_rehearsal() {
    let path = std::env::var("OWL_REHEARSAL_COPY").expect("disposable COPY required");
    let started = std::time::Instant::now();
    let mut source = Store::open(&path).unwrap();
    eprintln!("OWL_CORPUS open_ms={}", started.elapsed().as_millis());
    let t = std::time::Instant::now();
    let mut plan = Snapshot::capture(&source, Store::open_in_memory().unwrap(), TS).unwrap();
    eprintln!(
        "OWL_CORPUS capture_ms={} head={} ontologies={}",
        t.elapsed().as_millis(),
        plan.premise_head,
        plan.ontology_count()
    );
    let t = std::time::Instant::now();
    plan.derive().unwrap();
    eprintln!(
        "OWL_CORPUS derive_ms={} proposals={}",
        t.elapsed().as_millis(),
        plan.remaining()
    );
    let mut worst = std::time::Duration::ZERO;
    let mut batches = 0;
    while plan.remaining() > 0 {
        let t = std::time::Instant::now();
        assert!(plan.apply_batch(&mut source).unwrap() <= APPLY_BATCH);
        worst = worst.max(t.elapsed());
        batches += 1;
    }
    let t = std::time::Instant::now();
    plan.finish(&mut source).unwrap();
    eprintln!(
        "OWL_CORPUS batches={batches} max_apply_ms={} finish_ms={} total_ms={}",
        worst.as_millis(),
        t.elapsed().as_millis(),
        started.elapsed().as_millis()
    );
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines().filter(|s| s.starts_with("VmHWM:")) {
            eprintln!("OWL_CORPUS {line}");
        }
    }
}
