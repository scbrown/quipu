#![cfg(feature = "shacl")]

use quipu::{store::Store, tool_knot, validate_shapes};
use serde_json::json;

const SHAPES: &str = include_str!("../shapes/demoted-derivation.ttl");
const PREFIXES: &str = r#"
@prefix q: <http://quipu.local/graph#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
"#;
const FIELDS: &[(&str, &str)] = &[
    ("q:derivationState", "\"unsupported\""),
    ("rdf:subject", "<urn:example:subject>"),
    ("rdf:predicate", "<urn:example:predicate>"),
    ("rdf:object", "\"retained object\""),
    ("q:premiseGraph", "<urn:example:premises>"),
    ("q:deriverSource", "\"reasoner:example\""),
    ("q:promotionTx", "1"),
    ("q:invalidationTx", "2"),
    (
        "prov:invalidatedAtTime",
        "\"2026-01-01T00:00:00Z\"^^xsd:dateTime",
    ),
];

fn record(replace: Option<(&str, &str)>) -> String {
    let mut data = format!("{PREFIXES}\n<urn:example:record> a q:DemotedDerivation .\n");
    for &(predicate, original) in FIELDS {
        let value = replace
            .filter(|(field, _)| *field == predicate)
            .map_or(original, |(_, value)| value);
        if !value.is_empty() {
            data.push_str(&format!("<urn:example:record> {predicate} {value} .\n"));
        }
    }
    data
}

fn assert_valid(data: &str, expected: bool) {
    let report = validate_shapes(SHAPES, data).unwrap();
    assert_eq!(
        report.conforms, expected,
        "data: {data}\nreport: {report:?}"
    );
}

#[test]
fn accepts_both_states_and_all_rdf_object_kinds() {
    assert_valid(&record(None), true);
    assert_valid(&record(Some(("q:derivationState", "\"resolved\""))), true);
    for object in [
        "<urn:example:object>",
        "_:object",
        "42",
        "\"objet\"@fr",
        "\"2026-01-01\"^^xsd:date",
    ] {
        assert_valid(&record(Some(("rdf:object", object))), true);
    }
}

#[test]
fn rejects_each_missing_field() {
    for &(predicate, _) in FIELDS {
        assert_valid(&record(Some((predicate, ""))), false);
    }
}

#[test]
fn rejects_multiple_values_for_each_field() {
    for &(predicate, original) in FIELDS {
        let second = match predicate {
            "q:derivationState" => "\"resolved\"",
            "q:promotionTx" | "q:invalidationTx" => "3",
            "prov:invalidatedAtTime" => "\"2026-01-02T00:00:00Z\"^^xsd:dateTime",
            "q:deriverSource" => "\"reasoner:another\"",
            _ => "<urn:example:another>",
        };
        assert_valid(
            &record(Some((predicate, &format!("{original}, {second}")))),
            false,
        );
    }
}

#[test]
fn rejects_unknown_state_and_wrong_term_kinds() {
    for (predicate, value) in [
        ("q:derivationState", "\"supported\""),
        ("q:derivationState", "\"unsupported\"@en"),
        ("q:derivationState", "<urn:example:unsupported>"),
        ("rdf:subject", "\"subject\""),
        ("rdf:subject", "_:subject"),
        ("rdf:predicate", "\"predicate\""),
        ("rdf:predicate", "_:predicate"),
        ("q:premiseGraph", "\"graph\""),
        ("q:premiseGraph", "_:graph"),
        ("q:deriverSource", "<urn:example:source>"),
        ("q:deriverSource", "\"reasoner:example\"@en"),
        ("prov:invalidatedAtTime", "\"2026-01-01T00:00:00Z\""),
        ("prov:invalidatedAtTime", "<urn:example:time>"),
    ] {
        assert_valid(&record(Some((predicate, value))), false);
    }
    assert_valid(
        &record(None).replace("<urn:example:record>", "_:record"),
        false,
    );
}

#[test]
fn transaction_ids_are_positive_integers() {
    for predicate in ["q:promotionTx", "q:invalidationTx"] {
        for value in ["0", "-1", "1.5", "\"1\"", "<urn:example:tx>"] {
            assert_valid(&record(Some((predicate, value))), false);
        }
    }
}

#[test]
fn loaded_product_vocabulary_accepts_record_through_knot() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .load_shapes("demoted-derivation", SHAPES, "2026-01-01T00:00:00Z")
        .unwrap();
    let result = tool_knot(&mut store, &json!({"turtle": record(None)})).unwrap();
    assert_eq!(result["conforms"], true, "{result}");
    assert_eq!(result["count"], 10, "{result}");
}

#[test]
fn declares_standard_reification_superclass() {
    // Parse the shipped schema and query the declaration, rather than matching text.
    let mut store = Store::open_in_memory().unwrap();
    tool_knot(&mut store, &json!({"turtle": SHAPES})).unwrap();
    let result = quipu::tool_query(&store, &json!({"query":
        "ASK { <http://quipu.local/graph#DemotedDerivation> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://www.w3.org/1999/02/22-rdf-syntax-ns#Statement> }"
    })).unwrap();
    assert_eq!(result["result"], true, "{result}");
}
