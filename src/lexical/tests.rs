use super::*;
use crate::store::Datum;
use crate::types::Op;

const T1: &str = "2026-01-01T00:00:00Z";
const T2: &str = "2026-02-01T00:00:00Z";
const LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";

fn fact(s: &Store, entity: &str, attribute: &str, value: Value, ts: &str, op: Op) -> Datum {
    Datum {
        entity: s.intern(entity).unwrap(),
        attribute: s.intern(attribute).unwrap(),
        value,
        valid_from: ts.into(),
        valid_to: None,
        op,
    }
}

fn enabled() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.search_config_mut().keyword = true;
    s.initialize_lexical_index().unwrap();
    s
}

fn hits(s: &Store, q: &str, at: Option<&str>) -> Vec<VectorMatch> {
    s.keyword_search(q, 20, at, None).unwrap()
}

#[test]
fn disabled_is_explicit_and_read_never_creates_or_backfills() {
    let mut s = Store::open_in_memory().unwrap();
    assert!(
        s.keyword_search("word", 10, None, None)
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    assert!(s.lexical_progress().unwrap().is_none());
    s.search_config_mut().keyword = true;
    assert!(
        s.keyword_search("word", 10, None, None)
            .unwrap_err()
            .to_string()
            .contains("not ready")
    );
    assert!(s.lexical_progress().unwrap().is_none());
}

#[test]
fn all_fields_full_body_and_literal_codecs_are_immediately_searchable() {
    let mut s = enabled();
    let e = "https://example.org/ClockworkStation";
    let ty = s.intern("https://example.org/ServiceClass").unwrap();
    let body = format!("{} violet submarine tail phrase", "padding ".repeat(400));
    let rows = vec![
        fact(
            &s,
            e,
            LABEL,
            Value::Str("Silver Lighthouse".into()),
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            e,
            "http://www.w3.org/2004/02/skos/core#altLabel",
            Value::Lang {
                lexical: "phare azur".into(),
                lang: "fr".into(),
            },
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            e,
            "http://www.w3.org/2000/01/rdf-schema#comment",
            Value::Str(body),
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            e,
            "https://example.org/status",
            Value::Typed {
                lexical: "operational amber".into(),
                datatype: "https://example.org/statusType".into(),
            },
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            e,
            "https://example.org/port",
            Value::Int(8675309),
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            e,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            Value::Ref(ty),
            T1,
            Op::Assert,
        ),
    ];
    s.transact(&rows, T1, None, None).unwrap();
    for q in [
        "lighthouse",
        "azur",
        "\"violet submarine tail phrase\"",
        "amber",
        "8675309",
        "ServiceClass",
        "service class",
        "clockwork station",
    ] {
        assert_eq!(hits(&s, q, None).len(), 1, "{q}");
    }
}

#[test]
fn retraction_and_replacement_respect_the_exclusive_valid_to_boundary() {
    let mut s = enabled();
    let old = fact(
        &s,
        "https://example.org/device",
        LABEL,
        Value::Str("retiredword".into()),
        T1,
        Op::Assert,
    );
    s.transact(std::slice::from_ref(&old), T1, None, None)
        .unwrap();
    let mut retract = old;
    retract.op = Op::Retract;
    let new = fact(
        &s,
        "https://example.org/device",
        LABEL,
        Value::Str("replacementword".into()),
        T2,
        Op::Assert,
    );
    s.transact(&[retract, new], T2, None, None).unwrap();
    assert!(hits(&s, "retiredword", None).is_empty());
    assert_eq!(hits(&s, "retiredword", Some(T1)).len(), 1);
    assert!(hits(&s, "retiredword", Some(T2)).is_empty());
    assert!(hits(&s, "replacementword", Some(T1)).is_empty());
    assert_eq!(hits(&s, "replacementword", None).len(), 1);
}

#[test]
fn outer_rollback_removes_the_fresh_index_row_with_the_fact() {
    let mut s = enabled();
    let row = fact(
        &s,
        "https://example.org/device",
        LABEL,
        Value::Str("rollbackword".into()),
        T1,
        Op::Assert,
    );
    s.conn.execute_batch("SAVEPOINT rollback_probe").unwrap();
    s.transact(&[row], T1, None, None).unwrap();
    assert_eq!(hits(&s, "rollbackword", None).len(), 1);
    s.conn
        .execute_batch("ROLLBACK TO rollback_probe; RELEASE rollback_probe")
        .unwrap();
    assert!(hits(&s, "rollbackword", None).is_empty());
    assert_eq!(s.lexical_progress().unwrap().unwrap().documents, 0);
}

#[test]
fn bounded_backfill_is_resumable_and_concurrent_new_writes_are_not_lost() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("corpus.db");
    let mut s = Store::open(db.to_str().unwrap()).unwrap();
    for i in 0..7 {
        let row = fact(
            &s,
            &format!("https://example.org/item{i}"),
            LABEL,
            Value::Str("sharedneedle".into()),
            T1,
            Op::Assert,
        );
        s.transact(&[row], T1, None, None).unwrap();
    }
    s.search_config_mut().keyword = true;
    let first = s.backfill_lexical_batch(2).unwrap();
    assert!(!first.complete);
    assert!(first.cursor <= 2);
    assert!(
        s.keyword_search("sharedneedle", 10, None, None)
            .unwrap_err()
            .to_string()
            .contains("not ready")
    );
    // A separate connection writes BETWEEN batches. It has query mode off,
    // but registers the persistent triggers' decoders and preserves freshness.
    let mut other = Store::open(db.to_str().unwrap()).unwrap();
    let row = fact(
        &other,
        "https://example.org/concurrent",
        LABEL,
        Value::Str("sharedneedle".into()),
        T1,
        Op::Assert,
    );
    other.transact(&[row], T1, None, None).unwrap();
    drop(s);
    let mut s = Store::open(db.to_str().unwrap()).unwrap();
    s.search_config_mut().keyword = true;
    assert_eq!(s.lexical_progress().unwrap().unwrap().cursor, first.cursor);
    while !s.backfill_lexical_batch(2).unwrap().complete {}
    assert_eq!(hits(&s, "sharedneedle", None).len(), 8);
    assert!(s.backfill_lexical_batch(0).is_err());
    assert!(s.backfill_lexical_batch(10_001).is_err());
    s.drop_lexical_index().unwrap();
    assert!(s.lexical_progress().unwrap().is_none());
    assert_eq!(s.current_facts().unwrap().len(), 8);
}

