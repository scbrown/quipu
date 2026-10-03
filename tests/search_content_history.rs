//! Historical ranking must use explanatory content only during its valid interval.

use quipu::vector::KnowledgeVectorStore;
use quipu::{Store, tool_search};
use serde_json::json;

#[test]
fn retracted_description_stops_exempting_a_heading_at_the_upper_bound() {
    let mut store = Store::open_in_memory().unwrap();
    let turtle = format!(
        "@prefix ex: <http://example.org/> .
         @prefix model: <{}> .
         @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
         ex:heading a model:Section; rdfs:label \"Search\";
             rdfs:comment \"An explanation of search behavior\" .
         ex:lesson a model:FailureMode; rdfs:comment \"A useful operational lesson\" .",
        store.base_ns()
    );
    quipu::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01",
        None,
        None,
    )
    .unwrap();
    for (name, score) in [("heading", 0.8_f32), ("lesson", 0.6_f32)] {
        let entity = store.intern(&format!("http://example.org/{name}")).unwrap();
        store
            .embed_entity(
                entity,
                name,
                &[score, (1.0 - score * score).sqrt()],
                "2026-01-01",
            )
            .unwrap();
    }
    let heading = store.lookup("http://example.org/heading").unwrap().unwrap();
    let comment = store
        .lookup("http://www.w3.org/2000/01/rdf-schema#comment")
        .unwrap()
        .unwrap();
    let (_, count) = store
        .retract_triples(
            heading,
            Some(comment),
            None,
            "2026-02-01",
            None,
            false,
            None,
        )
        .unwrap();
    assert_eq!(count, 1);
    for (at, expected) in [
        (Some("2026-01-15"), "heading"),
        (Some("2026-02-01"), "lesson"),
        (Some("2026-02-02"), "lesson"),
        (None, "lesson"),
    ] {
        let result = tool_search(
            &store,
            &json!({
                "embedding": [1,0], "ranking": "content", "limit": 1,
                "verbose": true, "valid_at": at
            }),
        )
        .unwrap();
        assert_eq!(
            result["results"][0]["entity"],
            format!("http://example.org/{expected}"),
            "as of {at:?}"
        );
    }
}
