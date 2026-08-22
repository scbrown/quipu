//! Acceptance tests for the P1 event log (event-log P1). Each test maps to a
//! bead acceptance line; none assume — every claim is asserted against the
//! store.

use super::*;
use crate::episode::{Edge, Episode, Node, ingest_episode};
use crate::namespace::DEFAULT_BASE_NS;

fn ep(name: &str, nodes: Vec<Node>, edges: Vec<Edge>) -> Episode {
    Episode {
        name: name.into(),
        episode_body: Some("test body".into()),
        source: Some("test".into()),
        group_id: Some("aegis-ontology".into()),
        nodes,
        edges,
        graph: None,
        shapes: None,
        replace_snapshot: false,
    }
}

fn node(name: &str, ty: &str) -> Node {
    Node {
        name: name.into(),
        node_type: Some(ty.into()),
        description: Some(format!("{name} description")),
        properties: None,
        distinct_from: Vec::new(),
    }
}

fn edge(s: &str, rel: &str, t: &str) -> Edge {
    Edge {
        source: s.into(),
        target: t.into(),
        relation: rel.into(),
        confidence: None,
    }
}

fn all_events(store: &Store) -> Vec<EventRow> {
    store.events_after(0, 10_000, None, None).unwrap()
}

/// ACCEPTANCE: ingest -> episode.ingested + one edge.added per edge,
/// offsets strictly monotonic, `tx_id` matches the episode's tx.
#[test]
fn ingest_emits_episode_and_edges_in_commit_order() {
    let mut store = Store::open_in_memory().unwrap();
    let e = ep(
        "ev-test-1",
        vec![node("svc-a", "Service"), node("host-b", "Host")],
        vec![edge("svc-a", "runs_on", "host-b")],
    );
    let (tx_id, count) =
        ingest_episode(&mut store, &e, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
    assert!(tx_id > 0 && count > 0);

    let events = all_events(&store);
    assert!(!events.is_empty(), "ingest must emit events");

    // Offsets strictly monotonic, all stamped with the episode's tx.
    for w in events.windows(2) {
        assert!(
            w[1].offset > w[0].offset,
            "offsets must be strictly increasing"
        );
    }
    for ev in &events {
        assert_eq!(
            ev.tx_id, tx_id,
            "{}: tx_id must match the ingest tx",
            ev.event_type
        );
        assert_eq!(ev.group_id.as_deref(), Some("aegis-ontology"));
    }

    let of = |t: &str| events.iter().filter(|e| e.event_type == t).count();
    assert_eq!(of("episode.ingested"), 1);
    // Exactly one edge in the episode; wasGeneratedBy provenance links are
    // ALSO Ref-valued edges, so require >= and assert the semantic one.
    assert!(of("edge.added") >= 1);
    let runs_on = events
        .iter()
        .find(|e| e.event_type == "edge.added" && e.payload.contains("runs_on"));
    assert!(
        runs_on.is_some(),
        "the episode's runs_on edge must be an edge.added event"
    );

    // Brand-new store: both node types announced exactly once each.
    assert!(
        of("type.new") >= 2,
        "Service + Host (+ episode activity type)"
    );
    assert!(of("predicate.new") > 0);
    assert!(of("entity.added") > 0);
}

/// ACCEPTANCE (P1 spec): a brand-new type+predicate emits
/// type.new AND predicate.new ONCE; re-ingesting the same type emits NO
/// duplicate type.new; an edge on a PRE-EXISTING entity carries
/// `subject_preexisting=true`.
#[test]
fn schema_events_fire_once_and_preexisting_is_flagged() {
    let mut store = Store::open_in_memory().unwrap();
    let e1 = ep("ev-first", vec![node("svc-a", "Service")], vec![]);
    ingest_episode(&mut store, &e1, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();

    let type_new_1 = all_events(&store)
        .iter()
        .filter(|e| e.event_type == "type.new" && e.payload.contains("Service"))
        .count();
    assert_eq!(type_new_1, 1, "first sight of Service announces once");

    // Second episode: same type (no dup), a NEW predicate, an edge whose
    // subject svc-a already exists.
    let mark = store.latest_event_offset().unwrap();
    let e2 = ep(
        "ev-second",
        vec![node("svc-b", "Service"), node("host-x", "Host")],
        vec![edge("svc-a", "wired_to", "host-x")],
    );
    ingest_episode(&mut store, &e2, "2026-07-23T00:10:00Z", DEFAULT_BASE_NS).unwrap();

    let new_events = store.events_after(mark, 10_000, None, None).unwrap();
    let service_dups = new_events
        .iter()
        .filter(|e| e.event_type == "type.new" && e.payload.contains("Service"))
        .count();
    assert_eq!(
        service_dups, 0,
        "re-ingesting a known type must NOT re-announce it"
    );
    assert_eq!(
        new_events
            .iter()
            .filter(|e| e.event_type == "type.new" && e.payload.contains("Host"))
            .count(),
        1,
        "the genuinely new Host type announces once"
    );
    assert_eq!(
        new_events
            .iter()
            .filter(|e| e.event_type == "predicate.new" && e.payload.contains("wired_to"))
            .count(),
        1,
        "the new predicate announces once"
    );

    let wired = new_events
        .iter()
        .find(|e| e.event_type == "edge.added" && e.payload.contains("wired_to"))
        .expect("wired_to edge event");
    let payload: serde_json::Value = serde_json::from_str(&wired.payload).unwrap();
    assert_eq!(
        payload["subject_preexisting"], true,
        "svc-a existed before this tx — the Stiwi filter bit must be set"
    );
    assert_eq!(
        payload["object_preexisting"], false,
        "host-x is new in this tx — the mirror bit must be clear"
    );

    // And svc-a is entity.updated (existing), svc-b entity.added (new).
    let has = |t: &str, frag: &str| {
        new_events
            .iter()
            .any(|e| e.event_type == t && e.subject.as_deref().unwrap_or("").contains(frag))
    };
    assert!(has("entity.updated", "svc-a"));
    assert!(has("entity.added", "svc-b"));
}

/// ACCEPTANCE: identical re-ingest is a no-op write and emits NOTHING —
/// the log records commits, not attempts.
#[test]
fn idempotent_reingest_emits_no_events() {
    let mut store = Store::open_in_memory().unwrap();
    let e = ep("ev-idem", vec![node("svc-a", "Service")], vec![]);
    ingest_episode(&mut store, &e, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
    let before = store.latest_event_offset().unwrap();
    let (tx2, n2) =
        ingest_episode(&mut store, &e, "2026-07-23T00:20:00Z", DEFAULT_BASE_NS).unwrap();
    assert_eq!((tx2, n2), (crate::episode::NOOP_TX, 0));
    assert_eq!(
        store.latest_event_offset().unwrap(),
        before,
        "a no-op ingest must not append events"
    );
}

/// ACCEPTANCE: ?types= and ?group= filter; batches page in offset order
/// with a usable cursor.
#[test]
fn pull_filters_and_pagination() {
    let mut store = Store::open_in_memory().unwrap();
    let mut e1 = ep("ev-g1", vec![node("a", "T1")], vec![]);
    e1.group_id = Some("group-one".into());
    let mut e2 = ep("ev-g2", vec![node("b", "T2")], vec![]);
    e2.group_id = Some("group-two".into());
    ingest_episode(&mut store, &e1, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
    ingest_episode(&mut store, &e2, "2026-07-23T00:01:00Z", DEFAULT_BASE_NS).unwrap();

    // types filter
    let only_types = store
        .events_after(0, 100, Some(&["type.new".into()]), None)
        .unwrap();
    assert!(!only_types.is_empty());
    assert!(only_types.iter().all(|e| e.event_type == "type.new"));

    // group filter
    let g2 = store.events_after(0, 100, None, Some("group-two")).unwrap();
    assert!(!g2.is_empty());
    assert!(
        g2.iter()
            .all(|e| e.group_id.as_deref() == Some("group-two"))
    );

    // pagination: limit 1 pages the full log in order via the cursor
    let all = all_events(&store);
    let mut cursor = 0;
    let mut paged = Vec::new();
    loop {
        let batch = store.events_after(cursor, 1, None, None).unwrap();
        match batch.first() {
            None => break,
            Some(ev) => {
                cursor = ev.offset;
                paged.push(ev.offset);
            }
        }
    }
    assert_eq!(
        paged,
        all.iter().map(|e| e.offset).collect::<Vec<_>>(),
        "limit-1 paging must walk the identical sequence"
    );
}

/// ACCEPTANCE: commit -> resume returns events strictly AFTER the durable
/// cursor (the 'reactor down 6wk lost events' fix), and an unknown
/// consumer starts from 0 (full replay).
#[test]
fn consumer_commit_and_resume() {
    let mut store = Store::open_in_memory().unwrap();
    ingest_episode(
        &mut store,
        &ep("ev-c1", vec![node("a", "T1")], vec![]),
        "2026-07-23T00:00:00Z",
        DEFAULT_BASE_NS,
    )
    .unwrap();
    let mid = store.latest_event_offset().unwrap();
    ingest_episode(
        &mut store,
        &ep("ev-c2", vec![node("b", "T2")], vec![]),
        "2026-07-23T00:01:00Z",
        DEFAULT_BASE_NS,
    )
    .unwrap();

    assert_eq!(
        store.consumer_committed("reactor").unwrap(),
        0,
        "fresh consumer replays from 0"
    );

    store
        .commit_consumer("reactor", mid, "2026-07-23T00:02:00Z")
        .unwrap();
    assert_eq!(
        store.consumer_committed("reactor").unwrap(),
        mid,
        "cursor is durable"
    );

    let resumed = store
        .events_after(
            store.consumer_committed("reactor").unwrap(),
            100,
            None,
            None,
        )
        .unwrap();
    assert!(!resumed.is_empty());
    assert!(
        resumed.iter().all(|e| e.offset > mid),
        "resume returns only events AFTER the committed offset"
    );
    // Everything resumed belongs to the second episode's tx.
    let first_tx = resumed[0].tx_id;
    assert!(resumed.iter().all(|e| e.tx_id == first_tx));
}

/// ACCEPTANCE: the schema is additive — a store written by the pre-events
/// code path (simulated: facts present, events absent) reopens fine, keeps
/// its facts, and starts emitting from offset 1 without migration.
#[test]
fn additive_schema_reopen() {
    let dir = std::env::temp_dir().join(format!("quipu-ev-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("additive.db");
    let path_s = path.to_str().unwrap();
    let _ = std::fs::remove_file(&path);

    {
        let mut store = Store::open(path_s).unwrap();
        ingest_episode(
            &mut store,
            &ep("ev-persist", vec![node("a", "T1")], vec![]),
            "2026-07-23T00:00:00Z",
            DEFAULT_BASE_NS,
        )
        .unwrap();
    }
    {
        // Reopen: facts intact, event log intact, both APIs answer.
        let store = Store::open(path_s).unwrap();
        assert!(!store.current_facts().unwrap().is_empty());
        let events = all_events(&store);
        assert!(!events.is_empty(), "events survive reopen (durable log)");
        assert!(store.latest_event_offset().unwrap() > 0);
    }
    let _ = std::fs::remove_file(&path);
}

/// The retract paths (`retract_entity` / `retract_triples`) route through
/// `transact`, so retractions EMIT — this was mis-reported as a gap when the
/// event log first landed, and this test is the correction: claim by
/// mechanism, not by memory of the code.
#[test]
fn retraction_emits_edge_retracted() {
    let mut store = Store::open_in_memory().unwrap();
    let e = ep(
        "ev-retract",
        vec![node("svc-a", "Service"), node("host-b", "Host")],
        vec![edge("svc-a", "runs_on", "host-b")],
    );
    ingest_episode(&mut store, &e, "2026-07-23T00:00:00Z", DEFAULT_BASE_NS).unwrap();
    let mark = store.latest_event_offset().unwrap();

    // Retract everything on svc-a (label, type, description, the edge).
    let svc_a = store
        .lookup(&format!("{DEFAULT_BASE_NS}svc-a"))
        .unwrap()
        .expect("svc-a interned");
    let (tx, n) = store
        .retract_entity(svc_a, None, "2026-07-23T01:00:00Z", None)
        .unwrap();
    assert!(tx > 0 && n > 0, "retraction must be a real tx");

    let evs = store.events_after(mark, 1000, None, None).unwrap();
    assert!(!evs.is_empty(), "retraction must emit events");
    assert!(evs.iter().all(|e| e.tx_id == tx));
    let retracted: Vec<_> = evs
        .iter()
        .filter(|e| e.event_type == "edge.retracted")
        .collect();
    assert!(
        retracted.iter().any(|e| e.payload.contains("runs_on")),
        "the runs_on edge retraction must be an edge.retracted event; got {retracted:?}"
    );
    // svc-a existed before the retraction tx -> entity.updated, not .added.
    assert!(evs.iter().any(|e| e.event_type == "entity.updated"
        && e.subject.as_deref().unwrap_or("").contains("svc-a")));
    // And no phantom episode.ingested from a non-episode source.
    assert!(!evs.iter().any(|e| e.event_type == "episode.ingested"));
}

/// ACCEPTANCE (event P3): the four onViolation routing cases.
/// Shapes: HARD (unannotated -> reject) requires rdfs:label on aegis:Thing;
/// SOFT (quipu:onViolation "emit") requires aegis:size on aegis:Widget.
#[cfg(feature = "shacl")]
mod on_violation {
    use super::*;

    const SHAPES: &str = r#"@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix aegis: <http://aegis.gastown.local/ontology/> .
@prefix quipu: <http://quipu.dev/ontology/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

aegis:HardStatusShape a sh:NodeShape ;
sh:targetClass aegis:Thing ;
sh:property [ sh:path aegis:status ; sh:minCount 1 ] .

aegis:SoftSizeShape a sh:NodeShape ;
quipu:onViolation "emit" ;
sh:targetClass aegis:Widget ;
sh:property [ sh:path aegis:size ; sh:minCount 1 ] .
"#;

    fn store_with_shapes() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store
            .load_shapes("p3-test", SHAPES, "2026-07-23T00:00:00Z")
            .unwrap();
        store.shacl_config_mut().validate_on_write = true;
        store
    }

    fn shacl_events(store: &Store) -> Vec<EventRow> {
        all_events(store)
            .into_iter()
            .filter(|e| e.event_type == "shacl.violation")
            .collect()
    }

    /// An emit-shape violation: the write COMMITS and the event is in the
    /// log, inside the same tx (its `tx_id` equals the episode's).
    #[test]
    fn emit_shape_violation_commits_and_emits_event() {
        let mut store = store_with_shapes();
        // A Widget without aegis:size -> violates SoftSizeShape only.
        // (Node descriptions give every node an rdfs:comment; the episode
        // writer always emits rdfs:label from the node name, satisfying
        // HardLabelShape.)
        let episode = ep("p3-emit", vec![node("widget-1", "Widget")], vec![]);
        let (tx, count) = ingest_episode(
            &mut store,
            &episode,
            "2026-07-23T00:00:01Z",
            DEFAULT_BASE_NS,
        )
        .expect("emit-mode violation must NOT gate the write");
        assert!(tx > 0 && count > 0, "write committed");
        let evs = shacl_events(&store);
        assert_eq!(evs.len(), 1, "exactly one shacl.violation event");
        let ev = &evs[0];
        assert_eq!(ev.tx_id, tx, "event rides the episode's own tx");
        assert!(ev.subject.as_deref().unwrap_or("").contains("widget-1"));
        let payload: serde_json::Value = serde_json::from_str(&ev.payload).unwrap();
        assert_eq!(payload["mode"], "emit");
        assert!(
            payload["message"]
                .as_str()
                .unwrap_or("")
                .contains("MinCount")
        );
    }

    /// A reject-shape violation: hard error, NO write, NO event — including
    /// no shacl.violation from the emit pass (the tx never happens).
    #[test]
    fn reject_shape_violation_rejects_with_no_event_and_no_write() {
        let mut store = store_with_shapes();
        // A Thing WITHOUT aegis:status violates HardStatusShape (default
        // reject). Constructible through the writer: plain typed node, no
        // properties.
        let episode = ep("p3-reject", vec![node("thing-1", "Thing")], vec![]);
        let before = store.latest_event_offset().unwrap();
        let result = ingest_episode(
            &mut store,
            &episode,
            "2026-07-23T00:00:02Z",
            DEFAULT_BASE_NS,
        );
        assert!(result.is_err(), "reject shape must gate the write");
        // The write's own events (semantic + shacl.violation) died with the
        // rollback; the ONE event a rejected write leaves is the durable
        // `write.refused` refusal record (camayoc-0d3).
        let after: Vec<_> = store.events_after(before, 100, None, None).unwrap();
        assert_eq!(
            after
                .iter()
                .map(|e| e.event_type.as_str())
                .collect::<Vec<_>>(),
            vec!["write.refused"],
            "a rejected write leaves exactly its refusal record, nothing else"
        );
        assert!(shacl_events(&store).is_empty());
    }

    /// A conforming write with emit shapes loaded: no violation events.
    #[test]
    fn conforming_write_emits_nothing() {
        let mut store = store_with_shapes();
        let mut n = node("widget-2", "Widget");
        n.properties = Some(
            [("size".to_string(), serde_json::json!("large"))]
                .into_iter()
                .collect(),
        );
        let episode = ep("p3-clean", vec![n], vec![]);
        let (tx, _) = ingest_episode(
            &mut store,
            &episode,
            "2026-07-23T00:00:03Z",
            DEFAULT_BASE_NS,
        )
        .unwrap();
        assert!(tx > 0);
        assert!(
            shacl_events(&store).is_empty(),
            "conforming write emits no shacl events"
        );
    }

    /// Abort-path hygiene: queued advisory events from a failed write must
    /// not leak into the next successful one.
    #[test]
    fn pending_events_do_not_leak_across_writes() {
        let mut store = store_with_shapes();
        // Queue directly (as the gate would), then fail nothing — just
        // clear, then do a clean write and assert zero shacl events.
        store.queue_write_event(crate::store::PendingWriteEvent {
            event_type: "shacl.violation".into(),
            subject: Some("ghost".into()),
            payload: serde_json::json!({}),
        });
        store.clear_pending_write_events();
        let episode = ep("p3-noleak", vec![node("plain", "Widget")], vec![]);
        // plain Widget lacks size -> ONE legitimate event from THIS write;
        // the cleared ghost must not appear.
        ingest_episode(
            &mut store,
            &episode,
            "2026-07-23T00:00:04Z",
            DEFAULT_BASE_NS,
        )
        .unwrap();
        let evs = shacl_events(&store);
        assert_eq!(evs.len(), 1);
        assert!(!evs[0].subject.as_deref().unwrap_or("").contains("ghost"));
    }
}

/// ACCEPTANCE (quipu-9z9): age prunes, but a registered consumer's
/// uncommitted backlog is retained no matter how old — the committed
/// offset is never invalidated.
#[test]
fn retention_prunes_by_age_but_never_past_a_consumer() {
    let mut store = Store::open_in_memory().unwrap();
    let ing = |store: &mut Store, name: &str, entity: &str, ts: &str| {
        ingest_episode(
            store,
            &ep(name, vec![node(entity, "T")], vec![]),
            ts,
            DEFAULT_BASE_NS,
        )
        .unwrap();
    };
    ing(&mut store, "old-1", "a", "2026-01-01T00:00:00Z");
    ing(&mut store, "old-2", "b", "2026-01-02T00:00:00Z");
    ing(&mut store, "new-1", "c", "2026-08-01T00:00:00Z");
    let all = all_events(&store);

    // The consumer has committed through old-1 only; old-2 is its backlog.
    let mid = all
        .iter()
        .filter(|e| e.ts.starts_with("2026-01-01"))
        .map(|e| e.offset)
        .max()
        .unwrap();
    store
        .commit_consumer("lagger", mid, "2026-08-01T00:00:00Z")
        .unwrap();

    let deleted = store.prune_events("2026-07-01T00:00:00Z").unwrap();
    let old1_count = all
        .iter()
        .filter(|e| e.ts.starts_with("2026-01-01"))
        .count() as u64;
    assert_eq!(deleted, old1_count, "only the committed-past prefix goes");

    // The lagger's replay from its committed cursor is exactly intact:
    // old-2's events (older than the cutoff!) are still there, in order.
    let replay = store.events_after(mid, 100, None, None).unwrap();
    assert!(replay.iter().any(|e| e.ts.starts_with("2026-01-02")));
    assert!(replay.iter().any(|e| e.ts.starts_with("2026-08-01")));

    // Once the consumer commits forward, the aged backlog becomes eligible;
    // recent events survive on age.
    let latest = store.latest_event_offset().unwrap();
    store
        .commit_consumer("lagger", latest, "2026-08-01T00:00:01Z")
        .unwrap();
    let deleted2 = store.prune_events("2026-07-01T00:00:00Z").unwrap();
    assert!(deleted2 > 0);
    let remaining = all_events(&store);
    assert!(remaining.iter().all(|e| e.ts.starts_with("2026-08")));
    assert!(!remaining.is_empty(), "recent events are retained");
}

/// With no registered consumers there is no cursor to honour: age alone
/// decides (the browser-pack / no-consumers deployment).
#[test]
fn retention_with_no_consumers_prunes_by_age_alone() {
    let mut store = Store::open_in_memory().unwrap();
    ingest_episode(
        &mut store,
        &ep("old", vec![node("a", "T")], vec![]),
        "2026-01-01T00:00:00Z",
        DEFAULT_BASE_NS,
    )
    .unwrap();
    let before = all_events(&store).len();
    assert!(before > 0);
    let deleted = store.prune_events("2026-07-01T00:00:00Z").unwrap();
    assert_eq!(deleted as usize, before, "everything aged out");

    // AUTOINCREMENT: offsets continue past the pruned range, never reused.
    let pruned_max = store.latest_event_offset().unwrap(); // MAX over empty = 0
    assert_eq!(pruned_max, 0);
    ingest_episode(
        &mut store,
        &ep("new", vec![node("b", "T")], vec![]),
        "2026-08-01T00:00:00Z",
        DEFAULT_BASE_NS,
    )
    .unwrap();
    let evs = all_events(&store);
    assert!(
        evs.iter()
            .all(|e| usize::try_from(e.offset).unwrap_or(0) > before),
        "pruned offsets must never be reissued"
    );
}

/// Default behaviour is untouched: nothing prunes unless asked.
#[test]
fn retention_is_opt_in() {
    assert!(
        crate::config::EventsConfig::default()
            .retention_days
            .is_none(),
        "default must be keep-forever"
    );
}

// -- Refusal events (camayoc-0d3) -----------------------------------------

use crate::types::{Op, Value};

const RTS: &str = "2026-08-22T00:00:00Z";
const RDF_TYPE_IRI: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const DOC_TYPE: &str = "http://ex/Doc";

fn assert_datum(store: &Store, s: &str, p: &str, v: Value) -> Datum {
    Datum {
        entity: store.intern(s).unwrap(),
        attribute: store.intern(p).unwrap(),
        value: v,
        valid_from: RTS.to_string(),
        valid_to: None,
        op: Op::Assert,
    }
}

/// Define an action-boundary `deny` policy: entities of `DOC_TYPE` must carry
/// an `rdfs:label` (the `guard_tests` fixture, minimally).
fn define_require_label_policy(store: &mut Store) {
    store.governance_config_mut().enforce_on_write = true;
    let ns = DEFAULT_BASE_NS;
    let claim = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";
    let policy_class = Value::Ref(store.intern(&format!("{ns}Policy")).unwrap());
    let datums = vec![
        assert_datum(store, "http://ex/P1", RDF_TYPE_IRI, policy_class),
        assert_datum(
            store,
            "http://ex/P1",
            &format!("{ns}targets"),
            Value::Str(DOC_TYPE.into()),
        ),
        assert_datum(
            store,
            "http://ex/P1",
            &format!("{ns}claim"),
            Value::Str(claim.into()),
        ),
        assert_datum(
            store,
            "http://ex/P1",
            &format!("{ns}boundary"),
            Value::Str("action".into()),
        ),
        assert_datum(
            store,
            "http://ex/P1",
            &format!("{ns}effect"),
            Value::Str("deny".into()),
        ),
    ];
    store.transact(&datums, RTS, None, None).unwrap();
}

/// A Doc with no label — refused by the require-label deny policy.
fn unlabelled_doc(store: &Store) -> Vec<Datum> {
    let ty = Value::Ref(store.intern(DOC_TYPE).unwrap());
    vec![assert_datum(store, "http://ex/d1", RDF_TYPE_IRI, ty)]
}

fn refusal_events(store: &Store) -> Vec<EventRow> {
    store
        .events_after(0, 10_000, Some(&["write.refused".to_string()]), None)
        .unwrap()
}

/// ACCEPTANCE: a policy-refused transact leaves the graph unchanged but a
/// `write.refused` event with gate=policy exists, flows through the existing
/// `events_after` consumer surface, and is countable by gate.
#[test]
fn policy_refusal_records_refusal_event() {
    let mut store = Store::open_in_memory().unwrap();
    define_require_label_policy(&mut store);

    let bad = unlabelled_doc(&store);
    let err = store.transact(&bad, RTS, Some("tester"), Some("unit-test"));
    assert!(matches!(err, Err(crate::error::Error::PolicyDenied(_))));

    // The write itself left nothing behind...
    assert!(
        !store
            .current_facts()
            .unwrap()
            .iter()
            .any(|f| store.resolve(f.entity).unwrap() == "http://ex/d1"),
        "a refused write must leave no facts"
    );

    // ...but the refusal survives the rollback, on the events spine.
    let evs = refusal_events(&store);
    assert_eq!(evs.len(), 1, "exactly one refusal event");
    let payload: serde_json::Value = serde_json::from_str(&evs[0].payload).unwrap();
    assert_eq!(payload["gate"], "policy");
    assert_eq!(payload["graph"], crate::schema::ROOT_GRAPH_IRI);
    assert_eq!(payload["actor"], "tester");
    assert_eq!(payload["source"], "unit-test");
    assert_eq!(payload["refused_datums"], 1);
    assert!(
        payload["reason"].as_str().unwrap().contains("policy"),
        "reason must carry the gate's own terse text: {}",
        payload["reason"]
    );
    // Metadata, not bodies: the gate's reason may NAME the target, but the
    // refused datums themselves (op/valid_from/value tuples) are not stored.
    assert!(!evs[0].payload.contains("valid_from"));

    assert_eq!(
        store.refusals_by_gate().unwrap(),
        vec![("policy".to_string(), 1)]
    );
}

/// ACCEPTANCE: an authority refusal (pre-savepoint gate) records too, and the
/// count-by-gate helper aggregates across gates.
#[test]
fn authority_refusal_records_and_counts_by_gate() {
    let mut store = Store::open_in_memory().unwrap();
    define_require_label_policy(&mut store);
    // Two policy refusals...
    for _ in 0..2 {
        let bad = unlabelled_doc(&store);
        assert!(store.transact(&bad, RTS, None, None).is_err());
    }
    // ...then one authority refusal: a bound chain with no declared authority
    // intersects to nothing (fail-safe), so the write is refused.
    store.governance_config_mut().enforce_authority = true;
    store.set_principal_chain(vec!["nobody".to_string()]);
    let labelled = vec![assert_datum(
        &store,
        "http://ex/d2",
        "http://www.w3.org/2000/01/rdf-schema#label",
        Value::Str("fine".into()),
    )];
    let err = store.transact(&labelled, RTS, Some("nobody"), None);
    assert!(matches!(err, Err(crate::error::Error::PolicyDenied(_))));

    let evs = refusal_events(&store);
    assert_eq!(evs.len(), 3);
    let last: serde_json::Value = serde_json::from_str(&evs[2].payload).unwrap();
    assert_eq!(last["gate"], "authority");

    assert_eq!(
        store.refusals_by_gate().unwrap(),
        vec![("authority".to_string(), 1), ("policy".to_string(), 2)]
    );
}

/// ACCEPTANCE: a refusal inside `speculate` is NOT a real refusal — the whole
/// speculation rolls back by design — so it leaves NO refusal event.
#[test]
fn speculate_refusal_leaves_no_refusal_event() {
    let mut store = Store::open_in_memory().unwrap();
    define_require_label_policy(&mut store);

    let bad = unlabelled_doc(&store);
    let result = store.speculate(&bad, RTS, |_s| Ok(()));
    assert!(
        result.is_err(),
        "the hypothetical write is still refused to the caller"
    );

    assert!(
        refusal_events(&store).is_empty(),
        "a speculative refusal must not be recorded as a real one"
    );
}

/// ACCEPTANCE: a failure to record the refusal must not mask the original
/// refusal error. Simulated by dropping the events table out from under the
/// recording path.
#[test]
fn recording_failure_does_not_mask_the_refusal() {
    let mut store = Store::open_in_memory().unwrap();
    define_require_label_policy(&mut store);
    store.conn.execute_batch("DROP TABLE events").unwrap();

    let bad = unlabelled_doc(&store);
    let err = store.transact(&bad, RTS, None, None);
    assert!(
        matches!(err, Err(crate::error::Error::PolicyDenied(_))),
        "the caller must still see the gate's own refusal, got {err:?}"
    );
}

/// ACCEPTANCE: a SHACL-refused episode write leaves the graph unchanged but a
/// countable `write.refused` event with gate=shacl exists.
#[test]
#[cfg(feature = "shacl")]
fn shacl_refused_episode_records_refusal_event() {
    let mut store = Store::open_in_memory().unwrap();

    let shapes = concat!(
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n",
        "@prefix aegis: <http://aegis.gastown.local/ontology/> .\n",
        "aegis:WebServiceShape a sh:NodeShape ;\n",
        "    sh:targetClass aegis:WebService ;\n",
        "    sh:property [ sh:path aegis:port ; sh:minCount 1 ] .\n"
    );
    let mut episode = ep("bad-svc", vec![node("broken", "WebService")], vec![]);
    episode.shapes = Some(shapes.into());

    let err = ingest_episode(&mut store, &episode, RTS, DEFAULT_BASE_NS);
    assert!(matches!(
        err,
        Err(crate::error::Error::ValidationFailed { .. })
    ));
    assert!(
        store.current_facts().unwrap().is_empty(),
        "a refused episode leaves the graph unchanged"
    );

    let evs = refusal_events(&store);
    assert_eq!(evs.len(), 1);
    let payload: serde_json::Value = serde_json::from_str(&evs[0].payload).unwrap();
    assert_eq!(payload["gate"], "shacl");
    assert_eq!(payload["graph"], crate::schema::ROOT_GRAPH_IRI);
    assert_eq!(payload["source"], "episode:bad-svc");
    assert_eq!(payload["refused_datums"], 1);
    let reason = payload["reason"].as_str().unwrap();
    assert!(
        reason.contains("SHACL"),
        "terse reason names the gate: {reason}"
    );
    // The refused node's description/body is not stored — metadata only.
    assert!(!evs[0].payload.contains("broken description"));

    assert_eq!(
        store.refusals_by_gate().unwrap(),
        vec![("shacl".to_string(), 1)]
    );
}