#[test]
fn scope_filters_before_limit_and_named_graph_text_never_leaks() {
    let mut s = enabled();
    for i in 0..30 {
        let row = fact(
            &s,
            &format!("https://example.org/item{i}"),
            LABEL,
            Value::Str("commonword".into()),
            T1,
            Op::Assert,
        );
        s.transact(&[row], T1, None, None).unwrap();
    }
    let allowed = std::collections::HashSet::from(["https://example.org/item29".to_string()]);
    assert_eq!(
        s.keyword_search("commonword", 1, None, Some(&allowed))
            .unwrap()
            .len(),
        1
    );
    let overlay = s.intern("https://example.org/privateGraph").unwrap();
    let row = fact(
        &s,
        "https://example.org/private",
        LABEL,
        Value::Str("overlaysecret".into()),
        T1,
        Op::Assert,
    );
    s.transact_to_graph(&[row], T1, None, None, overlay)
        .unwrap();
    assert!(hits(&s, "overlaysecret", None).is_empty());
}

#[test]
fn api_keyword_requires_no_embedder_and_semantic_mode_keeps_its_error() {
    let mut s = enabled();
    let row = fact(
        &s,
        "https://example.org/aegis-like-id",
        LABEL,
        Value::Str("needle".into()),
        T1,
        Op::Assert,
    );
    s.transact(&[row], T1, None, None).unwrap();
    let result = crate::tool_search(
        &s,
        &serde_json::json!({"mode":"keyword","query":"aegis-like-id"}),
    )
    .unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["ranking"], "keyword");
    assert!(
        crate::tool_search(&s, &serde_json::json!({"mode":"semantic","query":"needle"})).is_err()
    );
    assert!(crate::tool_search(&s, &serde_json::json!({"mode":"bogus","query":"needle"})).is_err());
    for q in ["", "\"unclosed"] {
        assert!(s.keyword_search(q, 10, None, None).is_err());
    }
}

