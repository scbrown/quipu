//! Controlled nominal models, not legal advice or an accepted legal oracle.
//! No legal expiry, raw history certificate or live vocabulary is manufactured.

use quipu::{
    reasoner::{
        editions::select_from_catalogue_snapshot, historical::evaluate_temporal_view, parse_rules,
    },
    sparql::{GraphScope, TemporalContext},
    store::{Datum, Store},
    types::{Op, Value},
};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

const PACKAGE: &str = "https://example.org/teaching/package";
const PREFIX: &str = "https://example.org/teaching/";
const TRANSITION: &str = "2008-06-19T00:00:00Z";

fn rules(transition: bool) -> String {
    let rows = if transition {
        vec![
            (
                "prior-cap",
                "modelNominal(?c, ?x)",
                "oldEndpoint(?c, ?x), reducedEndpoint(?c, ?y), earlierOrEqual(?x, ?y), reduction(?c, ?t)",
            ),
            (
                "reduced-binds",
                "modelNominal(?c, ?y)",
                "oldEndpoint(?c, ?x), reducedEndpoint(?c, ?y), earlierOrEqual(?y, ?x), reduction(?c, ?t)",
            ),
            (
                "new-claim",
                "modelNominal(?c, ?e)",
                "newEndpoint(?c, ?e), newClaim(?c, ?t)",
            ),
            ("pre-entry", "modelRegime(?c, ?r)", "preEntryRegime(?c, ?r)"),
            (
                "expired-input",
                "modelClassification(?c, ?e)",
                "expiredClassification(?c, ?e)",
            ),
        ]
    } else {
        vec![(
            "old",
            "modelNominal(?c, ?e)",
            "oldEndpoint(?c, ?e), ordinaryOpen(?c, ?t)",
        )]
    };
    let mut turtle = "@prefix rule: <http://quipu.local/rule#> .\n".to_string();
    for (id, head, body) in rows {
        turtle += &format!(
            "<{PREFIX}rule/{id}> a rule:Rule; rule:id \"{id}\"; rule:prefix \"{PREFIX}\"; rule:head \"{head}\"; rule:body \"{body}\" .\n"
        );
    }
    turtle
}

fn catalogue() -> String {
    let mut text = format!(
        r#"
@prefix rule: <http://quipu.local/rule#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix dct: <http://purl.org/dc/terms/> .
@prefix prov: <http://www.w3.org/ns/prov#> .
@prefix time: <http://www.w3.org/2006/time#> .
@prefix spdx: <http://spdx.org/rdf/terms#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
<{PACKAGE}> a rule:RuleSet; rdfs:label "Illustrative nominal model" .
"#
    );
    for (name, start, end, content) in [
        (
            "old",
            "1900-01-01T00:00:00Z",
            Some(TRANSITION),
            rules(false),
        ),
        ("transition", TRANSITION, None, rules(true)),
    ] {
        let value = oxrdf::Literal::new_simple_literal(&content);
        let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
        let ends = end.map_or_else(String::new, |end| format!("; time:hasEnd <{PREFIX}{name}-end> . <{PREFIX}{name}-end> a time:Instant; time:inXSDDateTimeStamp \"{end}\"^^xsd:dateTimeStamp"));
        text += &format!(
            r#"
<{PREFIX}{name}> a rule:RuleEdition; dct:isVersionOf <{PACKAGE}>;
 prov:value {value}; dct:source <https://example.org/supplied-teaching-model>;
 time:hasTime <{PREFIX}{name}-interval>; spdx:checksum <{PREFIX}{name}-checksum> .
<{PREFIX}{name}-interval> a time:Interval; time:hasBeginning <{PREFIX}{name}-begin> {ends} .
<{PREFIX}{name}-begin> a time:Instant; time:inXSDDateTimeStamp "{start}"^^xsd:dateTimeStamp .
<{PREFIX}{name}-checksum> a spdx:Checksum; spdx:algorithm spdx:checksumAlgorithm_sha256; spdx:checksumValue "{hash}" .
"#
        );
    }
    text
}

fn cases() -> Vec<Json> {
    let data: Json = serde_json::from_str(include_str!(
        "fixtures/temporal_rule_editions/french-model-cases.json"
    ))
    .unwrap();
    data["cases"].as_array().unwrap().clone()
}

