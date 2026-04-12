//! Episode ingestion — structured write path for agent-extracted knowledge.
//!
//! Episodes are the primary unit of knowledge ingestion from Gas Town agents.
//! An episode contains a set of nodes (entities) and edges (relationships)
//! extracted from operational events, bead work, or infrastructure observations.
//!
//! This module converts the structured JSON format used by Graphiti/Gas Town
//! into RDF Turtle and writes it through the existing `ingest_rdf` pipeline.

use serde::Deserialize;

#[cfg(feature = "shacl")]
use crate::error::Error;
use crate::error::Result;
use crate::namespace;
use crate::rdf::ingest_rdf;
use crate::resolution::{self, EntityCandidate};
#[cfg(feature = "shacl")]
use crate::shacl;
use crate::store::Store;

/// Options controlling entity resolution during episode ingest.
#[derive(Debug, Clone)]
pub struct IngestResolutionOpts {
    /// Whether resolution is enabled.
    pub enabled: bool,
    /// Similarity threshold for candidate matches.
    pub threshold: f64,
    /// Maximum candidates per entity.
    pub top_k: usize,
    /// When true, reject writes with near-duplicate candidates.
    pub strict_mode: bool,
}

/// Result of episode ingestion, including resolution hints.
#[derive(Debug)]
pub struct IngestResult {
    /// Transaction ID.
    pub tx_id: i64,
    /// Number of triples written.
    pub count: usize,
    /// Per-node resolution candidates (node name → candidates).
    /// Only populated when resolution is enabled and matches were found.
    pub resolution_hints: Vec<(String, Vec<EntityCandidate>)>,
}

/// Ingest an episode into the store with optional entity resolution.
///
/// When `resolution_opts` is provided and enabled, each node is checked
/// against existing entities before writing. In strict mode, the write is
/// rejected if near-duplicates are found.
pub fn ingest_episode_with_resolution(
    store: &mut Store,
    episode: &Episode,
    timestamp: &str,
    base_ns: &str,
    resolution_opts: Option<&IngestResolutionOpts>,
) -> Result<IngestResult> {
    let mut resolution_hints = Vec::new();

    // Run entity resolution for each node if enabled.
    if let Some(opts) = resolution_opts
        && opts.enabled
    {
        for node in &episode.nodes {
            let properties: Vec<(String, String)> = node
                .description
                .as_ref()
                .map(|d| vec![("description".to_string(), d.clone())])
                .unwrap_or_default();

            let result = resolution::resolve_entity(
                store,
                &node.name,
                &properties,
                opts.threshold,
                opts.top_k,
            )?;

            if result.has_matches {
                if opts.strict_mode {
                    let top = &result.candidates[0];
                    return Err(crate::error::Error::InvalidValue(format!(
                        "entity resolution: '{}' matches existing entity '{}' \
                             (score: {:.2}, matched by: {}). Use an existing IRI or \
                             assert quipu:distinctFrom to override.",
                        node.name, top.iri, top.score, top.matched_on,
                    )));
                }
                resolution_hints.push((node.name.clone(), result.candidates));
            }
        }
    }

    let (tx_id, count) = ingest_episode(store, episode, timestamp, base_ns)?;

    Ok(IngestResult {
        tx_id,
        count,
        resolution_hints,
    })
}

/// An episode — a unit of knowledge to ingest.
#[derive(Debug, Deserialize)]
pub struct Episode {
    pub name: String,
    #[serde(default)]
    pub episode_body: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
    /// Optional SHACL shapes (Turtle) to validate generated triples against.
    #[serde(default)]
    pub shapes: Option<String>,
}

/// A node (entity) extracted from an episode.
#[derive(Debug, Deserialize)]
pub struct Node {
    pub name: String,
    #[serde(rename = "type", default)]
    pub node_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub properties: Option<serde_json::Map<String, serde_json::Value>>,
}

/// An edge (relationship) between two nodes.
#[derive(Debug, Deserialize)]
pub struct Edge {
    pub source: String,
    pub target: String,
    pub relation: String,
}

/// Ingest an episode into the store.
///
/// Converts nodes and edges to Turtle and writes via `ingest_rdf`.
/// Returns `(tx_id, triple_count)`.
pub fn ingest_episode(
    store: &mut Store,
    episode: &Episode,
    timestamp: &str,
    base_ns: &str,
) -> Result<(i64, usize)> {
    let turtle = episode_to_turtle(episode, base_ns);

    // SHACL validation gate: if shapes provided, validate before writing.
    #[cfg(feature = "shacl")]
    if let Some(shapes) = &episode.shapes {
        let feedback = shacl::validate_shapes(shapes, &turtle)?;
        if !feedback.conforms {
            let messages: Vec<String> = feedback
                .results
                .iter()
                .map(|r| {
                    format!(
                        "{}: {} ({})",
                        r.severity,
                        r.message.as_deref().unwrap_or("no message"),
                        r.focus_node
                    )
                })
                .collect();
            return Err(Error::ValidationFailed {
                violations: feedback.violations,
                messages,
            });
        }
    }

    let actor = episode.source.as_deref();
    let source_str = format!("episode:{}", episode.name);

    ingest_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        timestamp,
        actor,
        Some(&source_str),
    )
}

