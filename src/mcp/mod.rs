//! MCP tool handlers for Quipu -- the agent-facing API surface.
//!
//! Each function takes JSON input and returns JSON output, matching the
//! Model Context Protocol tool calling convention. Bobbin's MCP server
//! delegates knowledge graph operations to these handlers.

pub mod align;
#[cfg(feature = "owl")]
pub mod explain;
pub mod governance;
pub mod graphiti;
pub mod impact;
pub mod knot;
pub mod named_query;
#[cfg(feature = "owl")]
pub mod owl;
pub mod path;
pub mod proposal;
pub mod resolution;
pub mod search;
#[cfg(test)]
mod tests;
pub mod tools;
mod value;

use serde_json::Value as JsonValue;

pub use align::{tool_align_apply, tool_align_decide, tool_align_propose};
pub use governance::{
    tool_cooccurrence, tool_graph_create, tool_graph_label, tool_overlay_compose,
    tool_overlay_create, tool_overlay_write, tool_policy_check, tool_verdict_verify,
    tool_verifier_authorized,
};
pub use knot::tool_knot;

use crate::error::{Error, Result};
use crate::resolution::{Contention, EntityCandidate};
use crate::sparql::{self, QueryResult, TemporalContext, rdfs};
use crate::store::Store;
use crate::store::labels;

/// Render episode-ingest resolution hints (node name → near-duplicate
/// candidates) as JSON for the `resolution_hints` response field (hq-uye).
/// Empty when resolution is disabled or no matches were found.
pub(crate) fn resolution_hints_json(hints: &[(String, Vec<EntityCandidate>)]) -> Vec<JsonValue> {
    hints
        .iter()
        .map(|(node, candidates)| {
            let cands: Vec<JsonValue> = candidates
                .iter()
                .map(|c| {
                    serde_json::json!({
                        "iri": c.iri,
                        "score": c.score,
                        "matched_on": c.matched_on
                    })
                })
                .collect();
            serde_json::json!({ "node": node, "candidates": cands })
        })
        .collect()
}

/// Render resolution contentions — existing entities that MORE THAN ONE node of
/// the same episode claimed — as JSON for the `resolution_contentions` field.
///
/// Separate from `resolution_hints` because it answers a different question. A
/// hint says "this node may be a duplicate of something already stored". A
/// contention says "these nodes are about to become duplicates OF EACH OTHER",
/// which no per-node hint can express and which the caller, not quipu, has to
/// decide about.
pub(crate) fn resolution_contentions_json(contentions: &[Contention]) -> Vec<JsonValue> {
    contentions
        .iter()
        .map(|c| {
            let claimants: Vec<JsonValue> = c
                .claimants
                .iter()
                .map(|(node, score)| serde_json::json!({ "node": node, "score": score }))
                .collect();
            serde_json::json!({ "iri": c.iri, "claimants": claimants })
        })
        .collect()
}

/// Execute a `/query` request and apply the server-side row ceiling.
///
/// Returns the (possibly truncated) [`QueryResult`] and whether truncation
/// occurred. Shared by the default bespoke-JSON path ([`tool_query`]) and the
/// content-negotiated W3C path ([`crate::w3c`]) so both honor the same
/// `max_sparql_rows` ceiling (hq-gkd) from one place — a LIMIT-less query cannot
/// dump the whole fact log to either serializer.
pub fn query_result(store: &Store, input: &JsonValue) -> Result<(QueryResult, bool)> {
    query_result_with_federation(store, input, None)
}

/// [`query_result`] with the server's configured SERVICE allowlist installed.
pub fn query_result_with_federation(
    store: &Store,
    input: &JsonValue,
    federation: Option<std::sync::Arc<crate::config::FederationConfig>>,
) -> Result<(QueryResult, bool)> {
    let (query_str, mut ctx) = query_context(store, input)?;
    ctx.service_remotes = federation;
    let max_rows = store.search_config().max_sparql_rows;
    // Stop a wholly prefix-safe SELECT after one row beyond the response
    // ceiling. The extra row preserves the `truncated: true` signal. Unsafe
    // plans ignore this hint and retain full evaluation semantics.
    ctx.result_limit = Some(max_rows.saturating_add(1));

    // quipu #70: per-row labels are OPT-IN and only annotate under `GRAPH ?g`.
    // Off by default so no existing response shape changes.
    let result = if input
        .get("row_labels")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        sparql::query_row_labeled(store, query_str, &ctx)?
    } else {
        sparql::query_temporal(store, query_str, &ctx)?
    };
    Ok(match result {
        QueryResult::Select {
            variables,
            mut rows,
        } => {
            let truncated = rows.len() > max_rows;
            if truncated {
                rows.truncate(max_rows);
            }
            (QueryResult::Select { variables, rows }, truncated)
        }
        QueryResult::Graph(mut triples) => {
            let truncated = triples.len() > max_rows;
            if truncated {
                triples.truncate(max_rows);
            }
            (QueryResult::Graph(triples), truncated)
        }
        QueryResult::Ask(value) => (QueryResult::Ask(value), false),
    })
}