/// Missing inputs are unknown, never an inferred absence of an exception.
fn model(case: &Json, question: &str) -> Option<Vec<String>> {
    let p = &case["premises"];
    let pre_entry = p["pre_entry_proceedings"].as_bool()?;
    let expired = p["expired_at_entry"].as_bool()?;
    if !p["general_regime_assumed"].as_bool()?
        || !p["adult"].as_bool()?
        || p["special_exception"].as_bool()?
        || p["calendar_computation_in_scope"].as_bool()?
    {
        return None;
    }
    let accrual = p["accrual_at"].as_str()?;
    if p["actual_knowledge_at"].as_str()? != accrual
        || p["constructive_knowledge_at"].as_str()? != accrual
    {
        return None;
    }
    if !pre_entry && (p["suspension"].as_bool()? || p["interruption"].as_bool()?) {
        return None;
    }
    let mut triples = Vec::new();
    let mut add = |s: &str, predicate: &str, o: &str| {
        triples.push((s.to_string(), predicate.to_string(), o.to_string()));
    };
    let claim = "claim";
    if pre_entry {
        add(claim, "preEntryRegime", "old-law-regime");
    } else if expired {
        add(claim, "expiredClassification", "supplied-already-expired");
    } else if p["accrual_at"].as_str()? >= "2008-06-19" {
        add(claim, "newClaim", "supplied");
        add(
            claim,
            "newEndpoint",
            case["proposed_expected"]["nominal_model_endpoint"].as_str()?,
        );
    } else {
        let old = p["old_nominal_endpoint"]
            .as_str()
            .or_else(|| case["proposed_expected"]["nominal_model_endpoint"].as_str())?;
        add(claim, "oldEndpoint", old);
        add(claim, "ordinaryOpen", "supplied");
        if let Some(reduced) = p["entry_plus_five_nominal_endpoint"].as_str() {
            add(claim, "reducedEndpoint", reduced);
            add(claim, "reduction", "supplied");
            // Ordering of already supplied ISO endpoints, not calendar arithmetic.
            if old <= reduced {
                add(old, "earlierOrEqual", reduced);
            }
            if reduced <= old {
                add(reduced, "earlierOrEqual", old);
            }
        }
    }
    let mut store = Store::open_in_memory().unwrap();
    let datums = triples
        .into_iter()
        .map(|(s, p, o)| Datum {
            entity: store.intern(&format!("{PREFIX}{s}")).unwrap(),
            attribute: store.intern(&format!("{PREFIX}{p}")).unwrap(),
            value: Value::Ref(store.intern(&format!("{PREFIX}{o}")).unwrap()),
            valid_from: "1900-01-01T00:00:00Z".into(),
            valid_to: None,
            op: Op::Assert,
        })
        .collect::<Vec<_>>();
    store
        .transact(&datums, "2000-01-01T00:00:00Z", None, None)
        .unwrap();
    let selected = select_from_catalogue_snapshot(&catalogue(), PACKAGE, question).unwrap();
    let parsed = parse_rules(&selected.turtle, None).unwrap();
    let context = TemporalContext {
        valid_at: Some(question.into()),
        as_of_tx: Some(store.latest_tx_id().unwrap()),
        graph: GraphScope::Named(vec![quipu::schema::ROOT_GRAPH]),
        ..TemporalContext::default()
    };
    Some(
        evaluate_temporal_view(&store, &parsed, &context)
            .unwrap()
            .into_iter()
            .map(|f| f.object)
            .collect(),
    )
}

#[test]
fn temporal_rule_editions_six_nominal_cases_keep_legal_expiry_unknown() {
    let rows = cases();
    assert_eq!(rows.len(), 6);
    for case in rows {
        assert_eq!(case["proposed_expected"]["legal_expiry"], "UNKNOWN");
        for source in case["primary_sources"].as_array().unwrap() {
            assert!(source["article"].is_string() && source["edition"].is_string());
            assert!(source["raw_extract_sha256"].is_null());
            if source["article"] == "26" {
                assert!(matches!(source["paragraph"].as_str(), Some("II" | "III")));
            }
        }
        let date = format!(
            "{}T00:00:00Z",
            case["premises"]["question_date"].as_str().unwrap()
        );
        let expected = match case["id"].as_str().unwrap() {
            "FR-PREENTRY-PROCEEDINGS" => "old-law-regime",
            "FR-EXPIRED-INPUT" => "supplied-already-expired",
            _ => case["proposed_expected"]["nominal_model_endpoint"]
                .as_str()
                .unwrap(),
        };
        assert_eq!(
            model(&case, &date).unwrap(),
            vec![format!("{PREFIX}{expected}")],
            "{}",
            case["id"]
        );
    }
}

#[test]
fn temporal_rule_editions_transition_uses_same_facts_and_unknown_inputs_stay_unknown() {
    let mut case = cases()
        .into_iter()
        .find(|c| c["id"] == "FR-REDUCTION-BINDS")
        .unwrap();
    assert_eq!(
        model(&case, "2007-01-01T00:00:00Z").unwrap(),
        vec![format!("{PREFIX}2020-01-01")]
    );
    assert_eq!(
        model(&case, "2014-01-01T00:00:00Z").unwrap(),
        vec![format!("{PREFIX}2013-06-19")]
    );
    for field in [
        "pre_entry_proceedings",
        "expired_at_entry",
        "general_regime_assumed",
        "adult",
        "special_exception",
        "calendar_computation_in_scope",
        "actual_knowledge_at",
        "constructive_knowledge_at",
        "suspension",
        "interruption",
    ] {
        let saved = case["premises"]
            .as_object_mut()
            .unwrap()
            .remove(field)
            .unwrap();
        assert!(model(&case, "2014-01-01T00:00:00Z").is_none());
        case["premises"][field] = saved;
    }
}
