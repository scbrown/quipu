use super::{Evidence, Object, Request, Truth, read_evidence};
use crate::{Datum, Op, Store, Value};

const TS: &str = "2026-01-01T00:00:00Z";

fn request(subject: &str) -> Request {
    Request {
        subject: format!("urn:{subject}"),
        positive_predicate: "urn:positive".into(),
        negative_predicate: "urn:negative".into(),
        object: Object::Iri("urn:target".into()),
        judgment_policy: "urn:policy".into(),
        witness_limit: 64,
    }
}

fn put(store: &mut Store, subject: &str, predicate: &str, value: Value, source: &str) {
    let datum = Datum {
        entity: store.intern(&format!("urn:{subject}")).unwrap(),
        attribute: store.intern(&format!("urn:{predicate}")).unwrap(),
        value,
        valid_from: TS.into(),
        valid_to: None,
        op: Op::Assert,
    };
    store.transact(&[datum], TS, None, Some(source)).unwrap();
}

#[test]
fn rule_answer_states_native_snapshot_controls_removal_and_failed_reads() {
    let mut store = Store::open_in_memory().unwrap();
    let target = store.intern("urn:target").unwrap();
    for (subject, predicate) in [
        ("supported", "positive"),
        ("refuted", "negative"),
        ("both", "positive"),
        ("both", "negative"),
    ] {
        put(
            &mut store,
            subject,
            predicate,
            Value::Ref(target),
            "fixture",
        );
    }
    let before = store.latest_tx_id().unwrap();
    for (subject, expected) in [
        ("supported", Truth::Supported),
        ("refuted", Truth::Refuted),
        ("both", Truth::Both),
        ("neither", Truth::Neither),
    ] {
        let response = read_evidence(&store, &request(subject)).unwrap();
        let Evidence::Complete {
            truth, transaction, ..
        } = &response
        else {
            panic!("{response:?}");
        };
        assert_eq!(*truth, expected);
        assert_eq!(*transaction, before);
        assert_eq!(read_evidence(&store, &request(subject)).unwrap(), response);
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(serde_json::from_value::<Evidence>(wire).unwrap(), response);
    }
    assert_eq!(store.latest_tx_id().unwrap(), before);
    assert_eq!(store.lookup("urn:neither").unwrap(), None);
    store
        .retract_triples(
            store.lookup("urn:both").unwrap().unwrap(),
            store.lookup("urn:positive").unwrap(),
            Some(&Value::Ref(target)),
            TS,
            None,
            false,
            None,
        )
        .unwrap();
    assert!(matches!(
        read_evidence(&store, &request("both")).unwrap(),
        Evidence::Complete {
            truth: Truth::Refuted,
            ..
        }
    ));
    assert!(matches!(
        read_evidence(&store, &request("neither")).unwrap(),
        Evidence::Complete {
            truth: Truth::Neither,
            ..
        }
    ));
    store
        .conn
        .execute_batch("ALTER TABLE facts RENAME TO unavailable_facts")
        .unwrap();
    assert!(matches!(
        read_evidence(&store, &request("supported")).unwrap(),
        Evidence::Unknown { .. }
    ));
    assert!(matches!(
        read_evidence(&store, &request("neither")).unwrap(),
        Evidence::Unknown { .. }
    ));
}

#[test]
fn rule_answer_states_sqlite_readonly_connection_and_missing_term_are_readonly() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("answers.db");
    let path = file.to_str().unwrap();
    let mut writer = Store::open(path).unwrap();
    let target = writer.intern("urn:target").unwrap();
    put(
        &mut writer,
        "supported",
        "positive",
        Value::Ref(target),
        "fixture",
    );
    let reader = Store::open_read_only(path).unwrap();
    let before = reader.latest_tx_id().unwrap();
    assert!(matches!(
        read_evidence(&reader, &request("supported")).unwrap(),
        Evidence::Complete {
            truth: Truth::Supported,
            ..
        }
    ));
    assert!(matches!(
        read_evidence(&reader, &request("neither")).unwrap(),
        Evidence::Complete {
            truth: Truth::Neither,
            ..
        }
    ));
    assert_eq!(reader.lookup("urn:neither").unwrap(), None);
    assert_eq!(reader.latest_tx_id().unwrap(), before);
    assert!(reader.conn.execute("DELETE FROM facts", []).is_err());
}

