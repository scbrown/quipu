use super::*;

const PUBLIC: &str = "https://scbrown.github.io/quechua/ns#";

struct FixedEmbedding;

impl crate::embedding::EmbeddingProvider for FixedEmbedding {
    fn embed_text(&self, _: &str) -> Result<Vec<f32>> {
        Ok(vec![1.0, 0.0])
    }

    fn dimension(&self) -> usize {
        2
    }
}

fn load(store: &mut Store, turtle: &str, graph: Option<&str>) {
    let graph = graph.map_or(0, |iri| store.intern(iri).unwrap());
    crate::rdf::ingest_rdf_to_graph(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
        graph,
    )
    .unwrap();
}

fn search(store: &Store, group: &str) -> JsonValue {
    tool_search_nodes(
        store,
        &serde_json::json!({"query":"needle", "group_ids":[group], "max_results":100}),
    )
    .unwrap()
}

#[test]
fn graphiti_namespace_groups_and_properties() {
    let legacy = crate::namespace::DEFAULT_BASE_NS;
    for namespaces in [vec![legacy], vec![PUBLIC], vec![legacy, PUBLIC]] {
        for direct in [true, false] {
            let mut store = Store::open_in_memory().unwrap();
            let mut ttl = String::from(
                "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n\
                 <urn:hit> rdfs:label \"needle\" .\n\
                 <urn:foreign> rdfs:label \"needle\" ; <https://example.org/foreign#groupId> \"wanted\" .\n\
                 <urn:ungrouped> rdfs:label \"needle\" .\n\
                 <urn:other> rdfs:label \"needle\" .\n",
            );
            for (entity, group) in [("hit", "wanted"), ("other", "different")] {
                let subject = if direct {
                    format!("urn:{entity}")
                } else {
                    ttl.push_str(&format!(
                        "<urn:{entity}> <http://www.w3.org/ns/prov#wasGeneratedBy> <urn:episode:{entity}> .\n"
                    ));
                    format!("urn:episode:{entity}")
                };
                for ns in &namespaces {
                    ttl.push_str(&format!("<{subject}> <{ns}groupId> \"{group}\" .\n"));
                    ttl.push_str(&format!("<urn:{entity}> <{ns}filePath> \"src/lib.rs\" .\n"));
                }
            }
            load(&mut store, &ttl, None);
            let out = search(&store, "wanted");
            assert_eq!(out["count"], 1, "{namespaces:?}/direct={direct}: {out}");
            let node = &out["nodes"][0];
            assert_eq!(node["iri"], "urn:hit");
            assert_eq!(node["group_id"], "wanted");
            assert_eq!(node["group_ids"], serde_json::json!(["wanted"]));
            assert_eq!(node["properties"]["filePath"], "src/lib.rs");
            assert!(node["properties"].get("ns#filePath").is_none());
            assert_eq!(search(&store, "absent")["count"], 0);
            assert_eq!(search(&store, "different")["nodes"][0]["iri"], "urn:other");
        }
    }
}

#[test]
fn graphiti_namespace_mixed_multiple_groups_and_root_scope() {
    let mut store = Store::open_in_memory().unwrap();
    let old = crate::namespace::DEFAULT_BASE_NS;
    load(
        &mut store,
        &format!(
            "<urn:hit> <http://www.w3.org/2000/01/rdf-schema#label> \"needle\" ;\n\
             <{old}groupId> \"direct\" ;\n\
             <http://www.w3.org/ns/prov#wasGeneratedBy> <urn:episode> .\n\
             <urn:episode> <{PUBLIC}groupId> \"episode\" ."
        ),
        None,
    );
    load(
        &mut store,
        &format!("<urn:episode> <{PUBLIC}groupId> \"named-only\" ."),
        Some("urn:private-graph"),
    );
    for group in ["direct", "episode"] {
        let out = search(&store, group);
        assert_eq!(out["count"], 1);
        assert_eq!(
            out["nodes"][0]["group_ids"],
            serde_json::json!(["direct", "episode"])
        );
        assert!(out["nodes"][0].get("group_id").is_none());
    }
    assert_eq!(search(&store, "named-only")["count"], 0);
}

#[test]
fn graphiti_namespace_configured_group_and_foreign_provenance_control() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_base_ns("https://example.org/custom#");
    load(
        &mut store,
        "<urn:hit> <http://www.w3.org/2000/01/rdf-schema#label> \"needle\" ;\n\
         <http://www.w3.org/ns/prov#wasGeneratedBy> <urn:episode> .\n\
         <urn:episode> <https://example.org/custom#groupId> \"wanted\" ;\n\
         <https://example.org/foreign#groupId> \"foreign\" .",
        None,
    );
    assert_eq!(search(&store, "wanted")["count"], 1);
    assert_eq!(search(&store, "foreign")["count"], 0);
}

#[test]
fn graphiti_namespace_vector_path_preserves_group_filter() {
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(std::sync::Arc::new(FixedEmbedding));
    store.embedding_config_mut().auto_embed = true;
    load(
        &mut store,
        &format!(
            "<urn:vector-hit> <http://www.w3.org/2000/01/rdf-schema#label> \"semantic result\" ;\n\
             <http://www.w3.org/ns/prov#wasGeneratedBy> <urn:vector-episode> .\n\
             <urn:vector-episode> <{PUBLIC}groupId> \"wanted\" ."
        ),
        None,
    );
    // The query is absent from the label: only the vector path can find it.
    assert_eq!(
        search(&store, "wanted")["nodes"][0]["iri"],
        "urn:vector-hit"
    );
    assert_eq!(search(&store, "absent")["count"], 0);
}
