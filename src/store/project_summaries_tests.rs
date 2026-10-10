use super::*;
use crate::{
    Datum,
    types::{Op, Value},
};
const T: &str = "2026-10-10T00:00:00Z";
fn fixture() -> (Store, i64, i64) {
    let s = Store::open_in_memory().unwrap();
    let g = s.graph_create("urn:example:project").unwrap();
    let e = s.graph_create("urn:example:ephemeral").unwrap();
    s.register_project_summary_scope(g, e).unwrap();
    (s, g, e)
}
fn publish(s: &Store, g: i64) -> SummaryCoverage {
    let token = s.project_summary_coverage(g).unwrap();
    s.publish_project_summary(&token, "{\"counter_fixture\":1}", T, None)
        .unwrap();
    token
}
fn datum(s: &Store) -> Datum {
    Datum {
        entity: s.intern("urn:example:item").unwrap(),
        attribute: s.intern("urn:example:field").unwrap(),
        value: Value::Str("value".into()),
        valid_from: T.into(),
        valid_to: None,
        op: Op::Assert,
    }
}
#[test]
fn opt_in_missing_and_dirty_are_not_success() {
    let s = Store::open_in_memory().unwrap();
    assert!(s.cached_project_summary(1, T).is_err());
    let (s, g, _) = fixture();
    assert!(s.cached_project_summary(g, T).is_err());
    publish(&s, g);
    assert_eq!(
        s.cached_project_summary(g, T).unwrap().payload,
        "{\"counter_fixture\":1}"
    );
}
#[test]
fn admitted_named_and_ephemeral_writes_invalidate_in_same_transaction() {
    for ephemeral in [false, true] {
        let (mut s, g, e) = fixture();
        let token = publish(&s, g);
        let d = datum(&s);
        let tx = s
            .transact_to_graph(&[d], T, None, None, if ephemeral { e } else { g })
            .unwrap();
        assert!(tx > 0);
        assert!(s.cached_project_summary(g, T).is_err());
        assert!(s.project_summary_coverage(g).unwrap().generation > token.generation);
        assert!(s.publish_project_summary(&token, "{}", T, None).is_err());
    }
}
#[test]
fn raw_sql_mutations_invalidate_and_outer_rollback_restores() {
    let (mut s, g, _) = fixture();
    let d = datum(&s);
    s.transact_to_graph(&[d], T, None, None, g).unwrap();
    let before = publish(&s, g);
    s.conn.execute_batch("SAVEPOINT outer_fixture").unwrap();
    s.conn
        .execute(
            "UPDATE facts SET valid_to=?1 WHERE g=?2",
            params!["2026-10-11", g],
        )
        .unwrap();
    assert!(s.cached_project_summary(g, T).is_err());
    s.conn
        .execute_batch("ROLLBACK TO outer_fixture; RELEASE outer_fixture")
        .unwrap();
    assert_eq!(s.project_summary_coverage(g).unwrap(), before);
    assert!(s.cached_project_summary(g, T).is_ok());
    s.conn
        .execute("DELETE FROM facts WHERE g=?1", params![g])
        .unwrap();
    assert!(s.cached_project_summary(g, T).is_err());
}
#[test]
fn stale_reconcile_cannot_replace_concurrent_update() {
    let (s, g, _) = fixture();
    let old = publish(&s, g);
    s.conn
        .execute(
            "UPDATE graphs SET source='fixture-source' WHERE g=?1",
            params![g],
        )
        .unwrap();
    assert!(s.publish_project_summary(&old, "{}", T, None).is_err());
    assert!(s.cached_project_summary(g, T).is_err());
    publish(&s, g);
    assert!(s.cached_project_summary(g, T).is_ok());
}
#[test]
fn foreign_graph_and_noop_do_not_advance_scope() {
    let (mut s, g, _) = fixture();
    let f = s.graph_create("urn:example:foreign").unwrap();
    let before = publish(&s, g);
    let d = datum(&s);
    s.transact_to_graph(&[d], T, None, None, f).unwrap();
    s.conn
        .execute("UPDATE facts SET valid_to=NULL WHERE g=-1", [])
        .unwrap();
    assert_eq!(s.project_summary_coverage(g).unwrap(), before);
    assert!(s.cached_project_summary(g, T).is_ok());
}
#[test]
fn dropped_or_modified_trigger_and_identity_change_refuse() {
    let (s, g, _) = fixture();
    publish(&s, g);
    s.conn
        .execute_batch("DROP TRIGGER project_summary_facts_insert")
        .unwrap();
    assert!(s.cached_project_summary(g, T).is_err());
    assert!(s.register_project_summary_scope(g, 1).is_err());
    let (s, g, _) = fixture();
    publish(&s, g);
    s.conn
        .execute(
            "UPDATE store_identity SET store_id='urn:example:replacement' WHERE id=1",
            [],
        )
        .unwrap();
    assert!(s.cached_project_summary(g, T).is_err());
    assert!(s.project_summary_coverage(g).is_err());
}
#[test]
fn clock_expiry_and_oversized_payload_refuse_without_recount() {
    let (s, g, _) = fixture();
    let token = s.project_summary_coverage(g).unwrap();
    s.publish_project_summary(&token, "{}", T, Some("2026-10-10T00:01:00Z"))
        .unwrap();
    assert!(s.cached_project_summary(g, "2026-10-09T23:59:59Z").is_err());
    assert!(s.cached_project_summary(g, "2026-10-10T00:00:59Z").is_ok());
    assert!(s.cached_project_summary(g, "2026-10-10T00:01:00Z").is_err());
    assert!(
        s.publish_project_summary(&token, &" ".repeat(MAX_BYTES + 1), T, None)
            .is_err()
    );
    s.conn
        .execute(
            "UPDATE project_summary_scopes SET payload=?1",
            params![format!("{{\"large\":\"{}\"}}", "x".repeat(MAX_BYTES + 1))],
        )
        .unwrap();
    assert!(s.cached_project_summary(g, T).is_err());
}
#[test]
fn unregistered_or_overlapping_pairs_refuse_and_registration_is_idempotent() {
    let (s, g, e) = fixture();
    let before = publish(&s, g);
    s.register_project_summary_scope(g, e).unwrap();
    assert_eq!(s.project_summary_coverage(g).unwrap(), before);
    assert!(s.register_project_summary_scope(g, 9999).is_err());
    assert!(s.register_project_summary_scope(e, g).is_err());
    assert!(s.register_project_summary_scope(g, g).is_err());
}

