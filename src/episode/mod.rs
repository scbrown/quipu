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

impl IngestResolutionOpts {
    /// Build ingest options from the store's `[quipu.resolution]` config so the
    /// episode write paths honour the configured dedup policy (hq-uye).
    pub fn from_config(cfg: &crate::config::ResolutionConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            threshold: cfg.threshold,
            top_k: cfg.top_k,
            strict_mode: cfg.strict_mode,
        }
    }
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
    /// Optional confidence qualifier for the generated triple (hq-cug6, aegis-1p0
    /// Gap 5). Lets agents flag uncertain AUTO-extracted facts for review. Accepts
    /// an enum grade (`"EXTRACTED"`/`"INFERRED"`/`"AMBIGUOUS"`) or a 0–1 number;
    /// when present the triple is reified and qualified with `quipu:confidence`.
    /// Absent (the common case) = unqualified bare triple, fully back-compatible.
    #[serde(default)]
    pub confidence: Option<serde_json::Value>,
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
    // Idempotency key (hq-fhc). The episode activity IRI is derived purely from
    // the episode name, so re-ingesting the same name targets the same node. We
    // stamp the activity with a content hash: identical re-ingests are no-ops,
    // and a changed episode retracts the activity's stale provenance facts
    // before re-asserting, instead of accumulating duplicate activity nodes.
    let new_hash = episode_content_hash(episode);
    let ep_local = sanitize_iri_local(&episode.name);
    let ep_iri = format!("{base_ns}episode_{ep_local}");
    let turtle = episode_to_turtle(episode, base_ns, &new_hash);

    // SHACL validation gates, run before any write — and before the idempotency
    // short-circuit, so a no-op re-ingest is still validated (e.g. if
    // validate_on_write was toggled on since the last ingest).
    #[cfg(feature = "shacl")]
    {
        // Shapes carried inline on the episode (existing behaviour).
        if let Some(shapes) = &episode.shapes {
            shacl_validate_or_reject(shapes, &turtle)?;
        }
        // Persistently-loaded shapes, when write-validation is enabled (hq-c6s).
        // Without this, stored shapes only gate the `knot` path and episode
        // writes go unvalidated — undermining quipu's "start strict" thesis.
        if store.shacl_config().validate_on_write
            && let Some(stored) = store.get_combined_shapes()?
        {
            shacl_validate_or_reject(&stored, &turtle)?;
        }
    }

    let existing_hash = current_content_hash(store, &ep_iri, base_ns)?;

    // Idempotency fast path: same content already recorded → skip the write.
    if existing_hash.as_deref() == Some(new_hash.as_str()) {
        return Ok((NOOP_TX, 0));
    }

    let actor = episode.source.as_deref();

    // Existing episode whose content changed: retract the activity's prior
    // facts (label/comment/source/groupId/contentHash) so the update replaces
    // them rather than leaving stale active values. Only the activity node is
    // retracted; its generated entities are reconciled by fact-level dedup.
    // SHACL has already passed above, so this never half-mutates on rejection.
    if existing_hash.is_some()
        && let Some(ep_id) = store.lookup(&ep_iri)?
    {
        store.retract_entity(ep_id, None, timestamp, actor)?;
    }

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

/// Transaction id returned when an episode ingest is a no-op (the identical
/// content was already recorded). Distinguishable from a real tx, which is
/// always positive.
pub const NOOP_TX: i64 = 0;

/// Read the content hash currently stamped on an episode activity, if any.
fn current_content_hash(store: &Store, ep_iri: &str, base_ns: &str) -> Result<Option<String>> {
    let query = format!("SELECT ?h WHERE {{ <{ep_iri}> <{base_ns}contentHash> ?h }} LIMIT 1");
    let result = crate::sparql::query(store, &query)?;
    Ok(result.rows().first().and_then(|row| match row.get("h") {
        Some(crate::types::Value::Str(s)) => Some(s.clone()),
        _ => None,
    }))
}

