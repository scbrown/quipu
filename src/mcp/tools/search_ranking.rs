//! Query-time demotion of repository artifacts that contain only identifiers.
//!
//! This is a relevance heuristic, not a trust or governance classification.
//! Keep the underlying cosine score visible and allow callers to opt out.

use crate::error::{Error, Result};
use crate::store::Store;
use crate::types::Value;
use crate::vector::VectorMatch;
use serde_json::Value as JsonValue;

pub(super) fn ranking_mode(input: &JsonValue) -> Result<bool> {
    match input.get("ranking") {
        None => Ok(false),
        Some(JsonValue::String(s)) if s == "content" => Ok(true),
        Some(JsonValue::String(s)) if s == "semantic" => Ok(false),
        _ => Err(Error::InvalidValue(
            "ranking must be 'content' or 'semantic'".into(),
        )),
    }
}

pub(super) struct RankedMatch {
    pub matched: VectorMatch,
    pub score: f64,
    pub demoted: bool,
}

pub(super) fn rank(
    store: &Store,
    matches: Vec<VectorMatch>,
    content: bool,
    valid_at: Option<&str>,
    query: Option<&str>,
) -> Result<Vec<RankedMatch>> {
    let mut ranked = Vec::with_capacity(matches.len());
    for matched in matches {
        let demoted = content && is_stub(store, matched.entity_id, valid_at, query)?;
        // Halve positive similarity; negative matches must never be promoted.
        let score = if demoted {
            matched.score - matched.score.abs() * 0.5
        } else {
            matched.score
        };
        ranked.push(RankedMatch {
            matched,
            score,
            demoted,
        });
    }
    // Stable ordering preserves the backend's tie order, including semantic mode.
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(ranked)
}

fn lexical(value: &Value) -> Option<&str> {
    match value {
        Value::Str(s) | Value::Lang { lexical: s, .. } | Value::Typed { lexical: s, .. } => Some(s),
        _ => None,
    }
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn is_stub(
    store: &Store,
    entity: i64,
    valid_at: Option<&str>,
    query: Option<&str>,
) -> Result<bool> {
    // Restrict to the candidate entity and ROOT, including when time travelling.
    // Do not scan the graph or use today's description to rank yesterday's match.
    let facts = if let Some(at) = valid_at {
        let mut statement = store.conn.prepare(
            "SELECT e,a,v,tx,valid_from,valid_to,op FROM facts \
             WHERE e=?1 AND g=0 AND op=1 AND valid_from<=?2 \
             AND (valid_to IS NULL OR valid_to>?2) ORDER BY a",
        )?;
        Store::collect_facts(&mut statement, rusqlite::params![entity, at])?
    } else {
        store.entity_facts(entity)?
    };
    let base = store.base_ns();
    let type_id = store.lookup(crate::namespace::RDF_TYPE)?;
    let mut artifact = false;
    let mut names = Vec::new();
    let mut descriptions = Vec::new();
    for fact in &facts {
        if Some(fact.attribute) == type_id {
            if let Value::Ref(id) = fact.value {
                let iri = store.resolve(id)?;
                artifact |= ["Section", "Chunk", "CodeSymbol"]
                    .iter()
                    .any(|kind| iri == format!("{base}{kind}"));
            }
            continue;
        }
        let Some(text) = lexical(&fact.value) else {
            continue;
        };
        let predicate = store.resolve(fact.attribute)?;
        let text = normalized(text);
        if predicate == format!("{}label", crate::namespace::RDFS)
            || ["heading", "name", "symbolName"]
                .iter()
                .any(|p| predicate == format!("{base}{p}"))
        {
            names.push(text);
        } else if predicate == format!("{}comment", crate::namespace::RDFS)
            || [
                "content",
                "body",
                "text",
                "description",
                "summary",
                "docstring",
                "documentation",
            ]
            .iter()
            .any(|p| predicate == format!("{base}{p}"))
        {
            descriptions.push(text);
        }
    }
    // Unknown types and ordinary knowledge entities retain their semantic score.
    // Paths, line numbers, revisions and symbol kinds are not explanatory content.
    let exact_name = query.is_some_and(|q| names.contains(&normalized(q)));
    Ok(artifact
        && !exact_name
        && !descriptions
            .iter()
            .any(|text| !text.is_empty() && !names.contains(text)))
}

#[cfg(test)]
mod tests;
