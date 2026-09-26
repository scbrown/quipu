#![cfg(feature = "shacl")]

use quipu::validate_shapes;

const SHAPES: &str = include_str!("../shapes/governance.ttl");

fn verdict(outcome: &str, gated_tx: Option<&str>) -> String {
    let mut data = format!(
        r#"
@prefix a: <http://aegis.gastown.local/ontology/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<urn:example:verdict> a a:Verdict ;
    a:predicateId "policy" ; a:targetRef "urn:example:target" ;
    a:outcome "{outcome}" ; a:evidenceHash "sha256:example" ;
    a:verifier "example-verifier" ; a:signature "example-signature" .
"#
    );
    if let Some(value) = gated_tx {
        data.push_str(&format!("<urn:example:verdict> a:gatedTx {value} .\n"));
    }
    data
}

#[test]
fn historical_and_refused_verdicts_need_no_transaction_link() {
    for outcome in ["satisfied", "unsatisfied", "unknown"] {
        assert!(
            validate_shapes(SHAPES, &verdict(outcome, None))
                .unwrap()
                .conforms
        );
    }
}

#[test]
fn accepts_one_positive_integer_transaction_id() {
    for value in ["1", "42", "9223372036854775807"] {
        assert!(
            validate_shapes(SHAPES, &verdict("satisfied", Some(value)))
                .unwrap()
                .conforms
        );
    }
}

#[test]
fn rejects_zero_negative_and_noninteger_transaction_ids() {
    for value in [
        "0",
        "-1",
        "1.5",
        "true",
        "\"1\"",
        "<urn:example:tx>",
        "_:tx",
    ] {
        let report = validate_shapes(SHAPES, &verdict("satisfied", Some(value))).unwrap();
        assert!(!report.conforms, "{value}: {report:?}");
    }
}

#[test]
fn rejects_ambiguous_transaction_links() {
    let report = validate_shapes(SHAPES, &verdict("satisfied", Some("1, 2"))).unwrap();
    assert!(!report.conforms, "{report:?}");
}

#[test]
fn does_not_confuse_outcome_with_commit_or_refusal() {
    // An advisory gate can commit an unsatisfied/unknown decision. SHACL must
    // not infer transaction existence from the outcome; the producer owns it.
    for outcome in ["unsatisfied", "unknown"] {
        assert!(
            validate_shapes(SHAPES, &verdict(outcome, Some("1")))
                .unwrap()
                .conforms
        );
    }
}
