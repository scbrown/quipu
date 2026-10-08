use super::*;

fn row(id: &str, score: f64) -> JsonValue {
    json!({"entity":id,"score":score,"similarity":score,"bm25":-score,"text":"Memory <script> Beads"})
}

#[test]
fn weighted_union_normalizes_separate_scales_and_breaks_ties_by_entity() {
    let s = vec![row("b", 0.8), row("a", 0.4)];
    let k = vec![row("a", 100.0), row("c", 50.0)];
    let out = fuse(&s, &k, (0.5, "weighted", 60.0), 10, &json!({}), false);
    assert_eq!(
        out.iter()
            .map(|r| r["entity"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert_eq!(out[0]["score"], 0.5);
}

#[test]
fn rrf_is_weighted_and_explanation_retains_raw_components() {
    let out = fuse(
        &[row("b", 0.8), row("a", 0.4)],
        &[row("a", 100.0)],
        (0.25, "rrf", 10.0),
        10,
        &json!({"query":"Memory","valid_at":"2026-01-01"}),
        true,
    );
    assert_eq!(out[0]["entity"], "a");
    assert_eq!(out[0]["explain"]["cosine"], 0.4);
    assert_eq!(out[0]["explain"]["bm25"], -100.0);
    assert_eq!(out[0]["explain"]["semantic_rank"], 2);
    assert_eq!(out[0]["explain"]["keyword_rank"], 1);
    assert_eq!(
        out[0]["explain"]["applied_filters"]["valid_at"],
        "2026-01-01"
    );
    assert!(
        out[0]["snippet"]
            .as_str()
            .unwrap()
            .contains("<mark>Memory</mark> &lt;script&gt;")
    );
}

#[test]
fn empty_and_equal_score_controls() {
    assert!(fuse(&[], &[], (0.5, "weighted", 60.0), 5, &json!({}), false).is_empty());
    let out = fuse(
        &[row("z", 0.0), row("a", 0.0)],
        &[],
        (0.5, "weighted", 60.0),
        5,
        &json!({}),
        false,
    );
    assert_eq!(out[0]["entity"], "a");
    assert_eq!(out[0]["score"], 0.5);
}

#[test]
fn snippets_bound_unicode_and_escape_markup() {
    let text = format!("{} Memory & beads {}", "é".repeat(1000), "x".repeat(1000));
    let out = snippet(&text, "memory");
    assert!(out.contains("<mark>Memory</mark> &amp;"));
    assert!(out.chars().count() < 300);
}

#[test]
fn validation_and_disabled_positive_control() {
    let store = Store::open_in_memory().unwrap();
    for request in [
        json!({"alpha":-0.1}),
        json!({"alpha":"1"}),
        json!({"rrf_k":0}),
        json!({"fusion":"bad"}),
        json!({"mode":"bad"}),
        json!({"explain":"true"}),
    ] {
        assert!(
            dispatch_search_fusion(&store, &request).is_err(),
            "{request}"
        );
    }
    let e = dispatch_search_fusion(&store, &json!({"query":"Memory","mode":"hybrid"}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("hybrid search is disabled"), "{e}");
}

#[test]
fn pure_endpoints_are_exact_and_explain_has_real_lexical_fields() {
    use crate::vector::KnowledgeVectorStore;
    let mut store = Store::open_in_memory().unwrap();
    store.search_config_mut().hybrid = true;
    store.search_config_mut().keyword = true;
    store.initialize_lexical_index().unwrap();
    let turtle =
        r#"<https://example.org/a> <http://www.w3.org/2000/01/rdf-schema#label> "Memory Beads" ."#;
    crate::rdf::ingest_rdf(
        &mut store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        "2026-01-01T00:00:00Z",
        None,
        None,
    )
    .unwrap();
    let entity = store.lookup("https://example.org/a").unwrap().unwrap();
    store
        .embed_entity(entity, "Memory Beads", &[1., 0.], "2026-01-01T00:00:00Z")
        .unwrap();
    let fused = dispatch_search_fusion(
        &store,
        &json!({"mode":"hybrid","query":"Memory","embedding":[1.,0.],"alpha":0.5,"explain":true}),
    )
    .unwrap();
    assert_eq!(fused["count"], 1);
    assert_eq!(
        fused["results"][0]["explain"]["matched_fields"],
        json!(["label"])
    );
    let s = json!({"embedding":[1.,0.],"limit":1});
    let mut h = s.clone();
    h["mode"] = json!("hybrid");
    h["alpha"] = json!(1.0);
    assert_eq!(
        dispatch_search_fusion(&store, &s).unwrap(),
        dispatch_search_fusion(&store, &h).unwrap()
    );
    let k = json!({"mode":"keyword","query":"Memory","limit":1});
    let mut h = k.clone();
    h["mode"] = json!("hybrid");
    h["alpha"] = json!(0.0);
    assert_eq!(
        dispatch_search_fusion(&store, &k).unwrap(),
        dispatch_search_fusion(&store, &h).unwrap()
    );
    h["explain"] = json!(true);
    let explained = dispatch_search_fusion(&store, &h).unwrap();
    assert_eq!(
        explained["results"][0]["explain"]["matched_fields"],
        json!(["label"])
    );
    assert_eq!(
        explained["results"][0]["snippet"],
        "<mark>Memory</mark> Beads"
    );
    store.search_config_mut().keyword = false;
    // alpha1 has no keyword/index dependency.
    let mut h = s.clone();
    h["mode"] = json!("hybrid");
    h["alpha"] = json!(1.0);
    assert_eq!(
        dispatch_search_fusion(&store, &s).unwrap(),
        dispatch_search_fusion(&store, &h).unwrap()
    );
}

#[test]
fn non_object_requests_are_errors_not_panics() {
    let mut store = Store::open_in_memory().unwrap();
    store.search_config_mut().hybrid = true;
    store.search_config_mut().mode = "hybrid".into();
    for request in [json!(null), json!([]), json!(1)] {
        assert!(dispatch_search_fusion(&store, &request).is_err());
    }
}

#[test]
fn fts_delimiters_are_escaped_and_closed_when_truncated() {
    assert_eq!(
        lexical_snippet("<script>\u{1e}mémoire\u{1f}&"),
        "&lt;script&gt;<mark>mémoire</mark>&amp;"
    );
    let raw = format!("\u{1e}{}", "x".repeat(1000));
    assert!(lexical_snippet(&raw).ends_with("</mark>"));
}
