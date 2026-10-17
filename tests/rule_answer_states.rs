#![cfg(feature = "shacl")]
//! Real CLI wire behavior, paired positive/negative controls in one fixture.

use quipu::{
    Store,
    rule_answers::{Object, Request, query},
    tool_knot,
};
use serde_json::json;

#[test]
fn rule_answer_states_actual_cli_matches_native_and_preserves_ask() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("answers.db");
    let path = file.to_str().unwrap();
    let mut store = Store::open(path).unwrap();
    store
        .load_shapes(
            "rule-answers",
            include_str!("../shapes/rule-answers.ttl"),
            "2026-01-01",
        )
        .unwrap();
    tool_knot(&mut store, &json!({"turtle":
        "<urn:supported> <urn:positive> <urn:target> . <urn:refuted> <urn:negative> <urn:target> . <urn:both> <urn:positive> <urn:target> ; <urn:negative> <urn:target> ."
    })).unwrap();
    let before = store.latest_tx_id().unwrap();
    for (subject, positive, negative) in [
        ("supported", true, false),
        ("refuted", false, true),
        ("both", true, true),
        ("neither", false, false),
    ] {
        let request = Request {
            subject: format!("urn:{subject}"),
            positive_predicate: "urn:positive".into(),
            negative_predicate: "urn:negative".into(),
            object: Object::Iri("urn:target".into()),
            judgment_policy: "urn:policy".into(),
            witness_limit: 64,
        };
        let request_file = directory.path().join("request.json");
        std::fs::write(&request_file, serde_json::to_vec(&request).unwrap()).unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new(env!("CARGO_BIN_EXE_quipu"))
                .args(args)
                .args(["--db", path])
                .env_clear()
                .env("HOME", directory.path())
                .env("PATH", "/usr/bin:/bin")
                .current_dir(directory.path())
                .output()
                .unwrap()
        };
        let result = run(&["rule-query", request_file.to_str().unwrap()]);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let wire: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            wire,
            serde_json::to_value(query(&store, &request).unwrap()).unwrap()
        );
        assert_eq!(wire["action_authority"], "not_granted");
        assert_eq!(wire["claim_context"]["claim"]["subject"], request.subject);
        assert_eq!(
            wire["claim_context"]["judgment_policy"],
            request.judgment_policy
        );
        for (side, expected) in [("positive", positive), ("negative", negative)] {
            let sparql = format!("ASK {{ <urn:{subject}> <urn:{side}> <urn:target> }}");
            let result = run(&["read", &sparql]);
            assert!(result.status.success());
            assert_eq!(
                String::from_utf8(result.stdout).unwrap().trim(),
                expected.to_string()
            );
        }
    }
    assert_eq!(store.latest_tx_id().unwrap(), before);
    assert_eq!(store.lookup("urn:neither").unwrap(), None);
}
