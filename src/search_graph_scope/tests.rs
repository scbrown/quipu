use super::*;
use serde_json::json;

const AT: &str = "2026-01-01T00:00:00Z";

fn fixture() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.search_config_mut().keyword = true;
    s.search_config_mut().named_graphs = true;
    s.initialize_lexical_index().unwrap();
    for (graph, name, entity) in [
        (None, "shared root", "urn:root"),
        (Some("urn:graph:a"), "shared alpha", "urn:alpha"),
        (Some("urn:graph:b"), "shared beta", "urn:beta"),
    ] {
        let text = format!(
            "<{entity}> <http://www.w3.org/2000/01/rdf-schema#label> \"{name}\"; a <urn:type:WorkItem> ."
        );
        if let Some(graph) = graph {
            let id = s.graph_create(graph).unwrap();
            crate::rdf::ingest_rdf_to_graph(
                &mut s,
                text.as_bytes(),
                oxrdfio::RdfFormat::Turtle,
                None,
                AT,
                None,
                None,
                id,
            )
            .unwrap();
        } else {
            crate::rdf::ingest_rdf(
                &mut s,
                text.as_bytes(),
                oxrdfio::RdfFormat::Turtle,
                None,
                AT,
                None,
                None,
            )
            .unwrap();
        }
    }
    s.backfill_lexical_batch(100).unwrap();
    s
}

#[test]
fn keyword_scope_applies_before_limit_and_named_types_are_visible() {
    let s = fixture();
    let root = crate::tool_search(
        &s,
        &json!({"mode":"keyword","query":"shared","verbose":true}),
    )
    .unwrap();
    assert_eq!(root["count"], 1);
    assert_eq!(root["results"][0]["entity"], "urn:root");
    let named = crate::tool_search(&s, &json!({"mode":"keyword","query":"shared","graph":"urn:graph:a","limit":1,"entity_type":"urn:type:WorkItem","verbose":true})).unwrap();
    assert_eq!(named["count"], 1);
    assert_eq!(named["results"][0]["entity"], "urn:alpha");
    assert_eq!(named["results"][0]["graph"], "urn:graph:a");
    assert_eq!(named["results"][0]["plane"], "named");
}

