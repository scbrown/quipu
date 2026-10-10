//! Opt-in lexical/vector fusion, with unchanged pure-mode endpoints.
use std::collections::BTreeMap;

use serde_json::{Value as JsonValue, json};

use crate::{Error, Result, Store};

#[cfg(test)]
mod tests;

fn invalid(message: &str) -> Error {
    Error::InvalidValue(message.into())
}

pub(super) fn dispatch_search_fusion(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    if !input.is_object() {
        return Err(invalid("search input must be an object"));
    }
    let config = store.search_config();
    // Turning the feature off must restore old implicit callers even when
    // an operator previously selected hybrid as the server default.
    let configured_mode = if config.mode == "hybrid" && !config.hybrid {
        "semantic"
    } else {
        config.mode.as_str()
    };
    let mode = input
        .get("mode")
        .map_or(Some(configured_mode), JsonValue::as_str)
        .ok_or_else(|| invalid("mode must be semantic, keyword, or hybrid"))?;
    if !matches!(mode, "semantic" | "keyword" | "hybrid") {
        return Err(invalid("mode must be semantic, keyword, or hybrid"));
    }
    let alpha = number(input, "alpha", config.alpha)?;
    if !(0.0..=1.0).contains(&alpha) {
        return Err(invalid("alpha must be finite and in [0,1]"));
    }
    let fusion = input
        .get("fusion")
        .map_or(Some(config.fusion.as_str()), JsonValue::as_str)
        .ok_or_else(|| invalid("fusion must be weighted or rrf"))?;
    if !matches!(fusion, "weighted" | "rrf") {
        return Err(invalid("fusion must be weighted or rrf"));
    }
    let k = number(input, "rrf_k", config.rrf_k)?;
    if k <= 0.0 {
        return Err(invalid("rrf_k must be finite and positive"));
    }
    let explain = match input.get("explain") {
        None => false,
        Some(JsonValue::Bool(b)) => *b,
        _ => return Err(invalid("explain must be boolean")),
    };
    if mode == "hybrid" && !config.hybrid {
        return Err(invalid(
            "hybrid search is disabled ([quipu.search] hybrid = false)",
        ));
    }
    // No candidate enlargement, normalization, reranking, or lexical dependency
    // at alpha=1. Even scores and tie order retain their original representation.
    if mode == "semantic" || (mode == "hybrid" && alpha == 1.0) {
        let mut response = super::search::semantic_response(store, input)?;
        if explain {
            explain_pure(&mut response, input, true);
        }
        return Ok(response);
    }
    if mode == "keyword" || alpha == 0.0 {
        let mut response = super::search::keyword_response(store, input)?;
        if explain {
            explain_pure(&mut response, input, false);
        }
        return Ok(response);
    }
    if input.get("anchor").is_some()
        || input
            .get("ranking")
            .and_then(JsonValue::as_str)
            .is_some_and(|r| r != "semantic")
    {
        return Err(invalid(
            "hybrid fusion does not accept anchor or content ranking",
        ));
    }
    // Both branches must use the same type-inference contract, temporal point,
    // graph and group scopes. Semantic's historical default is inference-on.
    let mut request = input.clone();
    match input.get("infer_types") {
        None | Some(JsonValue::Bool(true)) => {
            request["infer_types"] = json!(true);
        }
        _ => {
            return Err(invalid(
                "hybrid fusion requires infer_types=true for semantic/lexical scope agreement",
            ));
        }
    }
    let limit = config.clamp_limit(input.get("limit").and_then(JsonValue::as_u64));
    if limit > 1000 {
        return Err(invalid("intermediate hybrid limit must not exceed 1000"));
    }
    // Bounded candidate union. Response records this ceiling; it does not claim
    // exhaustive global normalization of either backend.
    let pool = config
        .oversample(limit)
        .min(config.max_limit.min(1000))
        .max(limit);
    request["limit"] = json!(pool);
    request["mode"] = json!("semantic");
    let semantic = super::search::semantic_response(store, &request)?;
    request["mode"] = json!("keyword");
    request.as_object_mut().unwrap().remove("embedding");
    request["explain"] = json!(explain);
    let keyword = super::search::keyword_response(store, &request)?;
    let rows = fuse(
        semantic["results"].as_array().unwrap(),
        keyword["results"].as_array().unwrap(),
        (alpha, fusion, k),
        limit,
        &request,
        explain,
    );
    Ok(
        json!({"count":rows.len(),"results":rows,"ranking":"hybrid","scoped":semantic["scoped"],
        "fusion":fusion,"alpha":alpha,"rrf_k":k,"candidate_limit":pool,"infer_types":true}),
    )
}

