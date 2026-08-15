//! Drafting-scaffold tests. The two-sided discipline throughout: every refusal
//! is paired with the near-identical intent that must land, so each test
//! attributes the rejection to the field it names.

use super::*;
use crate::store::Store;

const TS: &str = "2026-01-01T00:00:00Z";
const EXEMPLAR: &str = "http://aegis.gastown.local/ontology/verdict_abc123";
const DOC_TYPE: &str = "http://ex/Doc";

fn intent() -> DraftIntent {
    DraftIntent {
        name: "no-unlabeled-docs".into(),
        label: "never land a Doc without a label again".into(),
        exemplar: EXEMPLAR.into(),
        target_type_iri: DOC_TYPE.into(),
        claim: "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }".into(),
        class: None,
        point: None,
        layer: None,
        authority: None,
    }
}

/// Ingest the emitted Turtle into a store with the placement check ON.
fn ingest_with_placement(turtle: &str) -> crate::error::Result<Store> {
    let mut store = Store::open_in_memory().unwrap();
    store.governance_config_mut().validate_placement = true;
    crate::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        TS,
        None,
        None,
    )?;
    Ok(store)
}

fn ask(store: &Store, q: &str) -> bool {
    matches!(
        crate::sparql::query(store, q).unwrap(),
        crate::sparql::QueryResult::Ask(true)
    )
}

#[test]
fn the_scaffold_round_trips_through_the_placement_check() {
    // The load-bearing claim of step 1: the emitted Turtle is not merely
    // syntactically Turtle, it is a policy the definition-time placement rules
    // ACCEPT — proven by ingesting it with the check on, not by asserting
    // about strings.
    let turtle = draft_turtle(&intent()).expect("a complete intent drafts");
    let store = ingest_with_placement(&turtle)
        .expect("the scaffold's defaults must satisfy the placement rules");
    let iri = intent().policy_iri();
    assert!(
        ask(
            &store,
            &format!("ASK {{ <{iri}> a <http://aegis.gastown.local/ontology/Policy> }}")
        ),
        "the drafted policy must land as an aegis:Policy"
    );
    assert!(
        ask(
            &store,
            &format!(
                "ASK {{ <{iri}> <http://aegis.gastown.local/ontology/exemplar> \"{EXEMPLAR}\" }}"
            )
        ),
        "the exemplar linkage must survive the round trip"
    );
}

#[test]
fn every_draft_is_born_advisory() {
    // Not "defaults to warn" — IS warn, with no field to say otherwise. The
    // hard-coded effect is the design's §4 contract, so the test asserts the
    // emitted fact, across every class the scaffold accepts.
    for class in [None, Some("soft".to_string()), Some("hard".to_string())] {
        let turtle = draft_turtle(&DraftIntent { class, ..intent() }).unwrap();
        assert!(
            turtle.contains("aegis:effect \"warn\""),
            "a draft must carry effect warn, got:\n{turtle}"
        );
        assert!(
            !turtle.contains("\"deny\"") && !turtle.contains("\"escalate\""),
            "no enforcing effect may appear in a draft:\n{turtle}"
        );
    }
}

#[test]
fn the_placement_check_still_refuses_a_malformed_override() {
    // The scaffold aims at placement validity but does not GUARANTEE it — the
    // check still runs and can refuse (design step 2: "which still runs and
    // still refuses a malformed result"). hard@PAA is Table 3's canonical
    // malformation; the scaffold passes the override through and the STORE
    // rejects the ingest, which is the division of labour the module doc
    // promises.
    let turtle = draft_turtle(&DraftIntent {
        class: Some("hard".into()),
        point: Some("PAA".into()),
        ..intent()
    })
    .expect("the scaffold does not second-guess placement");
    let err = ingest_with_placement(&turtle).err();
    assert!(
        matches!(err, Some(crate::error::Error::PolicyDenied(_))),
        "hard@PAA must be refused by the placement check, got {err:?}"
    );
}

#[test]
fn class_defaults_derive_the_advisory_point() {
    let soft = draft_turtle(&intent()).unwrap();
    assert!(soft.contains("\"soft\"") && soft.contains("\"PAA\""));
    let hard = draft_turtle(&DraftIntent {
        class: Some("hard".into()),
        ..intent()
    })
    .unwrap();
    assert!(hard.contains("\"hard\"") && hard.contains("\"PAG\""));
}

#[test]
fn an_escalation_class_draft_is_refused_with_the_reason() {
    let err = draft_turtle(&DraftIntent {
        class: Some("escalation".into()),
        ..intent()
    });
    let Err(crate::error::Error::InvalidValue(why)) = err else {
        panic!("escalation class must be refused, got {err:?}");
    };
    assert!(
        why.contains("born advisory") && why.contains("warn"),
        "the refusal must explain the born-advisory contract: {why}"
    );
}

#[test]
fn a_claim_without_target_is_refused_and_with_it_lands() {
    let err = draft_turtle(&DraftIntent {
        claim: "ASK { ?s ?p ?o }".into(),
        ..intent()
    });
    let Err(crate::error::Error::InvalidValue(why)) = err else {
        panic!("a target-less claim must be refused, got {err:?}");
    };
    assert!(
        why.contains("$target"),
        "the refusal must name the missing placeholder and the remedy: {why}"
    );
    // The green twin: the same intent with $target drafts.
    draft_turtle(&intent()).expect("a $target claim drafts");
}

#[test]
fn an_empty_exemplar_is_refused() {
    // The scaffold exists to draft FROM an example; without one there is
    // nothing for later refusals to cite. (aegis:exemplar stays optional on
    // Policy generally — hand-authored rules simply do not use the scaffold.)
    let err = draft_turtle(&DraftIntent {
        exemplar: "  ".into(),
        ..intent()
    });
    assert!(
        matches!(err, Err(crate::error::Error::InvalidValue(_))),
        "an exemplar-less draft must be refused, got {err:?}"
    );
}

#[test]
fn quotes_in_the_claim_survive_the_round_trip() {
    // Claims are SPARQL and routinely carry quoted literals; an escaping bug
    // here would emit different-but-parseable Turtle, which no error would
    // ever surface.
    let claim = "ASK { $target <http://ex/status> \"done\" }";
    let it = DraftIntent {
        claim: claim.into(),
        ..intent()
    };
    let store = ingest_with_placement(&draft_turtle(&it).unwrap()).unwrap();
    let got = format!(
        "ASK {{ <{}> <http://aegis.gastown.local/ontology/claim> ?c . \
         FILTER(CONTAINS(?c, \"\\\"done\\\"\")) }}",
        it.policy_iri()
    );
    assert!(
        ask(&store, &got),
        "the quoted literal must land inside the claim intact"
    );
}

#[test]
fn a_name_that_cannot_form_an_iri_is_refused() {
    let err = draft_turtle(&DraftIntent {
        name: "has spaces>".into(),
        ..intent()
    });
    assert!(
        matches!(err, Err(crate::error::Error::InvalidValue(_))),
        "an unusable name must be refused, got {err:?}"
    );
}

#[test]
fn the_verbatim_label_and_authority_are_kept() {
    let turtle = draft_turtle(&DraftIntent {
        authority: Some("stiwi".into()),
        ..intent()
    })
    .unwrap();
    assert!(
        turtle.contains("never land a Doc without a label again"),
        "the intent sentence must survive verbatim"
    );
    assert!(
        turtle.contains("aegis:authority \"stiwi\""),
        "declared authority must be emitted"
    );
}
