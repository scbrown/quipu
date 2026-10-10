//! Search seeds first, then evaluate SPARQL with a bounded VALUES relation.
//! # arming: library — explicitly invoked read operation; no index activation.
use std::collections::BTreeMap;

use serde_json::{Value, json};
use spargebra::{
    Query,
    algebra::GraphPattern,
    term::{GroundTerm, NamedNode, Variable},
};

use crate::{Error, Result, Store};

fn invalid(message: &str) -> Error {
    Error::InvalidValue(message.into())
}

/// Search-rooted SELECT. Keyword mode needs no embeddings; hybrid requires both
/// retrievers to succeed. Search provenance is returned separately from rows.
pub fn tool_search_query(store: &Store, input: &Value) -> Result<Value> {
    let mode = input
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("keyword");
    if !matches!(mode, "keyword" | "semantic" | "hybrid") {
        return Err(invalid("mode must be keyword, semantic or hybrid"));
    }
    let count = match input.get("seed_limit") {
        None => 20,
        Some(value) => value
            .as_u64()
            .filter(|n| (1..=100).contains(n))
            .ok_or_else(|| invalid("seed_limit must be an integer in 1..100"))?
            as usize,
    };
    let variable = input
        .get("seed_variable")
        .and_then(Value::as_str)
        .unwrap_or("s");
    let variable =
        Variable::new(variable).map_err(|_| invalid("invalid seed_variable (omit '?')"))?;
    let text = input
        .get("sparql")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("sparql is required"))?;
    let mut parsed = spargebra::SparqlParser::new()
        .parse_query(text)
        .map_err(|e| invalid(&format!("SPARQL: {e}")))?;
    let Query::Select { pattern, .. } = &mut parsed else {
        return Err(invalid("search-rooted queries support SELECT only"));
    };
    if projection_depth(pattern) != 1 {
        return Err(invalid("subqueries are not supported"));
    }
    let mut probe = pattern.clone();
    bind_before_modifiers(
        &mut probe,
        GraphPattern::Values {
            variables: vec![variable.clone()],
            bindings: vec![],
        },
        &variable,
    )?;
    if let Some(embedding) = input.get("embedding") {
        let values = embedding
            .as_array()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("embedding must be a nonempty numeric array"))?;
        if values
            .iter()
            .any(|v| v.as_f64().is_none_or(|n| !(n as f32).is_finite()))
        {
            return Err(invalid("embedding must contain finite f32 numbers"));
        }
    }
    if input.get("valid_at").is_some_and(|v| v.as_str().is_none()) {
        return Err(invalid("valid_at must be a string"));
    }
    // Query options are deliberately separate from search text.
    let mut options = input
        .get("query_options")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let object = options
        .as_object_mut()
        .ok_or_else(|| invalid("query_options must be an object"))?;
    if object.contains_key("query")
        || object.contains_key("tx")
        || object.contains_key("valid_at")
        || object.contains_key("federated")
    {
        return Err(invalid(
            "query_options cannot override query, tx, valid_at or federation",
        ));
    }
    for (key, value) in object.iter() {
        let valid = match key.as_str() {
            "graph" | "fork" | "entailment" => value.is_string(),
            "verbose" | "row_labels" => value.is_boolean(),
            "include_kinds" => value
                .as_array()
                .is_some_and(|v| v.iter().all(Value::is_string)),
            _ => false,
        };
        if !valid {
            return Err(invalid(&format!(
                "unsupported or malformed query option: {key}"
            )));
        }
    }
    if let Some(at) = input.get("valid_at") {
        object.insert("valid_at".into(), at.clone());
    }
    let mut seeds: BTreeMap<String, Value> = BTreeMap::new();
    for retriever in ["keyword", "semantic"] {
        if mode != "hybrid" && mode != retriever {
            continue;
        }
        let mut search = json!({"mode":retriever,"limit":count,"verbose":true});
        for key in ["query", "valid_at", "entity_type", "group_ids"] {
            if let Some(value) = input.get(key) {
                search[key] = value.clone();
            }
        }
        if retriever == "semantic" {
            if let Some(value) = input.get("embedding") {
                search["embedding"] = value.clone();
            }
        } else if mode == "keyword" && input.get("embedding").is_some() {
            return Err(invalid("keyword mode does not accept embedding"));
        }
        let result = crate::tool_search(store, &search)?;
        let hits = result["results"]
            .as_array()
            .ok_or_else(|| invalid("search omitted results"))?;
        for (position, hit) in hits.iter().enumerate() {
            let iri = hit["entity"]
                .as_str()
                .ok_or_else(|| invalid("search omitted entity IRI"))?;
            // Parse, never interpolate unchecked strings into SPARQL.
            NamedNode::new(iri).map_err(|_| invalid("search returned invalid entity IRI"))?;
            let seed = seeds.entry(iri.into()).or_insert_with(|| {
                json!({
                    "entity":iri, "rrf_score":0.0, "keyword_rank":null, "semantic_rank":null
                })
            });
            let rank = position + 1;
            seed[format!("{retriever}_rank")] = rank.into();
            seed["rrf_score"] =
                (seed["rrf_score"].as_f64().unwrap_or(0.0) + 1.0 / (60.0 + rank as f64)).into();
        }
    }
    let mut seeds: Vec<Value> = seeds.into_values().collect();
    seeds.sort_by(|a, b| {
        b["rrf_score"]
            .as_f64()
            .unwrap()
            .total_cmp(&a["rrf_score"].as_f64().unwrap())
            .then_with(|| a["entity"].as_str().cmp(&b["entity"].as_str()))
    });
    let candidates = seeds.len();
    seeds.truncate(count);
    let values = GraphPattern::Values {
        variables: vec![variable.clone()],
        bindings: seeds
            .iter()
            .map(|seed| {
                vec![Some(GroundTerm::NamedNode(
                    NamedNode::new(seed["entity"].as_str().unwrap()).unwrap(),
                ))]
            })
            .collect(),
    };
    bind_before_modifiers(pattern, values, &variable)?;
    options["query"] = parsed.to_string().into();
    let result = crate::tool_query(store, &options)?;
    Ok(json!({
        "seeds":seeds, "seed_count":seeds.len(), "seed_limit":count,
        "candidate_count":candidates, "seeds_truncated":candidates>count,
        "retrieval_mode":mode, "rrf_k":60, "lexical_plane":"ROOT",
        "search_complete":false,
        "scope_note":"Bounded retrieval seeds, not a complete graph enumeration; query dataset is selected separately.",
        "result":result
    }))
}