/// The query string and the temporal/graph context a `/query` request implies.
///
/// Factored out so the executor and the inference marker
/// ([`query_inference`]) derive their context from ONE place. The marker is
/// gated on graph scope — a type pattern inside a named graph is matched
/// literally and is NOT expanded — so a second, drifting copy of this
/// derivation could annotate a result with inference that never happened, which
/// is a worse failure than the silence it was added to fix.
fn query_context<'a>(store: &Store, input: &'a JsonValue) -> Result<(&'a str, TemporalContext)> {
    let query_str = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'query' parameter".into()))?;

    // Optional `graph` param (quipu #36): scope the query's DEFAULT graph to a
    // single named graph, without writing a `FROM`/`GRAPH` clause. Backward
    // compatible when omitted (ROOT default). An unknown IRI scopes to an empty
    // default graph (no rows) — never a silent fall-through to ROOT. A `FROM`
    // clause in the query text overrides this (see `apply_dataset`).
    // quipu #69: the `graph` param resolves a dataset name too, so `FROM
    // <dataset>` and `graph: "<dataset>"` mean the same thing rather than one
    // of them silently scoping to an empty graph.
    // quipu-gp5: `fork` resolves a fork NAME through the registry — unknown or
    // dropped forks are refused loudly, never a silent fall-through to ROOT.
    // Mutually exclusive with `graph`: one request, one scope authority.
    let fork = input.get("fork").and_then(|v| v.as_str());
    let graph = match (fork, input.get("graph").and_then(|v| v.as_str())) {
        (Some(_), Some(_)) => {
            return Err(Error::InvalidValue(
                "pass either 'fork' or 'graph', not both".into(),
            ));
        }
        (Some(name), None) => sparql::GraphScope::Default(vec![store.fork_graph_for_read(name)?]),
        (None, None) => sparql::GraphScope::default(),
        (None, Some(iri)) if store.is_dataset(iri)? => {
            sparql::GraphScope::Default(store.dataset_member_ids(iri)?)
        }
        (None, Some(iri)) => sparql::GraphScope::Default(vec![store.lookup(iri)?.unwrap_or(-1)]),
    };

    // Optional `include_kinds` (graph kinds + deep freeze): widen the DEFAULT
    // graph set with every graph declaring one of these dataKind tokens — the
    // explicit "also compose the cold graphs" switch. Silence never widens:
    // absent or empty, the scope is exactly what the rules above produced. A
    // `FROM` clause in the query text still overrides the whole request-level
    // scope (see `apply_dataset`), and `fork` is one-scope-authority, so the
    // combination is refused rather than half-honoured.
    let graph = match input.get("include_kinds").and_then(|v| v.as_array()) {
        None => graph,
        Some(arr) if arr.is_empty() => graph,
        Some(arr) => {
            if fork.is_some() {
                return Err(Error::InvalidValue(
                    "pass either 'fork' or 'include_kinds', not both — a fork is \
                     a pinned snapshot, and widening it with live graphs would \
                     answer from two different worlds"
                        .into(),
                ));
            }
            let mut kinds = Vec::with_capacity(arr.len());
            for v in arr {
                let s = v.as_str().ok_or_else(|| {
                    Error::InvalidValue("every include_kinds entry must be a string".into())
                })?;
                // Strict parse: an unrecognised SHAPE is an error, never a
                // silently-matching-nothing filter.
                kinds.push(
                    crate::lattice_kind::DataKind::parse(s)?
                        .as_str()
                        .to_string(),
                );
            }
            let extra = store.graphs_of_kinds(&kinds)?;
            match graph {
                sparql::GraphScope::Default(mut ids) => {
                    for g in extra {
                        if !ids.contains(&g) {
                            ids.push(g);
                        }
                    }
                    sparql::GraphScope::Default(ids)
                }
                other => other,
            }
        }
    };

    Ok((
        query_str,
        TemporalContext {
            valid_at: input
                .get("valid_at")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            as_of_tx: input.get("tx").and_then(serde_json::Value::as_i64),
            graph,
            ..Default::default()
        },
    ))
}

