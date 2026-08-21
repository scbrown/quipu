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
use crate::rdf::{ingest_rdf_to_graph, parse_rdf};
use crate::resolution::{self, Contention, EntityCandidate};
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
    /// What the ingest actually DID: `created`, `updated`, or `unchanged`.
    /// See [`IngestOutcome`] — this exists because the idempotent no-op was
    /// indistinguishable from a failed write.
    pub outcome: IngestOutcome,
    /// Per-node resolution candidates (node name → candidates).
    /// Only populated when resolution is enabled and matches were found.
    pub resolution_hints: Vec<(String, Vec<EntityCandidate>)>,
    /// Existing entities claimed by MORE THAN ONE node of this episode: the
    /// write is about to fragment one entity. See `resolution::matching`.
    pub resolution_contentions: Vec<Contention>,
}

/// What an `/episode` ingest did, so a caller can tell "already recorded" from
/// "nothing was written".
///
/// `/episode` has been idempotent since hq-fhc: the activity IRI derives from the
/// episode name and carries a content hash, so re-posting identical content is a
/// no-op. That is correct, and it makes retrying after a lost response SAFE.
///
/// **But the no-op returned `count: 0, tx_id: 0` — byte-for-byte what a write
/// that achieved nothing returns** — while the documented success check across
/// every caller of this API is "HTTP 200 with `count > 0`". So the successful
/// retry reported as a failure. And the natural recovery from "my episode did
/// not land" is to re-post under a different name or with re-worded nodes, which
/// is exactly the entity fragmentation this store already carries beads about.
/// The safe mechanism was steering callers into the unsafe action.
///
/// A mechanism that is correct and reports itself ambiguously is not one you can
/// act on. Branch on this field, never on `count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestOutcome {
    /// The episode did not exist; its facts were written.
    Created,
    /// The episode existed with DIFFERENT content: stale activity facts were
    /// retracted and the new content written.
    Updated,
    /// The episode already existed with identical content. Nothing was written
    /// and nothing needed to be. **This is success** — the facts are in the
    /// store, and a caller that retried after a lost response has its answer.
    Unchanged,
}

impl IngestOutcome {
    /// Stable wire string for the JSON response.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Unchanged => "unchanged",
        }
    }
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
    let mut resolution_contentions = Vec::new();

    // Resolve every node in ONE pass: it scans the label set once for the whole
    // episode rather than once per node, and it can see two nodes claiming one
    // existing entity — which a per-node loop structurally cannot.
    if let Some(opts) = resolution_opts
        && opts.enabled
    {
        let resolved = resolution::resolve_episode_nodes(store, &episode.nodes, base_ns, opts)?;
        if let Some(msg) = resolved.refusal {
            return Err(crate::error::Error::InvalidValue(msg));
        }
        resolution_hints = resolved.hints;
        resolution_contentions = resolved.contentions;
    }

    let (tx_id, count, outcome) = ingest_episode_outcome(store, episode, timestamp, base_ns)?;

    Ok(IngestResult {
        tx_id,
        count,
        outcome,
        resolution_hints,
        resolution_contentions,
    })
}