/// Insert beneath SELECT solution modifiers, before aggregation and projection.
/// VALUES is a left input so evaluation begins from bindings, including zero
/// bindings. It is not a string substitution or a post-query result filter.
fn bind_before_modifiers(
    pattern: &mut GraphPattern,
    values: GraphPattern,
    variable: &Variable,
) -> Result<()> {
    match pattern {
        GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Filter { inner, .. }
        | GraphPattern::Extend { inner, .. } => {
            bind_before_modifiers(inner, values, variable)?;
        }
        _ => {
            if !seed_safe(pattern) {
                return Err(invalid(
                    "root pattern supports BGP, GRAPH and joins only; OPTIONAL, UNION, paths, SERVICE and subqueries are not supported",
                ));
            }
            let mut in_scope = false;
            pattern.on_in_scope_variable(|v| in_scope |= v == variable);
            if !in_scope {
                return Err(invalid(
                    "seed_variable must occur in the query graph pattern",
                ));
            }
            let original = std::mem::replace(pattern, GraphPattern::Bgp { patterns: vec![] });
            *pattern = GraphPattern::Join {
                left: Box::new(values),
                right: Box::new(original),
            };
        }
    }
    Ok(())
}

fn projection_depth(pattern: &GraphPattern) -> usize {
    match pattern {
        GraphPattern::Project { inner, .. } => 1 + projection_depth(inner),
        GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Filter { inner, .. }
        | GraphPattern::Extend { inner, .. } => projection_depth(inner),
        _ => 0,
    }
}

fn seed_safe(pattern: &GraphPattern) -> bool {
    match pattern {
        GraphPattern::Bgp { .. } => true,
        GraphPattern::Graph { inner, .. } => seed_safe(inner),
        GraphPattern::Join { left, right } => seed_safe(left) && seed_safe(right),
        _ => false,
    }
}

#[cfg(test)]
#[path = "search_query_tests.rs"]
mod tests;