/// Which type constants in this request expand over subclasses.
///
/// Empty when subclass entailment does not alter this query. The marker
/// is omitted in that common case; see [`crate::sparql::rdfs::withheld_types`].
pub fn query_inference(store: &Store, input: &JsonValue) -> Result<Vec<rdfs::WithheldType>> {
    let (query_str, ctx) = query_context(store, input)?;
    Ok(rdfs::withheld_types(store, query_str, &ctx))
}

/// Attach the inference marker to a response, when there is one to attach.
///
/// `applied` is always true when the key is present — the field exists to say
/// "this answer is INFERRED, not asserted", and a permanent `applied: false` on
/// every ordinary response is noise that trains readers to skip the field.
/// `expandedTypes` names what was folded in, because "inference happened" is not
/// actionable on its own: the reader needs to know that `Service` swallowed
/// `SearchService` to spot that the answer is not the one they wanted.
///
/// Attached to EVERY result shape `/query` can return — SELECT rows, an ASK
/// boolean, CONSTRUCT/DESCRIBE triples. Instrumenting only the shape whose
/// defect was reported would leave the identical silence one query keyword away,
/// which is the failure this marker exists to end.
///
/// # What the marker claims, and what it does not
///
/// It says expansion WAS APPLIED to the query — a fact about the query. It does
/// NOT say the extra classes changed the answer: a marked ASK can be `true` on a
/// directly-asserted fact that needed no inference at all. That limit is the
/// same on SELECT, but it is worth stating out loud here because a bare boolean
/// invites less scrutiny than a count does — a reader must not read a marked
/// `true` as "this true is inferred". Establishing contribution means running
/// the query a second time without expansion; the marker deliberately does not.
/// Attach the dataset's composed label as a top-level `"labels"` key (quipu
/// #67, graph-labels.md §4.1).
///
/// Always present, `null` when nothing was declared — deliberately unlike the
/// inference marker above, whose PRESENCE is its signal. Here `null` is a
/// meaningful *undeclared*, and a reader must be able to tell it apart from a
/// server that does not do labels at all.
///
/// **A refusal is reported, never propagated.** If the fold refuses (member
/// graphs carrying trust from different chains — #66), the label becomes
/// `{"error": …}` and the query still returns its rows. Failing the whole query
/// would be a regression for every caller that never asked about labels, and
/// this key is attached unconditionally. Refusing to state a label is the
/// honest outcome; refusing to answer the query is a different and unasked-for
/// one.
fn add_labels(out: &mut JsonValue, labels: &Result<Option<labels::DatasetLabels>>) {
    if let Some(obj) = out.as_object_mut() {
        obj.insert("labels".to_string(), labels_json(labels));
    }
}

