//! Render-ready graph projection — the single payload the UI draws from.
//!
//! The explorer used to pull `/cord` (every entity with every literal fact,
//! capped at 5000) and then derive the node-link graph in the browser: scan all
//! facts to find relations, resolve targets, compute degree, rank, cap. That is
//! O(entities x facts) in JavaScript, re-run on every filter change, over a
//! payload dominated by literals the graph never draws.
//!
//! This does that work once, in Rust, against the fact log, and returns exactly
//! what a renderer needs. Edges reference nodes by INDEX rather than IRI, which
//! is most of the size win: an IRI averages ~45 bytes and appears in every edge
//! endpoint, so a 2000-edge graph carries ~180KB of repeated strings before
//! this and ~16KB after.

use std::collections::{HashMap, HashSet};

use serde_json::{Value as JsonValue, json};

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::Value;

/// Default node budget. Past this the graph stops being readable before it
/// stops being fast, so the cap is a legibility limit, not a perf one.
const DEFAULT_LIMIT: usize = 250;

/// Hard ceiling on the node budget a caller may request.
const MAX_LIMIT: usize = 2000;

/// Default edge budget. The node cap alone stopped bounding the payload once
/// OWL materialization densified the store: renderers pay O(edges) per frame
/// (spring attraction and line drawing both walk the whole edge list), so
/// edges are what hang the tab. Measured live 2026-08-07: 2000 nodes carried
/// 105,230 edges (2.4MB) and the UI would not load; 250 nodes still carried
/// 15,554.
const DEFAULT_EDGE_BUDGET: usize = 4_000;

/// Hard ceiling on the edge budget a caller may request.
const MAX_EDGE_BUDGET: usize = 50_000;

/// A predicate is a domain *relation* — a drawable edge — when it is none of
/// the scaffolding vocabularies. `rdf:type` is a node attribute, `rdfs:*` is
/// labels/comments, and `prov:*` is the provenance spine that otherwise buries
/// the domain graph under episode nodes.
fn is_relation_predicate(iri: &str) -> bool {
    !iri.starts_with(namespace::RDF)
        && !iri.starts_with(namespace::RDFS)
        && !iri.starts_with(namespace::PROV)
}

/// One entity's drawable attributes, accumulated in a single pass.
#[derive(Default)]
struct NodeAcc {
    label: Option<String>,
    type_iri: Option<String>,
    degree: usize,
}