fn number(input: &JsonValue, name: &str, default: f64) -> Result<f64> {
    let value = input
        .get(name)
        .map_or(Some(default), JsonValue::as_f64)
        .ok_or_else(|| invalid(&format!("{name} must be a finite number")))?;
    if !value.is_finite() {
        return Err(invalid(&format!("{name} must be a finite number")));
    }
    Ok(value)
}

fn normalized(rows: &[JsonValue]) -> Vec<f64> {
    let min = rows
        .iter()
        .filter_map(|r| r["score"].as_f64())
        .fold(f64::INFINITY, f64::min);
    let max = rows
        .iter()
        .filter_map(|r| r["score"].as_f64())
        .fold(f64::NEG_INFINITY, f64::max);
    rows.iter()
        .map(|r| {
            if max > min {
                (r["score"].as_f64().unwrap() - min) / (max - min)
            } else {
                1.0
            }
        })
        .collect()
}

fn filters(input: &JsonValue) -> JsonValue {
    let mut out = serde_json::Map::new();
    for key in [
        "valid_at",
        "group_ids",
        "entity_type",
        "infer_types",
        "graph",
        "graphs",
        "all_graphs",
    ] {
        if let Some(value) = input.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    JsonValue::Object(out)
}

fn explain_pure(response: &mut serde_json::Value, input: &JsonValue, semantic: bool) {
    if let Some(rows) = response["results"].as_array_mut() {
        for row in rows {
            row["explain"] = json!({"cosine":if semantic {row["similarity"].clone()} else {JsonValue::Null},
                "bm25":row.get("bm25").cloned().unwrap_or(JsonValue::Null),"fused_score":row["score"],
                "matched_fields":row.get("matched_fields").cloned().unwrap_or(json!([])),"applied_filters":filters(input)});
            if row.get("snippet").is_none() {
                row["snippet"] = json!(snippet(
                    row["text"].as_str().unwrap_or(""),
                    input["query"].as_str().unwrap_or("")
                ));
            }
        }
    }
}

struct Candidate {
    row: JsonValue,
    score: f64,
    cosine: Option<f64>,
    bm25: Option<f64>,
    semantic_rank: Option<usize>,
    keyword_rank: Option<usize>,
}

pub(super) fn fuse(
    semantic: &[JsonValue],
    keyword: &[JsonValue],
    options: (f64, &str, f64),
    limit: usize,
    input: &JsonValue,
    explain: bool,
) -> Vec<JsonValue> {
    let (alpha, fusion, k) = options;
    let mut candidates: BTreeMap<String, Candidate> = BTreeMap::new();
    for (rows, weight, is_semantic) in [(semantic, alpha, true), (keyword, 1.0 - alpha, false)] {
        let scores = normalized(rows);
        for (i, row) in rows.iter().enumerate() {
            let entity = row["entity"].as_str().unwrap().to_string();
            let entry = candidates.entry(entity).or_insert_with(|| Candidate {
                row: row.clone(),
                score: 0.0,
                cosine: None,
                bm25: None,
                semantic_rank: None,
                keyword_rank: None,
            });
            entry.score += weight
                * if fusion == "rrf" {
                    1.0 / (k + i as f64 + 1.0)
                } else {
                    scores[i]
                };
            if is_semantic {
                entry.cosine = row["similarity"].as_f64();
                entry.semantic_rank = Some(i + 1);
            } else {
                entry.bm25 = row["bm25"].as_f64();
                entry.keyword_rank = Some(i + 1);
                // Winning lexical literal supplies the evidence/snippet, rather
                // than an unrelated semantic text from another assertion.
                for key in [
                    "text",
                    "snippet",
                    "matched_fields",
                    "language",
                    "datatype",
                    "type_iri",
                    "graph",
                    "graphs",
                    "plane",
                    "valid_from",
                    "valid_to",
                ] {
                    if let Some(value) = row.get(key) {
                        entry.row[key] = value.clone();
                    }
                }
            }
        }
    }
    let mut ranked: Vec<_> = candidates.into_iter().collect();
    ranked.sort_by(|a, b| b.1.score.total_cmp(&a.1.score).then_with(|| a.0.cmp(&b.0)));
    ranked.into_iter().take(limit).map(|(_, Candidate {mut row,score,cosine,bm25,semantic_rank:srank,keyword_rank:krank})| {
        row["score"]=json!(score); row["similarity"]=json!(cosine); row["bm25"]=json!(bm25); row["ranking_reason"]=json!("hybrid");
        if explain {
            row["explain"]=json!({"cosine":cosine,"bm25":bm25,"fused_score":score,
                "semantic_rank":srank,"keyword_rank":krank,"matched_fields":row.get("matched_fields").cloned().unwrap_or(json!([])),"applied_filters":filters(input)});
            if row.get("snippet").is_none() { row["snippet"]=json!(snippet(row["text"].as_str().unwrap_or(""),input["query"].as_str().unwrap_or(""))); }
        }
        row
    }).collect()
}

/// Plain text with escaped HTML and <mark> delimiters, bounded to 240 source
/// characters around the first matching token. Never return executable markup.
fn snippet(text: &str, query: &str) -> String {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    let chars: Vec<char> = text.chars().collect();
    let first = text
        .split_inclusive(|c: char| !c.is_alphanumeric())
        .scan(0usize, |offset, word| {
            let pos = *offset;
            *offset += word.chars().count();
            Some((pos, word))
        })
        .find(|(_, word)| {
            terms.iter().any(|t| {
                *t == word
                    .trim_end_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
        })
        .map_or(0, |(pos, _)| pos);
    let start = first.saturating_sub(60);
    let end = (start + 240).min(chars.len());
    let selected: String = chars[start..end].iter().collect();
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    for word in selected.split_inclusive(|c: char| !c.is_alphanumeric()) {
        let token = word.trim_end_matches(|c: char| !c.is_alphanumeric());
        let matched = terms.iter().any(|t| *t == token.to_lowercase());
        if matched {
            out.push_str("<mark>");
        }
        for c in token.chars() {
            escape(c, &mut out);
        }
        if matched {
            out.push_str("</mark>");
        }
        for c in word[token.len()..].chars() {
            escape(c, &mut out);
        }
    }
    if end < chars.len() {
        out.push('…');
    }
    out
}

fn escape(c: char, out: &mut String) {
    match c {
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '&' => out.push_str("&amp;"),
        '"' => out.push_str("&quot;"),
        '\'' => out.push_str("&#39;"),
        _ => out.push(c),
    }
}

/// Escape content before turning the FTS-only delimiters into highlight markup.
pub(super) fn lexical_snippet(text: &str) -> String {
    let mut out = String::new();
    let mut marked = false;
    for c in text.chars().take(320) {
        match c {
            '\u{1e}' if !marked => {
                out.push_str("<mark>");
                marked = true;
            }
            '\u{1f}' if marked => {
                out.push_str("</mark>");
                marked = false;
            }
            '\u{1e}' | '\u{1f}' => {}
            _ => escape(c, &mut out),
        }
    }
    if marked {
        out.push_str("</mark>");
    }
    out
}