/// The JSON shape of a composed dataset label — shared by `add_labels` and the
/// federated `/query` path (quipu-fd1), so the two surfaces cannot drift.
pub fn labels_json(labels: &Result<Option<labels::DatasetLabels>>) -> JsonValue {
    match labels {
        Ok(None) => JsonValue::Null,
        Ok(Some(l)) => serde_json::json!({
            "freshness": {
                "value": l.freshness.value.map(crate::lattice::Freshness::as_str),
                "coverage": l.freshness.coverage.as_str(),
            },
            "durability": {
                "value": l.durability.value.map(crate::lattice::Durability::as_str),
                "coverage": l.durability.coverage.as_str(),
            },
            "trust": {
                "value": l.trust.value.as_ref().map(|t| serde_json::json!({
                    "iri": t.iri, "chain": t.chain, "rank": t.rank,
                })),
                "coverage": l.trust.coverage.as_str(),
            },
            "policy": {
                "value": l.policy.value.as_ref().map(|p| p.tokens()),
                "coverage": l.policy.coverage.as_str(),
            },
            "kind": {
                "value": l.kind.value.as_ref().map(crate::lattice_kind::KindSet::kinds),
                "coverage": l.kind.coverage.as_str(),
            },
        }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    }
}

fn add_inference(out: &mut JsonValue, withheld: &[rdfs::WithheldType]) {
    if withheld.is_empty() {
        return;
    }
    let types: Vec<JsonValue> = withheld
        .iter()
        .map(|e| {
            serde_json::json!({
                "type": e.type_iri,
                "subclasses": e.subclasses,
            })
        })
        .collect();
    if let Some(obj) = out.as_object_mut() {
        obj.insert(
            "inference".to_string(),
            serde_json::json!({
                "applied": true,
                "expandedTypes": types,
                "note": "RDFS subclass expansion was applied to the constant rdf:type pattern; use a variable type plus FILTER for an asserted-only census",
            }),
        );
    }
}

/// The subclass-entailment marker as an HTTP header value, or `None` when this
/// query was not expanded.
///
/// The W3C-negotiated response shapes (`application/sparql-results+json|xml`,
/// `text/turtle`) are fixed by spec: there is no place in a `{"head":{},
/// "boolean":true}` document or a Turtle graph to put a marker without emitting
/// something that is no longer the format the caller asked for. So on that path
/// the signal goes out of band, in a header, where it annotates the response
/// without touching the body a conformant parser will read.
///
/// Names only the affected type constants, not their subclass sets — a header is
/// bounded. The full `expandedTypes` detail is one Accept-free request away.
pub fn inference_header(withheld: &[rdfs::WithheldType]) -> Option<String> {
    if withheld.is_empty() {
        return None;
    }
    Some(format!(
        "applied: {}",
        withheld
            .iter()
            .map(|e| e.type_iri.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// MCP tool: `quipu_query` -- Execute a SPARQL query.
///
/// Input: `{ "query": "SELECT/ASK/CONSTRUCT/DESCRIBE ...", "valid_at": "...", "tx": N }`
/// Output depends on query form.
pub fn tool_query(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    tool_query_with_federation(store, input, None)
}

/// [`tool_query`] with configured remotes available to SPARQL `SERVICE`.
pub fn tool_query_with_federation(
    store: &Store,
    input: &JsonValue,
    federation: Option<std::sync::Arc<crate::config::FederationConfig>>,
) -> Result<JsonValue> {
    // quipu #68: label floors are enforced HERE, at the service boundary, and
    // deliberately not inside `query_temporal`.
    //
    // A floor is a consumer-facing quality gate, not an access-control
    // mechanism (graph-labels.md §11) — so it guards the surface external
    // callers reach, while the reasoner, SHACL validation and the episode write
    // path keep using the raw evaluator. Refusing an internal maintenance query
    // because a graph is stale would break the very machinery that makes it
    // fresh again.
    //
    // A no-op unless a floor is configured, so the default path is unchanged.
    if !store.labels_config().is_unset() {
        let (q, ctx) = query_context(store, input)?;
        let member_ids = sparql::dataset_member_ids(store, q, &ctx)?;
        store.check_label_floor(&member_ids)?;
    }

    let (result, truncated) = query_result_with_federation(store, input, federation)?;
    // Announce subclass inference when it widened the answer. Omitted entirely
    // when it did not, so the field's PRESENCE is the signal — a marker that
    // appears on every response is one readers stop seeing.
    let inferred = query_inference(store, input).unwrap_or_default();
    // Computed from the SAME `query_context` the executor used, so the label
    // describes the dataset the query actually read. Held as a Result: a
    // cross-chain refusal is reported in the field, never raised as a query
    // failure (see `add_labels`).
    let labeled =
        query_context(store, input).and_then(|(q, ctx)| sparql::dataset_labels_for(store, q, &ctx));
    let verbose = input
        .get("verbose")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let prefixes = (!verbose)
        .then(|| crate::compact::PrefixMap::from_store(store))
        .transpose()?;
    let render_value = |value: &crate::types::Value| {
        prefixes.as_ref().map_or_else(
            || value_to_json(store, value),
            |map| value_to_json_with_prefixes(store, value, map),
        )
    };
    let compact_iri = |iri: &str| {
        prefixes
            .as_ref()
            .map_or_else(|| iri.to_string(), |map| map.compact(iri))
    };

    match result {
        QueryResult::Select { variables, rows } => {
            let json_rows: Vec<JsonValue> = rows
                .iter()
                .map(|row| {
                    let obj: serde_json::Map<String, JsonValue> = row
                        .iter()
                        .map(|(k, v)| (k.clone(), render_value(v)))
                        .collect();
                    JsonValue::Object(obj)
                })
                .collect();

            let mut out = serde_json::json!({
                "variables": variables,
                "rows": json_rows,
                "count": json_rows.len(),
                "truncated": truncated
            });
            add_inference(&mut out, &inferred);
            add_labels(&mut out, &labeled);
            Ok(out)
        }
        QueryResult::Ask(result) => {
            // The highest-risk shape to leave silent, and the last one that was.
            // `ASK { <x> a <Service> }` is the natural way to ask "is x a
            // Service?", and it answered an inference-widened question with a
            // bare `true` — identical, byte for byte, to the `true` of a fact
            // asserted outright. There is no number here to look at twice, so
            // the marker is the only thing that can distinguish the two worlds.
            let mut out = serde_json::json!({ "result": result });
            add_inference(&mut out, &inferred);
            add_labels(&mut out, &labeled);
            Ok(out)
        }
        QueryResult::Graph(triples) => {
            let json_triples: Vec<JsonValue> = triples
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "subject": compact_iri(&t.subject),
                        "predicate": compact_iri(&t.predicate),
                        "object": render_value(&t.object)
                    })
                })
                .collect();
            // CONSTRUCT/DESCRIBE: expansion adds SUBJECTS to the constructed
            // graph, so the emitted triples are inference-widened exactly as a
            // SELECT's rows are — and a materialised graph is likelier than a
            // count to be written somewhere and re-read later as fact.
            let mut out = serde_json::json!({
                "triples": json_triples,
                "count": json_triples.len(),
                "truncated": truncated
            });
            add_inference(&mut out, &inferred);
            add_labels(&mut out, &labeled);
            Ok(out)
        }
    }
}

/// MCP tool: `quipu_export` -- pull a scoped SUBSET of the graph as RDF (quipu
/// #36). This is the "export a named-graph slice" primitive that federation
/// builds on. It exports one graph's OWN facts — the same scope a
/// `GRAPH <iri> { … }` read sees — not a composed overlay view.
///
/// Input selects at most one scope: `graph`, `group_id`, or `construct`.
pub fn tool_export(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let format_str = input
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("turtle");
    let format = match format_str {
        "turtle" | "ttl" => oxrdfio::RdfFormat::Turtle,
        "ntriples" | "nt" => oxrdfio::RdfFormat::NTriples,
        other => {
            return Err(Error::InvalidValue(format!(
                "unknown export format: {other} (try: turtle, ntriples)"
            )));
        }
    };
    let graph = input.get("graph").and_then(|v| v.as_str());
    let group = input.get("group_id").and_then(|v| v.as_str());
    let construct = input.get("construct").and_then(|v| v.as_str());
    if [graph.is_some(), group.is_some(), construct.is_some()]
        .into_iter()
        .filter(|selected| *selected)
        .count()
        > 1
    {
        return Err(Error::InvalidValue(
            "export accepts only one of graph, group_id, or construct".into(),
        ));
    }
    let (bytes, triples) = match (group, construct) {
        (Some(group_id), None) => crate::export_rdf_group(store, format, group_id)?,
        (None, Some(query)) => crate::export_rdf_construct(store, format, query)?,
        (None, None) => crate::export_rdf_subset(store, format, graph)?,
        _ => unreachable!("mutually exclusive export scopes checked above"),
    };
    let rdf = String::from_utf8(bytes)
        .map_err(|e| Error::InvalidValue(format!("export produced non-UTF8 RDF: {e}")))?;
    Ok(serde_json::json!({
        "rdf": rdf,
        "format": format_str,
        "graph": graph,
        "group_id": group,
        "construct": construct,
        "triples": triples,
    }))
}

pub use definitions::tool_definitions;
mod definitions;

// ── Helpers ──────────────────────────────────────────────────────

// The Value <-> JSON converters live in `value` (size ratchet split); the
// public path stays `mcp::value_to_json`.
use value::json_to_value;
pub use value::{value_to_json, value_to_json_compact, value_to_json_with_prefixes};