/// The IRI an episode node is written as. One definition, so resolution reads
/// back `quipu:distinctFrom` under the same IRI the Turtle generator asserts it.
pub(crate) fn node_iri(name: &str, base_ns: &str) -> String {
    format!("{base_ns}{}", sanitize_iri_local(name))
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
    /// Optional named graph to write into (aegis-g1al / #36). Absent = ROOT (the
    /// source of truth). A tenant/agent overlay passes its graph IRI here; those
    /// facts land in that graph and extend ROOT without mutating it.
    #[serde(default)]
    pub graph: Option<String>,
    /// Optional SHACL shapes (Turtle) to validate generated triples against.
    #[serde(default)]
    pub shapes: Option<String>,
    /// Replace the complete set of facts previously asserted by this episode.
    /// Intended for producers that publish a current snapshot rather than an
    /// append-only knowledge event. Retractions and assertions commit in one
    /// transaction, so readers never observe an empty intermediate state.
    #[serde(default)]
    pub replace_snapshot: bool,
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
    /// IRIs this entity is DELIBERATELY not, asserted as `quipu:distinctFrom`:
    /// overrides a strict refusal for exactly these pairings, durably.
    #[serde(default, alias = "distinctFrom")]
    pub distinct_from: Vec<String>,
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
///
/// Prefer [`ingest_episode_outcome`] on any path that reports back to a caller:
/// this signature cannot distinguish "wrote nothing because it was already
/// there" from "wrote nothing", and that ambiguity is the whole point of the outcome field.
pub fn ingest_episode(
    store: &mut Store,
    episode: &Episode,
    timestamp: &str,
    base_ns: &str,
) -> Result<(i64, usize)> {
    let (tx, count, _) = ingest_episode_outcome(store, episode, timestamp, base_ns)?;
    Ok((tx, count))
}

/// Ingest an episode, reporting WHAT IT DID as well as how much it wrote.
///
/// Returns `(tx_id, triple_count, outcome)`. See [`IngestOutcome`] for why the
/// third element is not optional information.
pub fn ingest_episode_outcome(
    store: &mut Store,
    episode: &Episode,
    timestamp: &str,
    base_ns: &str,
) -> Result<(i64, usize, IngestOutcome)> {
    // Idempotency key (hq-fhc). The episode activity IRI is derived purely from
    // the episode name, so re-ingesting the same name targets the same node. We
    // stamp the activity with a content hash: identical re-ingests are no-ops,
    // and a changed episode retracts the activity's stale provenance facts
    // before re-asserting, instead of accumulating duplicate activity nodes.
    let new_hash = episode_content_hash(episode);
    let ep_local = sanitize_iri_local(&episode.name);
    let ep_iri = format!("{base_ns}episode_{ep_local}");

    // Every node must carry a non-empty type. An untyped node emits malformed
    // Turtle — `aegis:foo ;\n rdfs:label …`, a ';' with no predicate before it —
    // which fails to parse and 400s the WHOLE episode with a cryptic error,
    // silently discarding every well-formed node beside it (aegis-uqd8). During a
    // batch drain that loses entire episodes with no clue why. Fail loud and
    // specific instead: name the offending node so the caller can fix and re-POST.
    for node in &episode.nodes {
        if node.node_type.as_deref().map_or("", str::trim).is_empty() {
            return Err(crate::error::Error::InvalidValue(format!(
                "node '{}' has no type — every node requires a non-empty type. An \
                 untyped node produces malformed Turtle that discards the whole \
                 episode (aegis-uqd8).",
                node.name
            )));
        }
        // …and the type it does carry must survive unrewritten. Same pre-flight, same
        // reason: refuse the whole episode before any write rather than land a node
        // under a class no reader can query (aegis-vngta).
        if let Some(ntype) = &node.node_type {
            validate_node_type(&node.name, ntype)?;
        }
    }

    // Every edge relation must be representable without being rewritten. Before
    // this check, a foreign-vocabulary predicate was forced into aegis: and
    // sanitized — `rdfs:subClassOf` stored as `aegis:rdfs_subClassOf`, inert,
    // behind an HTTP 200 with a healthy count (aegis-kuotp). Validate the whole
    // episode before any write, same as the untyped-node gate above, so a bad
    // edge fails loudly and specifically instead of landing as a dead triple.
    for edge in &episode.edges {
        resolve_edge_predicate(&edge.relation)?;
    }

    let turtle = episode_to_turtle(episode, base_ns, &new_hash);

    // SHACL validation gates, run before any write — and before the idempotency
    // short-circuit, so a no-op re-ingest is still validated (e.g. if
    // validate_on_write was toggled on since the last ingest).
    #[cfg(feature = "shacl")]
    {
        // Shapes carried inline on the episode (existing behaviour).
        if let Some(shapes) = &episode.shapes {
            shacl_validate_or_reject(store, shapes, &turtle)?;
        }
        // Persistently-loaded shapes, when write-validation is enabled (hq-c6s).
        // Without this, stored shapes only gate the `knot` path and episode
        // writes go unvalidated — undermining quipu's "start strict" thesis.
        //
        // Event P3 (event-based design §5/§7): shapes route by their
        // `quipu:onViolation` annotation. DEFAULT REJECT — unannotated shapes
        // gate the tx exactly as before (decided, design §9.3). A shape annotated
        // `"emit"` observes instead: its violations become `shacl.violation`
        // events appended INSIDE the write's savepoint, and the write commits.
        if store.shacl_config().validate_on_write
            && let Some(stored) = store.get_combined_shapes()?
        {
            let split = shacl::split_shapes_by_policy(&stored);
            shacl_validate_or_reject(store, &split.reject, &turtle)?;
            if split.has_emit {
                let feedback =
                    crate::shacl_context::validate_with_store_context(store, &split.emit, &turtle)?;
                if !feedback.conforms {
                    for issue in &feedback.results {
                        store.queue_write_event(crate::store::PendingWriteEvent {
                            event_type: "shacl.violation".to_string(),
                            subject: Some(issue.focus_node.clone()),
                            payload: serde_json::json!({
                                "shape": issue.source_shape,
                                "message": issue.message,
                                "component": issue.component,
                                "path": issue.path,
                                "severity": issue.severity,
                                "mode": "emit",
                            }),
                        });
                    }
                }
            }
        }
    }

    let existing_hash = current_content_hash(store, &ep_iri, base_ns)?;

    // Idempotency fast path: same content already recorded → skip the write.
    // Reported as `Unchanged`, NOT as a bare `count: 0` — see `IngestOutcome`.
    if existing_hash.as_deref() == Some(new_hash.as_str()) {
        return Ok((NOOP_TX, 0, IngestOutcome::Unchanged));
    }

    let actor = episode.source.as_deref();

    // Existing episode whose content changed: retract the activity's prior
    // facts (label/comment/source/groupId/contentHash) so the update replaces
    // them rather than leaving stale active values. Only the activity node is
    // retracted; its generated entities are reconciled by fact-level dedup.
    // SHACL has already passed above, so this never half-mutates on rejection.
    let outcome = if existing_hash.is_some() {
        IngestOutcome::Updated
    } else {
        IngestOutcome::Created
    };
    if existing_hash.is_some()
        && !episode.replace_snapshot
        && let Some(ep_id) = store.lookup(&ep_iri)?
    {
        store.retract_entity(ep_id, None, timestamp, actor)?;
    }

    let source_str = format!("episode:{}", episode.name);

    // Named graph (aegis-g1al / #36): intern the graph IRI to its term id and
    // write there. Absent = ROOT (g=0). The graph is itself an entity (its term
    // id), which is where #37 provenance (owner/tenant) attaches.
    let graph = match &episode.graph {
        Some(iri) if !iri.trim().is_empty() => store.intern(iri)?,
        _ => 0,
    };

    let (tx_id, count) = if episode.replace_snapshot {
        let mut datums = store.plan_episode_retraction(&episode.name, graph)?;
        let mut assertions = parse_rdf(
            store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            timestamp,
        )?;
        let count = assertions.len();
        // Facts present in both snapshots stay active. Besides avoiding needless
        // history churn, this is required by the fact-log key: one transaction
        // cannot contain both a Retract and Assert for the same (e, a, v).
        datums.retain(|old| {
            !assertions.iter().any(|new| {
                old.entity == new.entity && old.attribute == new.attribute && old.value == new.value
            })
        });
        datums.append(&mut assertions);
        let tx_id = store.transact_to_graph(&datums, timestamp, actor, Some(&source_str), graph)?;
        (tx_id, count)
    } else {
        ingest_rdf_to_graph(
            store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            timestamp,
            actor,
            Some(&source_str),
            graph,
        )?
    };
    Ok((tx_id, count, outcome))
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
        format!("replace_snapshot={}", episode.replace_snapshot),
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
///
/// Validated WITH THE STORE AS CONTEXT (aegis-fp17f): an episode references
/// entities it does not re-describe — that is what an edge to an existing node
/// is — so a payload-only `sh:class` check refuses correct writes for the
/// accident of what travelled with them. See `shacl_context`.
#[cfg(feature = "shacl")]
fn shacl_validate_or_reject(store: &Store, shapes_turtle: &str, data_turtle: &str) -> Result<()> {
    let feedback =
        crate::shacl_context::validate_with_store_context(store, shapes_turtle, data_turtle)?;
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
    ttl.push_str(&format!("@prefix xsd: <{}> .\n", namespace::XSD));
    // Declared so an edge relation may name them verbatim (aegis-kuotp). Keep in
    // lockstep with KNOWN_PREFIXES.
    ttl.push_str(&format!("@prefix owl: <{}> .\n", namespace::OWL));
    ttl.push_str(&format!("@prefix skos: <{}> .\n", namespace::SKOS));
    ttl.push_str(&format!("@prefix sh: <{}> .\n\n", namespace::SHACL));

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

        // Written as facts so the override outlives the write that made it.
        for other in &node.distinct_from {
            ttl.push_str(&format!(" ;\n    quipu:distinctFrom <{other}>"));
        }

        // Optional properties as typed literals. A JSON ARRAY yields one triple
        // per element — the natural RDF reading, and exactly what the /knot+Turtle
        // path's `a "x", "y" .` already does. Previously the array arm was a
        // silent no-op, so a multi-valued property (e.g. a CrewRole trait axis
        // that is MULTI by design) declared as a JSON array turned into a SILENTLY
        // incomplete node with a 200 response. Scalars are unchanged
        // (byte-identical output).
        if let Some(props) = &node.properties {
            // Object term for a SCALAR json value; None for array/object/null.
            let scalar_term = |v: &serde_json::Value| -> Option<String> {
                match v {
                    serde_json::Value::String(s) => Some(format!("\"{}\"", escape_turtle(s))),
                    serde_json::Value::Number(n) => n
                        .as_i64()
                        .map(|i| format!("\"{i}\"^^xsd:integer"))
                        .or_else(|| n.as_f64().map(|f| format!("\"{f}\"^^xsd:double"))),
                    serde_json::Value::Bool(b) => Some(format!("\"{b}\"^^xsd:boolean")),
                    _ => None,
                }
            };
            for (key, val) in props {
                let pred = sanitize_iri_local(key);
                match val {
                    // one triple per scalar element — multi-valued predicate preserved
                    serde_json::Value::Array(elems) => {
                        for elem in elems {
                            if let Some(term) = scalar_term(elem) {
                                ttl.push_str(&format!(" ;\n    aegis:{pred} {term}"));
                            }
                        }
                    }
                    _ => {
                        if let Some(term) = scalar_term(val) {
                            ttl.push_str(&format!(" ;\n    aegis:{pred} {term}"));
                        }
                    }
                }
            }
        }

        ttl.push_str(" .\n\n");
    }

    // Edges.
    for edge in &episode.edges {
        let src = sanitize_iri_local(&edge.source);
        let tgt = sanitize_iri_local(&edge.target);
        // Validated in ingest_episode_with_resolution before we get here; the
        // fallback keeps this function infallible and can only be reached by a
        // caller that generates Turtle without that gate.
        let rel = resolve_edge_predicate(&edge.relation)
            .unwrap_or_else(|_| format!("aegis:{}", sanitize_iri_local(&edge.relation)));
        ttl.push_str(&format!("aegis:{src} {rel} aegis:{tgt} .\n"));

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
            ttl.push_str(&format!("    rdf:predicate {rel} ;\n"));
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

/// Prefixes that `episode_to_turtle` declares, and so may appear verbatim in an
/// edge `relation`. Keep in lockstep with the `@prefix` block in
/// `episode_to_turtle` — a prefix resolved here but not declared there emits
/// Turtle that fails to parse.
const KNOWN_PREFIXES: &[(&str, &str)] = &[
    ("rdf", namespace::RDF),
    ("rdfs", namespace::RDFS),
    ("owl", namespace::OWL),
    ("skos", namespace::SKOS),
    ("prov", namespace::PROV),
    ("quipu", namespace::QUIPU),
    ("xsd", namespace::XSD),
    ("sh", namespace::SHACL),
];

/// Resolve an edge `relation` into the Turtle predicate term to emit.
///
/// `/episode` used to force EVERY relation into `aegis:` and then sanitize it,
/// so `rdfs:subClassOf` was stored as `aegis:rdfs_subClassOf` — a predicate that
/// resembles the intended one, matches nothing, and is inert. The response was
/// HTTP 200 with a healthy `count`, so nothing signalled the loss (aegis-kuotp).
/// Measured in the live graph before the fix: `aegis:owl_sameAs` had a real
/// instance that no `owl:sameAs` query could ever reach.
///
/// The policy is: represent the caller's predicate faithfully, or refuse and say
/// which path to use. Never silently rewrite it.
///
/// - `<http://example.org/p>` — a full IRI, emitted verbatim.
/// - `rdfs:subClassOf` — a declared prefix, emitted verbatim.
/// - `foo:bar` — an undeclared prefix, REFUSED (naming `/set`).
/// - `related_to` — no prefix, lands in `aegis:` as before.
/// - `runs on` — would not round-trip through `sanitize_iri_local`, REFUSED.
fn resolve_edge_predicate(relation: &str) -> Result<String> {
    let rel = relation.trim();
    if rel.is_empty() {
        return Err(crate::error::Error::InvalidValue(
            "edge relation is empty — every edge requires a relation.".to_string(),
        ));
    }

    // A full IRI, written in angle brackets. Emitted verbatim.
    if let Some(inner) = rel.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        if inner.contains(['<', '>', '"', ' ']) || !inner.contains(':') {
            return Err(crate::error::Error::InvalidValue(format!(
                "edge relation '{relation}' is not a usable IRI."
            )));
        }
        return Ok(format!("<{inner}>"));
    }

    // A prefixed name. Resolve against the prefixes this writer declares.
    if let Some((prefix, local)) = rel.split_once(':') {
        let Some((name, _)) = KNOWN_PREFIXES.iter().find(|(p, _)| *p == prefix) else {
            let known: Vec<&str> = KNOWN_PREFIXES.iter().map(|(p, _)| *p).collect();
            return Err(crate::error::Error::InvalidValue(format!(
                "edge relation '{relation}' uses undeclared prefix '{prefix}:'. \
                 /episode can emit these prefixes verbatim: {}. For any other \
                 vocabulary, POST the fact to /set, which takes a full predicate \
                 IRI — or write the relation as a full IRI in angle brackets, \
                 e.g. \"<http://example.org/{local}>\" (aegis-kuotp).",
                known.join(", ")
            )));
        };
        if local.is_empty() || sanitize_iri_local(local) != local {
            return Err(crate::error::Error::InvalidValue(format!(
                "edge relation '{relation}' has a local name that is not a valid \
                 IRI local part. Use only letters, digits, '-', '_' and '.' \
                 (aegis-kuotp)."
            )));
        }
        return Ok(format!("{name}:{local}"));
    }

    // A bare name: the aegis: domain vocabulary, as before. It must survive
    // sanitization unchanged, or we would be silently renaming it — the exact
    // defect this function exists to stop, one namespace over.
    if sanitize_iri_local(rel) != rel {
        return Err(crate::error::Error::InvalidValue(format!(
            "edge relation '{relation}' cannot be represented as-is — it would be \
             silently rewritten to '{}'. Use only letters, digits, '-', '_' and \
             '.' (e.g. '{}'), or a prefixed/full IRI for a foreign vocabulary \
             (aegis-kuotp).",
            sanitize_iri_local(rel),
            sanitize_iri_local(rel)
        )));
    }
    Ok(format!("aegis:{rel}"))
}

/// Validate a node `type`, refusing anything that would be silently rewritten.
///
/// `type` is a STRING, and a comma-separated one used to mint a single junk class:
/// `"Feature, Concept"` became `aegis:Feature__Concept` — one class, not two —
/// behind HTTP 200 with a healthy `count`. The node was in the store, correctly
/// described and edged, and **absent from `?s a Feature`**, the query anyone
/// actually runs (aegis-vngta).
///
/// It catches careful people specifically: `/search` renders a multi-typed node as
/// `type: Bead, Issue`, so the documented way to discover an existing node's typing
/// hands back a string that looks like valid input. Searching first — the rule that
/// exists to prevent duplicate nodes — is what fed the mistake.
///
/// REFUSE rather than split, deliberately. Splitting would be a lenient parser
/// guessing intent, and it would fork semantics from the crew-side guard in
/// `graph-extract` (aegis-vngta, muldoon), which already refuses this input and
/// documents `/search` output as display-only. Two layers must not disagree about
/// whether the same request is legal.
fn validate_node_type(node_name: &str, ntype: &str) -> Result<()> {
    let t = ntype.trim();
    if t.contains(',') {
        let split: Vec<&str> = t
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        return Err(crate::error::Error::InvalidValue(format!(
            "node '{node_name}' has a comma-separated type '{ntype}'. `type` is a \
             single class, and this would mint ONE junk class \
             'aegis:{}' that no `?s a <type>` query can reach. For multiple types, \
             send ONE ENTRY PER TYPE repeating the same node name — e.g. {} — which \
             resolves to one entity carrying both types. Note that `/search` renders \
             types as '{}' for DISPLAY only; that format is not valid input \
             (aegis-vngta).",
            sanitize_iri_local(t),
            split
                .iter()
                .map(|s| format!("{{\"name\":\"{node_name}\",\"type\":\"{s}\"}}"))
                .collect::<Vec<_>>()
                .join(", "),
            split.join(", "),
        )));
    }
    if sanitize_iri_local(t) != t {
        return Err(crate::error::Error::InvalidValue(format!(
            "node '{node_name}' has type '{ntype}', which cannot be represented as-is \
             — it would be silently rewritten to '{}'. Use only letters, digits, '-', \
             '_' and '.' (aegis-vngta).",
            sanitize_iri_local(t)
        )));
    }
    Ok(())
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