/// Ingest multiple episodes in sequence, each as its own transaction.
/// Stops on first error.
pub fn ingest_batch(
    store: &mut Store,
    episodes: &[Episode],
    timestamps: &[&str],
    base_ns: &str,
) -> Result<Vec<(i64, usize)>> {
    let mut results = Vec::with_capacity(episodes.len());
    for (i, episode) in episodes.iter().enumerate() {
        let ts = timestamps.get(i).copied().unwrap_or("1970-01-01T00:00:00Z");
        results.push(ingest_episode(store, episode, ts, base_ns)?);
    }
    Ok(results)
}

/// Query all entities generated by a specific episode via prov:wasGeneratedBy.
pub fn episode_provenance(
    store: &Store,
    episode_name: &str,
    base_ns: &str,
) -> Result<Vec<(String, Vec<crate::types::Fact>)>> {
    let ep_local = sanitize_iri_local(episode_name);
    let ep_iri = format!("{base_ns}episode_{ep_local}");
    let query = format!(
        "SELECT DISTINCT ?s WHERE {{ ?s <{}wasGeneratedBy> <{ep_iri}> }}",
        namespace::PROV,
    );
    let result = crate::sparql::query(store, &query)?;

    let mut entities = Vec::new();
    for row in result.rows() {
        if let Some(crate::types::Value::Ref(id)) = row.get("s") {
            let iri = store.resolve(*id)?;
            let facts = store.entity_facts(*id)?;
            entities.push((iri, facts));
        }
    }
    Ok(entities)
}

// ── Turtle generation ──────────────────────────────────────────

fn episode_to_turtle(episode: &Episode, base_ns: &str) -> String {
    let mut ttl = String::new();

    // Prefixes.
    ttl.push_str(&format!("@prefix aegis: <{base_ns}> .\n"));
    ttl.push_str(&format!("@prefix rdf: <{}> .\n", namespace::RDF));
    ttl.push_str(&format!("@prefix rdfs: <{}> .\n", namespace::RDFS));
    ttl.push_str(&format!("@prefix prov: <{}> .\n", namespace::PROV));
    ttl.push_str(&format!("@prefix xsd: <{}> .\n\n", namespace::XSD));

    let ep_local = sanitize_iri_local(&episode.name);

    // Episode provenance entity.
    ttl.push_str(&format!("aegis:episode_{ep_local} a prov:Activity ;\n"));
    ttl.push_str(&format!(
        "    rdfs:label \"{}\"",
        escape_turtle(&episode.name)
    ));
    if let Some(body) = &episode.episode_body {
        ttl.push_str(&format!(" ;\n    rdfs:comment \"{}\"", escape_turtle(body)));
    }
    if let Some(source) = &episode.source {
        ttl.push_str(&format!(
            " ;\n    prov:wasAssociatedWith \"{}\"",
            escape_turtle(source)
        ));
    }
    if let Some(gid) = &episode.group_id {
        ttl.push_str(&format!(" ;\n    aegis:groupId \"{}\"", escape_turtle(gid)));
    }
    ttl.push_str(" .\n\n");

    // Nodes.
    for node in &episode.nodes {
        let local = sanitize_iri_local(&node.name);
        ttl.push_str(&format!("aegis:{local}"));

        if let Some(ntype) = &node.node_type {
            let type_local = sanitize_iri_local(ntype);
            ttl.push_str(&format!(" a aegis:{type_local}"));
        }

        ttl.push_str(&format!(
            " ;\n    rdfs:label \"{}\"",
            escape_turtle(&node.name)
        ));

        if let Some(desc) = &node.description {
            ttl.push_str(&format!(" ;\n    rdfs:comment \"{}\"", escape_turtle(desc)));
        }

        // Link to episode provenance.
        ttl.push_str(&format!(
            " ;\n    prov:wasGeneratedBy aegis:episode_{ep_local}"
        ));

        // Optional properties as typed literals.
        if let Some(props) = &node.properties {
            for (key, val) in props {
                let pred = sanitize_iri_local(key);
                match val {
                    serde_json::Value::String(s) => {
                        ttl.push_str(&format!(" ;\n    aegis:{pred} \"{}\"", escape_turtle(s)));
                    }
                    serde_json::Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            ttl.push_str(&format!(" ;\n    aegis:{pred} \"{i}\"^^xsd:integer"));
                        } else if let Some(f) = n.as_f64() {
                            ttl.push_str(&format!(" ;\n    aegis:{pred} \"{f}\"^^xsd:double"));
                        }
                    }
                    serde_json::Value::Bool(b) => {
                        ttl.push_str(&format!(" ;\n    aegis:{pred} \"{b}\"^^xsd:boolean"));
                    }
                    _ => {} // skip arrays/objects/null
                }
            }
        }

        ttl.push_str(" .\n\n");
    }

    // Edges.
    for edge in &episode.edges {
        let src = sanitize_iri_local(&edge.source);
        let tgt = sanitize_iri_local(&edge.target);
        let rel = sanitize_iri_local(&edge.relation);
        ttl.push_str(&format!("aegis:{src} aegis:{rel} aegis:{tgt} .\n"));
    }

    ttl
}

/// Sanitize a name into a valid IRI local name.
fn sanitize_iri_local(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Escape special characters for Turtle string literals.
fn escape_turtle(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests;