#[test]
fn multiple_and_all_graphs_are_explicit_unions() {
    let s = fixture();
    for (selector, n) in [
        (json!({"graphs":["urn:graph:a","urn:graph:b"]}), 2),
        (json!({"all_graphs":true}), 3),
        (json!({"graph":"all"}), 3),
    ] {
        let mut input = json!({"mode":"keyword","query":"shared","verbose":true});
        input
            .as_object_mut()
            .unwrap()
            .extend(selector.as_object().unwrap().clone());
        let out = crate::tool_search(&s, &input).unwrap();
        assert_eq!(out["count"], n);
        assert!(
            out["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|hit| hit["graph"].is_string())
        );
    }
}

#[test]
fn unknown_malformed_and_conflicting_scopes_refuse_in_both_modes() {
    let s = fixture();
    for selector in [
        json!({"graph":"urn:missing"}),
        json!({"graph":7}),
        json!({"graphs":[]}),
        json!({"graphs":[7]}),
        json!({"all_graphs":"yes"}),
        json!({"graph":"urn:graph:a","all_graphs":true}),
        json!({"graph":"urn:graph:a","graphs":["urn:graph:b"]}),
    ] {
        for mode in ["semantic", "keyword"] {
            let mut input = json!({"mode":mode,"query":"shared"});
            input
                .as_object_mut()
                .unwrap()
                .extend(selector.as_object().unwrap().clone());
            assert!(crate::tool_search(&s, &input).is_err(), "{input}");
        }
    }
}

#[test]
fn all_scope_excludes_graph_metadata_plane() {
    let s = fixture();
    let scope = GraphScope::parse(&s, &json!({"all_graphs":true})).unwrap();
    assert!(!scope.ids.contains(&s.meta_graph_id().unwrap()));
}

#[test]
fn named_keyword_retraction_and_historical_scope_use_exclusive_end() {
    let mut s = fixture();
    let g = s.registered_graph_id("urn:graph:a").unwrap().unwrap();
    let entity = s.lookup("urn:alpha").unwrap().unwrap();
    let facts = s.entity_facts_in_graph(entity, g).unwrap();
    let retired: Vec<_> = facts
        .into_iter()
        .map(|f| crate::store::Datum {
            entity: f.entity,
            attribute: f.attribute,
            value: f.value,
            valid_from: "2026-02-01T00:00:00Z".into(),
            valid_to: None,
            op: crate::types::Op::Retract,
        })
        .collect();
    s.transact_to_graph(&retired, "2026-02-01T00:00:00Z", None, None, g)
        .unwrap();
    for (at, n) in [(AT, 1), ("2026-02-01T00:00:00Z", 0)] {
        let out = crate::tool_search(
            &s,
            &json!({"mode":"keyword","query":"shared","graph":"urn:graph:a","valid_at":at}),
        )
        .unwrap();
        assert_eq!(out["count"], n);
    }
}

#[test]
fn semantic_scopes_use_the_full_ranking_and_default_root_is_unchanged() {
    let s = fixture();
    for (iri, embedding) in [
        ("urn:root", vec![1.0, 0.0]),
        ("urn:alpha", vec![0.7, 0.7]),
        ("urn:beta", vec![0.6, 0.8]),
    ] {
        let id = s.lookup(iri).unwrap().unwrap();
        s.vector_store()
            .embed_entity(id, iri, &embedding, AT)
            .unwrap();
    }
    let root = crate::tool_search(&s, &json!({"embedding":[1.0,0.0],"verbose":true})).unwrap();
    assert_eq!(root["count"], 1);
    assert_eq!(root["results"][0]["entity"], "urn:root");
    let named=crate::tool_search(&s,&json!({"embedding":[1.0,0.0],"graph":"urn:graph:a","limit":1,"entity_type":"urn:type:WorkItem","verbose":true})).unwrap();
    assert_eq!(named["count"], 1);
    assert_eq!(named["results"][0]["entity"], "urn:alpha");
    let multiple = crate::tool_search(
        &s,
        &json!({"embedding":[1.0,0.0],"graphs":["urn:graph:a","urn:graph:b"],"verbose":true}),
    )
    .unwrap();
    assert_eq!(multiple["count"], 2);
    let all = crate::tool_search(
        &s,
        &json!({"embedding":[1.0,0.0],"all_graphs":true,"verbose":true}),
    )
    .unwrap();
    assert_eq!(all["count"], 3);
}

#[test]
fn frozen_fact_time_excludes_future_named_facts_from_both_modes() {
    let mut s = fixture();
    let g = s.registered_graph_id("urn:graph:a").unwrap().unwrap();
    let e = s.intern("urn:future").unwrap();
    let a = s
        .intern("http://www.w3.org/2000/01/rdf-schema#label")
        .unwrap();
    s.transact_to_graph(
        &[crate::Datum {
            entity: e,
            attribute: a,
            value: crate::Value::Str("futureprobe".into()),
            valid_from: "2027-01-01T00:00:00Z".into(),
            valid_to: None,
            op: crate::Op::Assert,
        }],
        "2026-01-01T00:00:00Z",
        None,
        None,
        g,
    )
    .unwrap();
    s.vector_store()
        .embed_entity(e, "futureprobe", &[1.0, 0.0], AT)
        .unwrap();
    for request in [
        json!({"mode":"keyword","query":"futureprobe"}),
        json!({"embedding":[1.0,0.0]}),
    ] {
        let mut input = request;
        input["graph"] = json!("urn:graph:a");
        input["valid_at"] = json!(AT);
        let out = crate::tool_search(&s, &input).unwrap();
        assert_eq!(out["count"], 0, "{input}");
    }
    let after=crate::tool_search(&s,&json!({"mode":"keyword","query":"futureprobe","graph":"urn:graph:a","valid_at":"2027-01-01T00:00:00Z"})).unwrap();
    assert_eq!(
        after["count"], 1,
        "positive control proves the indexed document is visible at its fact time"
    );
}

#[test]
fn keyword_empty_unknown_and_punctuation_queries_have_controls() {
    let mut s = fixture();
    let g = s.registered_graph_id("urn:graph:a").unwrap().unwrap();
    let e = s.intern("urn:punctuation").unwrap();
    let a = s
        .intern("http://www.w3.org/2000/01/rdf-schema#label")
        .unwrap();
    s.transact_to_graph(
        &[crate::Datum {
            entity: e,
            attribute: a,
            value: crate::Value::Str("issue-42_v2".into()),
            valid_from: AT.into(),
            valid_to: None,
            op: crate::Op::Assert,
        }],
        AT,
        None,
        None,
        g,
    )
    .unwrap();
    let query = |q: &str| {
        crate::tool_search(
            &s,
            &json!({"mode":"keyword","query":q,"graph":"urn:graph:a"}),
        )
    };
    assert_eq!(query("issue-42_v2").unwrap()["count"], 1);
    assert_eq!(query("unknowncontrolword999").unwrap()["count"], 0);
    assert!(query("").is_err());
    assert!(query("\"unclosed").is_err());
}

#[test]
fn default_off_refuses_exposure_while_prepared_named_vectors_stay_out_of_root() {
    let mut s = fixture();
    let named = s.lookup("urn:alpha").unwrap().unwrap();
    let root = s.lookup("urn:root").unwrap().unwrap();
    s.vector_store()
        .embed_entity(named, "prepared named", &[1.0, 0.0], AT)
        .unwrap();
    s.vector_store()
        .embed_entity(root, "root", &[0.5, 0.5], AT)
        .unwrap();
    s.search_config_mut().named_graphs = false;
    assert!(
        crate::tool_search(&s, &json!({"embedding":[1.0,0.0],"graph":"urn:graph:a"}))
            .unwrap_err()
            .to_string()
            .contains("disabled")
    );
    let out = crate::tool_search(&s, &json!({"embedding":[1.0,0.0],"verbose":true})).unwrap();
    assert_eq!(out["count"], 1);
    assert_eq!(out["results"][0]["entity"], "urn:root");
}

#[test]
fn deleting_named_facts_cannot_turn_their_vector_into_a_root_hit() {
    struct Fixed;
    impl crate::EmbeddingProvider for Fixed {
        fn embed_text(&self, _: &str) -> crate::Result<Vec<f32>> {
            Ok(vec![1.0, 0.0])
        }
        fn dimension(&self) -> usize {
            2
        }
    }
    let mut s = fixture();
    s.set_embedding_provider(std::sync::Arc::new(Fixed));
    s.embedding_config_mut().auto_embed = true;
    s.embedding_config_mut().dimension = 2;
    let named = s.lookup("urn:alpha").unwrap().unwrap();
    let root = s.lookup("urn:root").unwrap().unwrap();
    let g = s.registered_graph_id("urn:graph:a").unwrap().unwrap();
    let a = s
        .lookup("http://www.w3.org/2000/01/rdf-schema#label")
        .unwrap()
        .unwrap();
    s.transact_to_graph(
        &[crate::Datum {
            entity: named,
            attribute: a,
            value: crate::Value::Str("prepared named vector".into()),
            valid_from: AT.into(),
            valid_to: None,
            op: crate::Op::Assert,
        }],
        AT,
        None,
        None,
        g,
    )
    .unwrap();
    s.vector_store()
        .embed_entity(root, "root", &[0.5, 0.5], AT)
        .unwrap();
    s.search_config_mut().named_graphs = false;
    let query = json!({"embedding":[1.0,0.0],"verbose":true});
    let before = crate::tool_search(&s, &query).unwrap();
    assert_eq!(before["count"], 1);
    s.conn
        .execute(
            "DELETE FROM facts WHERE e=?1 AND g=?2",
            rusqlite::params![named, g],
        )
        .unwrap();
    let after = crate::tool_search(&s, &query).unwrap();
    assert_eq!(
        after["results"], before["results"],
        "deletion must not admit a previously hidden named vector into ROOT"
    );
}