/// A stable content hash of an episode's asserted data (name, body, source,
/// group, and its nodes/edges). Node and edge ordering is normalised so that
/// reordering alone does not defeat idempotency. SHACL `shapes` are excluded —
/// they gate validation but are not part of the asserted graph.
///
/// Uses FNV-1a so the digest is deterministic across processes and Rust
/// versions (unlike `DefaultHasher`), which matters because the value is
/// persisted and compared on later runs.
fn episode_content_hash(episode: &Episode) -> String {
    let mut parts: Vec<String> = vec![
        format!("name={}", episode.name),
        format!("body={}", episode.episode_body.as_deref().unwrap_or("")),
        format!("source={}", episode.source.as_deref().unwrap_or("")),
        format!("group={}", episode.group_id.as_deref().unwrap_or("")),
    ];

    let mut nodes: Vec<String> = episode
        .nodes
        .iter()
        .map(|n| {
            let mut props: Vec<String> = n
                .properties
                .as_ref()
                .map(|m| m.iter().map(|(k, v)| format!("{k}={v}")).collect())
                .unwrap_or_default();
            props.sort();
            format!(
                "node:{}|{}|{}|{}",
                n.name,
                n.node_type.as_deref().unwrap_or(""),
                n.description.as_deref().unwrap_or(""),
                props.join(",")
            )
        })
        .collect();
    nodes.sort();

    let mut edges: Vec<String> = episode
        .edges
        .iter()
        .map(|e| {
            format!(
                "edge:{}|{}|{}|{}",
                e.source,
                e.relation,
                e.target,
                e.confidence
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default()
            )
        })
        .collect();
    edges.sort();

    parts.extend(nodes);
    parts.extend(edges);

    format!("{:016x}", fnv1a_64(parts.join("\n").as_bytes()))
}

/// FNV-1a 64-bit hash — small, dependency-free, and deterministic across runs.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Validate `data_turtle` against `shapes_turtle`, returning a `ValidationFailed`
/// error that lists the violations when it does not conform (hq-c6s). Shared by
/// the inline-shapes and persistent-shapes gates in `ingest_episode`.
#[cfg(feature = "shacl")]
fn shacl_validate_or_reject(shapes_turtle: &str, data_turtle: &str) -> Result<()> {
    let feedback = shacl::validate_shapes(shapes_turtle, data_turtle)?;
    if feedback.conforms {
        return Ok(());
    }
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
    Err(Error::ValidationFailed {
        violations: feedback.violations,
        messages,
    })
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
    let now = crate::time::now_iso();
    for (i, episode) in episodes.iter().enumerate() {
        let ts = timestamps.get(i).copied().unwrap_or(now.as_str());
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

fn episode_to_turtle(episode: &Episode, base_ns: &str, content_hash: &str) -> String {
    let mut ttl = String::new();

    // Prefixes.
    ttl.push_str(&format!("@prefix aegis: <{base_ns}> .\n"));
    ttl.push_str(&format!("@prefix rdf: <{}> .\n", namespace::RDF));
    ttl.push_str(&format!("@prefix rdfs: <{}> .\n", namespace::RDFS));
    ttl.push_str(&format!("@prefix prov: <{}> .\n", namespace::PROV));
    ttl.push_str(&format!("@prefix quipu: <{}> .\n", namespace::QUIPU));
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
    // Idempotency key for re-ingest detection (hq-fhc).
    ttl.push_str(&format!(" ;\n    aegis:contentHash \"{content_hash}\""));
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

        // Optional confidence qualifier (hq-cug6). The bare triple above is always
        // asserted (back-compat); when a confidence is supplied we additionally
        // reify the statement so it can carry the qualifier and stay SPARQL-
        // queryable. The reification IRI is derived deterministically from the
        // triple so re-ingest dedups at fact level rather than accumulating.
        if let Some(conf) = &edge.confidence
            && let Some(literal) = confidence_literal(conf)
        {
            let stmt_hash = format!("{:016x}", fnv1a_64(format!("{src}|{rel}|{tgt}").as_bytes()));
            ttl.push_str(&format!("aegis:stmt_{stmt_hash} a rdf:Statement ;\n"));
            ttl.push_str(&format!("    rdf:subject aegis:{src} ;\n"));
            ttl.push_str(&format!("    rdf:predicate aegis:{rel} ;\n"));
            ttl.push_str(&format!("    rdf:object aegis:{tgt} ;\n"));
            ttl.push_str(&format!("    quipu:confidence {literal} .\n"));
        }
    }

    ttl
}

/// Render a confidence value as a Turtle literal, or `None` to skip it.
///
/// A string (e.g. `"EXTRACTED"`) becomes a plain quoted literal; a number (0–1)
/// becomes an `xsd:decimal`. Other JSON shapes (bool/array/object/null) are
/// ignored so a malformed field never corrupts the graph.
fn confidence_literal(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(s) => Some(format!("\"{}\"", escape_turtle(s))),
        serde_json::Value::Number(n) => n.as_f64().map(|f| format!("\"{f}\"^^xsd:decimal")),
        _ => None,
    }
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