#[test]
fn respace_invalidates_cached_scope_and_unknown_column_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("source.db");
    let dst = dir.path().join("moved.db");
    let s = Store::open(src.to_str().unwrap()).unwrap();
    let g = s.graph_create("urn:example:project").unwrap();
    let e = s.graph_create("urn:example:ephemeral").unwrap();
    s.register_project_summary_scope(g, e).unwrap();
    publish(&s, g);
    drop(s);
    crate::store::respace::respace_file(&src, &dst, 2).unwrap();
    let moved = Store::open(dst.to_str().unwrap()).unwrap();
    let moved_g = moved.lookup("urn:example:project").unwrap().unwrap();
    assert!(moved.cached_project_summary(moved_g, T).is_err());
    moved
        .conn
        .execute(
            "ALTER TABLE project_summary_scopes ADD COLUMN unknown_fixture TEXT",
            [],
        )
        .unwrap();
    drop(moved);
    assert!(crate::store::respace::respace_file(&dst, &dir.path().join("refused.db"), 3).is_err());
}

#[test]
fn deleted_graph_and_malformed_cache_are_not_reconcilable_success() {
    let (s, g, e) = fixture();
    let before = publish(&s, g);
    s.conn
        .execute("DELETE FROM graphs WHERE g=?1", params![e])
        .unwrap();
    assert!(s.project_summary_coverage(g).is_err());
    assert!(s.cached_project_summary(g, T).is_err());
    assert!(s.publish_project_summary(&before, "{}", T, None).is_err());
    let (s, g, _) = fixture();
    publish(&s, g);
    s.conn
        .execute(
            "UPDATE project_summary_scopes SET payload='broken-json'",
            [],
        )
        .unwrap();
    assert!(s.cached_project_summary(g, T).is_err());
}
