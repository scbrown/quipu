use super::*;

#[test]
fn unscoped_library_calls_are_inert() {
    assert!(phase(Phase::Scan).is_none());
}

#[test]
fn scope_preserves_results_errors_and_restores_nested_state() {
    let result: Result<_, &str> = run(Endpoint::Search, Stopwatch::start(), || {
        let outer = ACTIVE.with(|active| active.borrow().clone().unwrap());
        let inner: Result<(), &str> = run(Endpoint::HybridSearch, Stopwatch::start(), || {
            let _timer = phase(Phase::Embedding).unwrap();
            Err("original error")
        });
        assert_eq!(inner, Err("original error"));
        assert!(ACTIVE.with(|active| Arc::ptr_eq(active.borrow().as_ref().unwrap(), &outer)));
        assert!(outer.lock()[Phase::Embedding as usize].is_none());
        let _timer = phase(Phase::Tool).unwrap();
        Ok(42)
    });
    assert_eq!(result, Ok(42));
    assert!(phase(Phase::Tool).is_none());
}

#[test]
fn panic_restores_scope() {
    let result = std::panic::catch_unwind(|| {
        run::<(), ()>(Endpoint::Search, Stopwatch::start(), || panic!("probe"))
    });
    assert!(result.is_err());
    assert!(phase(Phase::Scan).is_none());
}

#[test]
fn records_have_fixed_fields_and_unknown_phases_stay_null() {
    let state = Arc::new(Mutex::new([None; PHASES.len()]));
    state.lock()[Phase::Scan as usize] = Some(230);
    let value = record(Endpoint::Search, 701, 470, &state, None, false);
    assert_eq!(value["admission_dispatch_ms"], 470);
    assert_eq!(value["phase_ms"]["sqlite_scan_decode_score"], 230);
    assert!(value["phase_ms"]["embedding"].is_null());
    assert!(value["thread_physical_read_bytes"].is_null());
    assert_eq!(value["thread_io_known"], false);
    assert_eq!(value["ok"], false);
    assert_eq!(value.as_object().unwrap().len(), 10);
    assert_eq!(value["phase_ms"].as_object().unwrap().len(), 6);
}

#[test]
fn thread_io_requires_both_counters_and_monotonic_deltas() {
    assert!(ThreadIo::parse("rchar: 123\n").is_none());
    assert!(ThreadIo::parse("rchar: broken\nread_bytes: 3\n").is_none());
    let before = ThreadIo::parse("rchar: 123\nread_bytes: 3\n").unwrap();
    let after = ThreadIo::parse("rchar: 130\nread_bytes: 9\n").unwrap();
    let delta = after.delta(before).unwrap();
    assert_eq!((delta.logical, delta.physical), (7, 6));
    assert!(before.delta(after).is_none());
}

#[test]
fn scoped_vector_search_preserves_ranking_and_records_only_executed_phases() {
    use crate::vector::KnowledgeVectorStore;
    let store = crate::Store::open_in_memory().unwrap();
    let entity = store.intern("http://example.org/control").unwrap();
    store
        .embed_entity(entity, "control text", &[1.0, 0.0], "2026-01-01")
        .unwrap();
    let plain = store.vector_search(&[1.0, 0.0], 3, None).unwrap();
    let traced = run::<_, crate::Error>(Endpoint::Search, Stopwatch::start(), || {
        let result = store.vector_search(&[1.0, 0.0], 3, None)?;
        let state = ACTIVE.with(|active| active.borrow().clone().unwrap());
        let values = state.lock();
        for phase in [Phase::Scan, Phase::Sort, Phase::Metadata] {
            assert!(values[phase as usize].is_some());
        }
        assert!(values[Phase::Embedding as usize].is_none());
        Ok(result)
    })
    .unwrap();
    let fields = |matches: Vec<crate::vector::VectorMatch>| {
        matches
            .into_iter()
            .map(|m| (m.entity_id, m.text, m.score, m.valid_from, m.valid_to))
            .collect::<Vec<_>>()
    };
    assert_eq!(fields(plain), fields(traced));
    run::<(), ()>(Endpoint::Search, Stopwatch::start(), || {
        assert!(store.vector_search(&[1.0], 3, None).is_err());
        let state = ACTIVE.with(|active| active.borrow().clone().unwrap());
        let values = state.lock();
        assert!(values[Phase::Scan as usize].is_some());
        assert!(values[Phase::Sort as usize].is_none());
        assert!(values[Phase::Metadata as usize].is_none());
        Ok(())
    })
    .unwrap();
}