/// MCP tool / `POST /graph`: project the store into a render-ready node-link
/// payload.
///
/// Input: `{ "limit": N, "edge_budget": N, "type": "<IRI>", "include_episodes": bool }`
///
/// Output:
/// ```json
/// {
///   "nodes": [{"iri": "…", "label": "…", "type": "…", "deg": 3}],
///   "edges": [[0, 1, "runs_on"]],
///   "types": [{"iri": "…", "count": 12}],
///   "truncated": {"shown": 250, "of": 1180},
///   "stats": {"nodes": 250, "edges": 612}
/// }
/// ```
///
/// `edges` are `[source_index, target_index, short_predicate]` into `nodes`.
/// `types` is ordered by descending count so the UI can assign its categorical
/// palette by prevalence and fold the tail into one "Other" slot — the palette
/// is a fixed order of eight, never a cycled hue per type.
pub fn tool_graph_view(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let limit = input
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map_or(DEFAULT_LIMIT, |n| (n as usize).min(MAX_LIMIT));
    let edge_budget = input
        .get("edge_budget")
        .and_then(serde_json::Value::as_u64)
        .map_or(DEFAULT_EDGE_BUDGET, |n| (n as usize).min(MAX_EDGE_BUDGET));
    let type_filter = input.get("type").and_then(|v| v.as_str());
    let include_episodes = input
        .get("include_episodes")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);

    let facts = store.current_facts()?;

    let rdf_type = store.lookup(namespace::RDF_TYPE)?;
    let rdfs_label = store.lookup(namespace::RDFS_LABEL)?;

    // Pass 1: node attributes (type + label), and the set of entities that are
    // episodes — provenance activities the domain graph should not draw.
    let mut acc: HashMap<i64, NodeAcc> = HashMap::new();
    let mut episodes: HashSet<i64> = HashSet::new();
    let prov_activity = format!("{}Activity", namespace::PROV);

    for f in &facts {
        if Some(f.attribute) == rdf_type
            && let Value::Ref(tid) = &f.value
        {
            let tiri = store.resolve(*tid)?;
            if tiri == prov_activity {
                episodes.insert(f.entity);
            }
            acc.entry(f.entity).or_default().type_iri = Some(tiri);
        } else if Some(f.attribute) == rdfs_label
            && let Some(text) = f.value.as_lexical()
        {
            let e = acc.entry(f.entity).or_default();
            if e.label.is_none() {
                e.label = Some(text.to_string());
            }
        } else {
            acc.entry(f.entity).or_default();
        }
    }

    let drawable = |eid: i64, acc: &HashMap<i64, NodeAcc>| -> bool {
        if !include_episodes && episodes.contains(&eid) {
            return false;
        }
        match type_filter {
            None => true,
            Some(t) => acc.get(&eid).and_then(|a| a.type_iri.as_deref()) == Some(t),
        }
    };

    // Pass 2: relation edges between drawable entities, and the degree that
    // decides which nodes survive the cap.
    let mut raw_edges: Vec<(i64, i64, String)> = Vec::new();
    let mut seen_edge: HashSet<(i64, i64, i64)> = HashSet::new();
    for f in &facts {
        let Value::Ref(target) = &f.value else {
            continue;
        };
        if !acc.contains_key(target) {
            continue; // object is a bare term (a type/class), not an entity
        }
        let pred = store.resolve(f.attribute)?;
        if !is_relation_predicate(&pred) {
            continue;
        }
        if !drawable(f.entity, &acc) || !drawable(*target, &acc) {
            continue;
        }
        if !seen_edge.insert((f.entity, f.attribute, *target)) {
            continue;
        }
        raw_edges.push((f.entity, *target, short_iri(&pred)));
        acc.entry(f.entity).or_default().degree += 1;
        acc.entry(*target).or_default().degree += 1;
    }

    // Rank by degree, then IRI for a stable order across identical degrees so
    // the same store always renders the same graph.
    let mut candidates: Vec<i64> = acc.keys().copied().filter(|e| drawable(*e, &acc)).collect();
    let total_candidates = candidates.len();
    let mut iri_of: HashMap<i64, String> = HashMap::new();
    for e in &candidates {
        iri_of.insert(*e, store.resolve(*e)?);
    }
    candidates.sort_by(|a, b| {
        let da = acc.get(a).map_or(0, |n| n.degree);
        let db = acc.get(b).map_or(0, |n| n.degree);
        db.cmp(&da).then_with(|| iri_of[a].cmp(&iri_of[b]))
    });
    candidates.truncate(limit);

    let index: HashMap<i64, usize> = candidates
        .iter()
        .enumerate()
        .map(|(i, e)| (*e, i))
        .collect();

    let nodes: Vec<JsonValue> = candidates
        .iter()
        .map(|e| {
            let a = acc.get(e);
            let iri = &iri_of[e];
            json!({
                "iri": iri,
                "label": a.and_then(|n| n.label.clone()).unwrap_or_else(|| short_iri(iri)),
                "type": a.and_then(|n| n.type_iri.clone()),
                "deg": a.map_or(0, |n| n.degree),
            })
        })
        .collect();

    // Only edges whose BOTH endpoints survived the cap. An edge to a dropped
    // node would render as an arrow into empty space.
    let mut kept: Vec<(usize, usize, &str)> = raw_edges
        .iter()
        .filter_map(|(s, t, p)| match (index.get(s), index.get(t)) {
            (Some(&si), Some(&ti)) => Some((si, ti, p.as_str())),
            _ => None,
        })
        .collect();
    let edges_total = kept.len();
    // Over budget, keep the edges among the highest-ranked nodes (index IS the
    // degree rank), dropping periphery first — deterministic, so the same
    // store always draws the same graph.
    if kept.len() > edge_budget {
        kept.sort_by(|a, b| {
            a.0.max(a.1)
                .cmp(&b.0.max(b.1))
                .then_with(|| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)))
        });
        kept.truncate(edge_budget);
    }
    let edges: Vec<JsonValue> = kept.iter().map(|(si, ti, p)| json!([si, ti, p])).collect();

    // Type census over the DRAWN nodes, descending — the UI assigns its fixed
    // eight-slot categorical order by this rank and folds the tail into one
    // neutral "Other" slot.
    let mut type_counts: HashMap<&str, usize> = HashMap::new();
    for e in &candidates {
        if let Some(t) = acc.get(e).and_then(|n| n.type_iri.as_deref()) {
            *type_counts.entry(t).or_insert(0) += 1;
        }
    }
    let mut types: Vec<(&str, usize)> = type_counts.into_iter().collect();
    types.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let types: Vec<JsonValue> = types
        .iter()
        .map(|(iri, n)| json!({"iri": iri, "label": short_iri(iri), "count": n}))
        .collect();

    Ok(json!({
        "nodes": nodes,
        "edges": edges,
        "types": types,
        // Never a silent cap: the UI says so on screen when this fires.
        "truncated": {
            "shown": candidates.len(),
            "of": total_candidates,
            "edges_shown": edges.len(),
            "edges_of": edges_total,
        },
        "stats": {"nodes": nodes.len(), "edges": edges.len()},
    }))
}

/// Last path/fragment segment of an IRI — the display form.
fn short_iri(iri: &str) -> String {
    iri.rsplit(['#', '/']).next().unwrap_or(iri).to_string()
}

#[cfg(test)]
mod tests;
