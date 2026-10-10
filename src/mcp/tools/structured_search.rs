//! Default-off SQLite structured search entrypoint and strict JSON AST.
use super::structured_candidates::Context;
use super::structured_syntax::{self, Expr};
use crate::sparql;
use crate::{Error, Result, Store};
use serde_json::{Value as Json, json};
fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidValue(message.into())
}
fn json_expr(value: &Json, depth: usize, nodes: &mut usize) -> Result<Expr> {
    *nodes += 1;
    if depth > 8 || *nodes > 64 {
        return Err(invalid("filters exceeds depth 8 or 64 nodes"));
    }
    let object = value
        .as_object()
        .ok_or_else(|| invalid("filters must be an expression object"))?;
    if object.len() != 1 {
        return Err(invalid(
            "filter expression must have exactly one of term, not, and, or",
        ));
    }
    let (key, value) = object.iter().next().expect("one key");
    match key.as_str() {
        "term" => {
            let term = value
                .as_str()
                .ok_or_else(|| invalid("term must be a string"))?;
            match structured_syntax::parse(term)
                .map_err(|e| invalid(format!("term byte {}: {}", e.offset, e.message)))?
            {
                Expr::Atom(atom) => Ok(Expr::Atom(atom)),
                _ => Err(invalid("JSON term must contain exactly one atom")),
            }
        }
        "not" => Ok(Expr::Not(Box::new(json_expr(value, depth + 1, nodes)?))),
        "and" | "or" => {
            let args = value
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or_else(|| invalid("and/or requires exactly two expression operands"))?;
            let a = Box::new(json_expr(&args[0], depth + 1, nodes)?);
            let b = Box::new(json_expr(&args[1], depth + 1, nodes)?);
            Ok(if key == "and" {
                Expr::And(a, b)
            } else {
                Expr::Or(a, b)
            })
        }
        _ => Err(invalid(format!("unknown filter operator {key:?}"))),
    }
}
pub(super) fn search(store: &Store, input: &Json) -> Result<Json> {
    if !store.search_config().structured {
        return Err(invalid(
            "structured search is disabled ([quipu.search] structured = false)",
        ));
    }
    if store.vector_delegate.is_some() || store.local_vector_backend.is_some() {
        return Err(invalid(
            "structured search requires the SQLite backend; no post-topK fallback",
        ));
    }
    if !store.attachments().is_empty() {
        return Err(invalid(
            "structured search currently requires an unattached ROOT SQLite store",
        ));
    }
    for field in [
        "graph",
        "graphs",
        "all_graphs",
        "anchor",
        "ranking",
        "group_ids",
        "entity_type",
        "infer_types",
        "explain",
        "alpha",
        "fusion",
        "rrf_k",
    ] {
        if input.get(field).is_some() {
            return Err(invalid(format!(
                "structured search does not yet support {field}; scope is never silently dropped"
            )));
        }
    }
    let expr = match (input.get("structured_query"), input.get("filters")) {
        (Some(Json::String(text)), None) => structured_syntax::parse(text)
            .map_err(|e| invalid(format!("structured query byte {}: {}", e.offset, e.message)))?,
        (None, Some(value)) => json_expr(value, 0, &mut 0)?,
        _ => {
            return Err(invalid(
                "supply exactly one structured_query string or filters object",
            ));
        }
    };
    if !structured_syntax::bounded(&expr, false) {
        return Err(invalid(
            "every OR branch requires a positive bounded term; negative-only expressions refuse",
        ));
    }
    let at = match input.get("valid_at") {
        None => None,
        Some(Json::String(s)) => Some(s.as_str()),
        _ => return Err(invalid("valid_at must be a string")),
    };
    let deadline = crate::time::request_deadline().or_else(|| {
        let ms = store.search_config().query_timeout_ms;
        (ms > 0).then(|| crate::time::Deadline::after_millis(ms))
    });
    let _ambient = crate::time::set_request_deadline(deadline);
    let _progress = deadline
        .map(|d| sparql::ProgressGuard::install(&store.conn, d))
        .transpose()?;
    let ctx = Context { store, at };
    let allowed = ctx.eval(&expr, None)?;
    let limit = store
        .search_config()
        .clamp_limit(input.get("limit").and_then(Json::as_u64));
    let mode = match input.get("mode") {
        None => "semantic",
        Some(Json::String(s)) => s.as_str(),
        _ => return Err(invalid("mode must be semantic or keyword")),
    };
    let query = input.get("query").and_then(Json::as_str);
    let matches = match mode {
        "keyword" => store.keyword_search(
            query.ok_or_else(|| invalid("keyword ranking requires query text"))?,
            limit,
            at,
            Some(&allowed),
        )?,
        "semantic" => {
            let embedding = if let Some(value) = input.get("embedding") {
                value
                    .as_array()
                    .ok_or_else(|| invalid("embedding must be an array"))?
                    .iter()
                    .map(|v| {
                        v.as_f64()
                            .filter(|n| n.is_finite())
                            .map(|n| n as f32)
                            .filter(|n| n.is_finite())
                            .ok_or_else(|| invalid("embedding components must be finite numbers"))
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                store
                    .embed_query(
                        query.ok_or_else(|| {
                            invalid("semantic ranking requires query or embedding")
                        })?,
                    )?
                    .ok_or_else(|| invalid("no embedding provider configured"))?
            };
            if embedding.is_empty() {
                return Err(invalid("embedding must not be empty"));
            }
            super::structured_rank::score_candidates(store, &allowed, &embedding, at, limit)?
        }
        _ => return Err(invalid("structured mode must be semantic or keyword")),
    };
    let prefixes = crate::compact::PrefixMap::from_store(store)?;
    let verbose = input
        .get("verbose")
        .and_then(Json::as_bool)
        .unwrap_or(false);
    let results: Vec<_> = matches.into_iter().map(|m| {
        let entity = store.resolve(m.entity_id)?;
        Ok(json!({"entity": if verbose { entity } else { prefixes.compact(&entity) }, "text":m.text,"score":m.score,"similarity":m.score,"source":"knowledge","ranking_reason":mode,"valid_from":m.valid_from,"valid_to":m.valid_to}))
    }).collect::<Result<_>>()?;
    Ok(
        json!({"count":results.len(),"results":results,"scoped":true,"ranking":mode,"structured":{"candidates":allowed.len(),"complete":true,"graph":"ROOT"}}),
    )
}
#[cfg(test)]
#[path = "structured_search_tests.rs"]
mod tests;