#[test]
fn source_snapshot_replacement_and_physical_deletion_update_index_atomically() {
    let mut s = enabled();
    let old = fact(
        &s,
        "https://example.org/device",
        LABEL,
        Value::Str("snapshotold".into()),
        T1,
        Op::Assert,
    );
    s.transact_snapshot(std::slice::from_ref(&old), T1, None, "producer", 0)
        .unwrap();
    let new = fact(
        &s,
        "https://example.org/device",
        LABEL,
        Value::Str("snapshotnew".into()),
        T2,
        Op::Assert,
    );
    let mut retired = old;
    retired.op = Op::Retract;
    s.transact_snapshot(&[retired, new], T2, None, "producer", 0)
        .unwrap();
    assert!(hits(&s, "snapshotold", None).is_empty());
    assert_eq!(hits(&s, "snapshotold", Some(T1)).len(), 1);
    assert_eq!(hits(&s, "snapshotnew", None).len(), 1);
    let before = s.lexical_progress().unwrap().unwrap().documents;
    let removed = s
        .conn
        .execute(
            "DELETE FROM facts WHERE op=1 AND g=0 AND valid_to IS NOT NULL",
            [],
        )
        .unwrap();
    assert!(removed > 0);
    assert!(hits(&s, "snapshotold", Some(T1)).is_empty());
    assert_eq!(
        s.lexical_progress().unwrap().unwrap().documents,
        before - i64::try_from(removed).unwrap()
    );
    assert_eq!(hits(&s, "snapshotnew", None).len(), 1);
}

#[test]
fn historical_keyword_type_scope_does_not_use_a_later_type() {
    let mut s = enabled();
    let ty = s.intern("https://example.org/Service").unwrap();
    let type_fact = fact(
        &s,
        "https://example.org/device",
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
        Value::Ref(ty),
        T1,
        Op::Assert,
    );
    let label = fact(
        &s,
        "https://example.org/device",
        LABEL,
        Value::Str("scopedneedle".into()),
        T1,
        Op::Assert,
    );
    s.transact(&[type_fact.clone(), label], T1, None, None)
        .unwrap();
    let mut retract = type_fact;
    retract.op = Op::Retract;
    s.transact(&[retract], T2, None, None).unwrap();
    let request = serde_json::json!({"query":"scopedneedle","mode":"keyword","entity_type":"https://example.org/Service","valid_at":T1});
    assert_eq!(crate::tool_search(&s, &request).unwrap()["count"], 1);
    let mut current = request;
    current.as_object_mut().unwrap().remove("valid_at");
    assert_eq!(crate::tool_search(&s, &current).unwrap()["count"], 0);
}

#[test]
fn activated_fts_shadow_tables_have_explicit_reconstruction_dispositions() {
    let s = enabled();
    let mut stmt = s
        .conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'lexical_%'")
        .unwrap();
    let tables = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
    let mut count = 0;
    for table in tables {
        let name = table.unwrap();
        assert!(
            crate::share_completeness::disposition(&name).is_some(),
            "{name}"
        );
        count += 1;
    }
    assert_eq!(count, 7);
}

#[test]
fn colliding_labels_remain_distinct_and_literal_metadata_is_not_text() {
    let mut s = enabled();
    let a = "https://example.org/first";
    let b = "https://example.org/second";
    let a_type = s.intern("https://example.org/a/Document").unwrap();
    let b_type = s.intern("https://example.org/b/Document").unwrap();
    let rows = vec![
        fact(
            &s,
            a,
            LABEL,
            Value::Lang {
                lexical: "sharedlabel".into(),
                lang: "en".into(),
            },
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            b,
            LABEL,
            Value::Typed {
                lexical: "sharedlabel".into(),
                datatype: "https://example.org/literalType".into(),
            },
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            a,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            Value::Ref(a_type),
            T1,
            Op::Assert,
        ),
        fact(
            &s,
            b,
            "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
            Value::Ref(b_type),
            T1,
            Op::Assert,
        ),
    ];
    s.transact(&rows, T1, None, None).unwrap();
    let out = crate::tool_search(
        &s,
        &serde_json::json!({"mode":"keyword","query":"sharedlabel","verbose":true}),
    )
    .unwrap();
    assert_eq!(out["count"], 2);
    assert_eq!(out["indexed_types"], "asserted");
    assert_eq!(out["infer_types"], false);
    let hits = out["results"].as_array().unwrap();
    assert_eq!(
        hits.iter().find(|h| h["entity"] == a).unwrap()["language"],
        "en"
    );
    assert_eq!(
        hits.iter().find(|h| h["entity"] == b).unwrap()["datatype"],
        "https://example.org/literalType"
    );
    let scoped=crate::tool_search(&s,&serde_json::json!({"mode":"keyword","query":"sharedlabel","entity_type":"https://example.org/a/Document","verbose":true})).unwrap();
    assert_eq!(scoped["count"], 1);
    assert_eq!(scoped["results"][0]["entity"], a);
    let stored: Option<String> = s
        .conn
        .query_row(
            "SELECT type_iri FROM lexical_fts WHERE type_iri=?1",
            ["https://example.org/b/Document"],
            |r| r.get(0),
        )
        .optional()
        .unwrap();
    assert_eq!(stored.as_deref(), Some("https://example.org/b/Document"));
}