#[test]
fn rule_answer_states_native_literal_aliases_and_cap_are_not_false_zeros() {
    let mut store = Store::open_in_memory().unwrap();
    put(
        &mut store,
        "claim",
        "positive",
        Value::Typed {
            lexical: "literal".into(),
            datatype: crate::namespace::XSD_STRING.into(),
        },
        "legacy",
    );
    put(
        &mut store,
        "claim",
        "positive",
        Value::Str("literal".into()),
        "second-owner",
    );
    let mut query = request("claim");
    query.object = Object::Str("literal".into());
    assert!(matches!(
        read_evidence(&store, &query).unwrap(),
        Evidence::Complete {
            truth: Truth::Supported,
            ..
        }
    ));
    query.witness_limit = 1;
    assert!(matches!(
        read_evidence(&store, &query).unwrap(),
        Evidence::Unknown { .. }
    ));
}

#[test]
fn rule_answer_states_oversized_provenance_is_unknown_before_materializing_it() {
    let mut store = Store::open_in_memory().unwrap();
    let target = store.intern("urn:target").unwrap();
    put(
        &mut store,
        "claim",
        "positive",
        Value::Ref(target),
        "control",
    );
    assert!(matches!(
        read_evidence(&store, &request("claim")).unwrap(),
        Evidence::Complete {
            truth: Truth::Supported,
            ..
        }
    ));
    put(
        &mut store,
        "claim",
        "positive",
        Value::Ref(target),
        &"x".repeat(16385),
    );
    assert!(matches!(
        read_evidence(&store, &request("claim")).unwrap(),
        Evidence::Unknown { .. }
    ));
}

#[test]
fn rule_answer_states_explicit_negative_is_independent_and_removal_does_not_explode() {
    // One target fixture includes both populated controls and an unrelated claim.
    let mut support = [(true, false), (false, true), (true, true), (false, false)];
    let states = |facts: &[(bool, bool)]| {
        facts
            .iter()
            .map(|&(positive, negative)| Truth::from_support(positive, negative))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        states(&support),
        [
            Truth::Supported,
            Truth::Refuted,
            Truth::Both,
            Truth::Neither
        ]
    );
    support[2].0 = false;
    assert_eq!(states(&support)[2], Truth::Refuted);
    support[2] = (true, false);
    assert_eq!(states(&support)[2], Truth::Supported);
    assert_eq!(states(&support)[3], Truth::Neither);
    for state in states(&support) {
        let wire = serde_json::to_value(state).unwrap();
        assert!(wire.is_string());
        assert_eq!(serde_json::from_value::<Truth>(wire).unwrap(), state);
    }
    assert!(serde_json::from_str::<Truth>("false").is_err());
}

#[test]
fn rule_answer_states_reject_unsupported_scope_terms_and_ambiguous_fields() {
    let request = Request {
        subject: "urn:claim".into(),
        positive_predicate: "urn:positive".into(),
        negative_predicate: "urn:negative".into(),
        object: Object::Str("same literal".into()),
        judgment_policy: "urn:policy".into(),
        witness_limit: 64,
    };
    request.validate().unwrap();
    let wire = serde_json::to_value(&request).unwrap();
    assert_eq!(
        serde_json::from_value::<Request>(wire.clone()).unwrap(),
        request
    );
    for (field, value) in [
        ("graph", serde_json::json!("urn:unsupported")),
        ("authorized", serde_json::json!(true)),
    ] {
        let mut invalid = wire.clone();
        invalid[field] = value;
        assert!(serde_json::from_value::<Request>(invalid).is_err());
    }
    let mut invalid = wire;
    invalid["object"] = serde_json::json!({"typed": {"lexical": "x", "datatype": "urn:t"}});
    assert!(serde_json::from_value::<Request>(invalid).is_err());
    let duplicate = r#"{"subject":"urn:a","subject":"urn:b","positive_predicate":"urn:p","negative_predicate":"urn:n","object":{"iri":"urn:o"},"judgment_policy":"urn:policy"}"#;
    assert!(serde_json::from_str::<Request>(duplicate).is_err());
    for limit in [0, 257, usize::MAX] {
        let mut invalid = request.clone();
        invalid.witness_limit = limit;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = request;
    invalid
        .negative_predicate
        .clone_from(&invalid.positive_predicate);
    assert!(invalid.validate().is_err());
}
