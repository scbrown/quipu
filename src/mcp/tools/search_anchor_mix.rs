//! Compose bounded anchor candidates with the shared text ranking.
use std::collections::{BTreeMap, HashSet};

use serde_json::{Value as JsonValue, json};

use crate::{Error, Result, Store};

use super::search_anchor::{AnchorRequest, rerank, summary, walk};

/// Collect global and neighbourhood candidates BEFORE fusion. Each source gets
/// half of the existing pool; the total per branch remains bounded by 1000.
/// The internal allowlist cannot be supplied by a caller to bypass scope checks.
pub(super) fn anchor_text_candidates(
    store: &Store,
    input: &JsonValue,
    neighbours: Option<&HashSet<i64>>,
    semantic: bool,
) -> Result<JsonValue> {
    let branch = |request: &JsonValue, ids: Option<&HashSet<i64>>| {
        if semantic {
            if ids.is_none() {
                super::search::semantic_response(store, request)
            } else {
                super::search::semantic_response_in(store, request, ids)
            }
        } else {
            if ids.is_none() {
                super::search::keyword_response(store, request)
            } else {
                super::search::keyword_response_in(store, request, ids)
            }
        }
    };
    let Some(neighbours) = neighbours else {
        return branch(input, None);
    };
    let pool = store
        .search_config()
        .clamp_limit(input.get("limit").and_then(JsonValue::as_u64))
        .min(1000);
    let mut request = input.clone();
    request["limit"] = json!(pool.div_ceil(2));
    let mut global = branch(&request, None)?;
    request["limit"] = json!((pool / 2).max(1));
    let nearby = branch(&request, Some(neighbours))?;
    let mut rows = BTreeMap::new();
    for row in global["results"]
        .as_array()
        .unwrap()
        .iter()
        .chain(nearby["results"].as_array().unwrap())
    {
        let key = row["entity"].as_str().unwrap().to_owned();
        rows.entry(key).or_insert_with(|| row.clone());
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    rows.sort_by(|a, b| {
        b["score"]
            .as_f64()
            .unwrap()
            .total_cmp(&a["score"].as_f64().unwrap())
            .then(a["entity"].as_str().cmp(&b["entity"].as_str()))
    });
    // pool=1 can request one from each source. Prefer the neighbourhood candidate
    // in that special case so the anchor remains a candidate source.
    if pool == 1 && !nearby["results"].as_array().unwrap().is_empty() {
        rows = vec![nearby["results"][0].clone()];
    }
    global["count"] = json!(rows.len());
    global["results"] = json!(rows);
    Ok(global)
}

pub(super) fn anchor_mix_response(
    store: &Store,
    input: &JsonValue,
    req: &AnchorRequest,
) -> Result<JsonValue> {
    if !store.search_config().anchored {
        return Err(Error::InvalidValue(
            "anchored search is disabled on this server ([quipu.search] anchored = false)".into(),
        ));
    }
    if crate::search_graph_scope::GraphScope::parse(store, input)?.explicit {
        return Err(Error::InvalidValue(
            "anchored search currently supports ROOT graph only".into(),
        ));
    }
    let limit = store
        .search_config()
        .clamp_limit(input.get("limit").and_then(JsonValue::as_u64));
    if limit > 1000 {
        return Err(Error::InvalidValue(
            "anchored result limit must not exceed 1000".into(),
        ));
    }
    let nb = walk(
        store,
        req,
        input.get("valid_at").and_then(JsonValue::as_str),
    )?;
    let neighbours: HashSet<_> = nb.hops.keys().copied().collect();
    let mut request = input.clone();
    request.as_object_mut().unwrap().remove("anchor");
    // Candidate generation needs a pool even when the user requests one result.
    request["limit"] = json!(
        store
            .search_config()
            .oversample(limit)
            .max(200)
            .min(store.search_config().max_limit.min(1000))
    );
    request["verbose"] = json!(true);
    let mut text =
        super::search_fusion::fusion_with_candidates(store, &request, Some(&neighbours))?;
    let by_id = text["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let id = store
                .lookup(row["entity"].as_str().unwrap())?
                .ok_or_else(|| Error::InvalidValue("search result has no entity id".into()))?;
            Ok((id, row.clone()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let candidates = by_id
        .iter()
        .map(|(id, row)| (*id, row["score"].as_f64().unwrap()))
        .collect();
    let prefixes = (!input
        .get("verbose")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false))
    .then(|| crate::compact::PrefixMap::from_store(store))
    .transpose()?;
    let rows: Vec<_> = rerank(candidates, &nb, req)
        .into_iter()
        .take(limit)
        .map(|(id, score, hops)| {
            let mut row = by_id[&id].clone();
            row["text_score"] = row["score"].clone();
            row["score"] = json!(score);
            row["hops"] = json!(hops);
            if let Some(prefixes) = &prefixes {
                row["entity"] = json!(prefixes.compact(row["entity"].as_str().unwrap()));
            }
            if req.explain {
                row["path"] = json!(nb.path(store, id));
            }
            row
        })
        .collect();
    text["count"] = json!(rows.len());
    text["results"] = json!(rows);
    text["ranking"] = json!("anchored");
    text["anchor"] = summary(&nb, req);
    text["anchor"]["candidate_sources"] = json!(["global", "neighbourhood"]);
    Ok(text)
}
