//! MCP tool handlers for Quipu -- the agent-facing API surface.
//!
//! Each function takes JSON input and returns JSON output, matching the
//! Model Context Protocol tool calling convention. Bobbin's MCP server
//! delegates knowledge graph operations to these handlers.

pub mod governance;
pub mod graphiti;
pub mod impact;
pub mod named_query;
#[cfg(feature = "owl")]
pub mod owl;
pub mod proposal;
pub mod resolution;
pub mod search;
#[cfg(test)]
mod tests;
pub mod tools;
mod value;

use serde_json::Value as JsonValue;

pub use governance::{
    tool_cooccurrence, tool_graph_create, tool_graph_label, tool_overlay_compose,
    tool_overlay_create, tool_overlay_write, tool_policy_check, tool_verdict_verify,
    tool_verifier_authorized,
};

use crate::error::{Error, Result};
use crate::resolution::EntityCandidate;
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

/// Execute a `/query` request and apply the server-side row ceiling.
///
/// Returns the (possibly truncated) [`QueryResult`] and whether truncation
/// occurred. Shared by the default bespoke-JSON path ([`tool_query`]) and the
/// content-negotiated W3C path ([`crate::w3c`]) so both honor the same
/// `max_sparql_rows` ceiling (hq-gkd) from one place — a LIMIT-less query cannot
/// dump the whole fact log to either serializer.
pub fn query_result(store: &Store, input: &JsonValue) -> Result<(QueryResult, bool)> {
    let (query_str, ctx) = query_context(store, input)?;

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
    let max_rows = store.search_config().max_sparql_rows;

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
    let graph = match input.get("graph").and_then(|v| v.as_str()) {
        None => sparql::GraphScope::default(),
        Some(iri) if store.is_dataset(iri)? => {
            sparql::GraphScope::Default(store.dataset_member_ids(iri)?)
        }
        Some(iri) => sparql::GraphScope::Default(vec![store.lookup(iri)?.unwrap_or(-1)]),
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

/// Which type constants in this request now withhold former subclass expansion.
///
/// Empty when the asserted-only migration did not alter this query. The marker
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
    let value = match labels {
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
        }),
        Err(e) => serde_json::json!({ "error": e.to_string() }),
    };
    if let Some(obj) = out.as_object_mut() {
        obj.insert("labels".to_string(), value);
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
                "applied": false,
                "withheldTypes": types,
                "note": "asserted-only since the SPARQL type-form migration; rdfs:subClassOf \
            expansion was withheld. For the inferred answer use ?s a/rdfs:subClassOf* <Type>",
            }),
        );
    }
}

/// The asserted-only migration marker as an HTTP header value, or `None` when
/// this query was unchanged.
///
/// The W3C-negotiated response shapes (`application/sparql-results+json|xml`,
/// `text/turtle`) are fixed by spec: there is no place in a `{"head":{},
/// "boolean":true}` document or a Turtle graph to put a marker without emitting
/// something that is no longer the format the caller asked for. So on that path
/// the signal goes out of band, in a header, where it annotates the response
/// without touching the body a conformant parser will read.
///
/// Names only the affected type constants, not their subclass sets — a header is
/// bounded. The full `withheldTypes` detail is one Accept-free request away.
pub fn inference_header(withheld: &[rdfs::WithheldType]) -> Option<String> {
    if withheld.is_empty() {
        return None;
    }
    Some(format!(
        "withheld: {}",
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

    let (result, truncated) = query_result(store, input)?;
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

    match result {
        QueryResult::Select { variables, rows } => {
            let json_rows: Vec<JsonValue> = rows
                .iter()
                .map(|row| {
                    let obj: serde_json::Map<String, JsonValue> = row
                        .iter()
                        .map(|(k, v)| (k.clone(), value_to_json(store, v)))
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
                        "subject": t.subject,
                        "predicate": t.predicate,
                        "object": value_to_json(store, &t.object)
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
/// Input: `{ "graph": "<iri>"?, "format": "turtle"|"ntriples"? }`. `graph`
/// omitted -> the ROOT default graph (`g = 0`); an unknown graph IRI is an
/// error. Output: `{ "rdf": "<document>", "format": "turtle",
/// "graph": "<iri>"|null, "triples": N }`.
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
    let (bytes, triples) = crate::export_rdf_subset(store, format, graph)?;
    let rdf = String::from_utf8(bytes)
        .map_err(|e| Error::InvalidValue(format!("export produced non-UTF8 RDF: {e}")))?;
    Ok(serde_json::json!({
        "rdf": rdf,
        "format": format_str,
        "graph": graph,
        "triples": triples,
    }))
}

/// MCP tool: `quipu_knot` -- Assert facts with optional SHACL validation.
///
/// Input: `{ "turtle": "<data>", "timestamp": "...", "actor": "...",
///           "source": "...", "shapes": "<optional shapes turtle>",
///           "replace_snapshot": false, "snapshot": "<stable producer key>" }`
/// Output: `{ "tx_id": N, "count": N }` or validation feedback on failure.
pub fn tool_knot(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let turtle = input
        .get("turtle")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'turtle' parameter".into()))?;

    let now = crate::time::now_iso();
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(&now);

    let actor = input.get("actor").and_then(|v| v.as_str());
    let source = input.get("source").and_then(|v| v.as_str());
    let replace_snapshot = input
        .get("replace_snapshot")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let snapshot = input.get("snapshot").and_then(JsonValue::as_str);
    if replace_snapshot && snapshot.is_none_or(|s| s.trim().is_empty()) {
        return Err(Error::InvalidValue(
            "replace_snapshot requires a non-empty stable 'snapshot' producer key".into(),
        ));
    }

    // SHACL validation: combine per-request shapes with stored shapes.
    let request_shapes = input.get("shapes").and_then(|v| v.as_str());
    let stored_shapes = store.get_combined_shapes()?;

    #[allow(unused_variables)]
    let combined_shapes = match (request_shapes, &stored_shapes) {
        (Some(req), Some(stored)) => Some(format!("{stored}\n\n{req}")),
        (Some(req), None) => Some(req.to_string()),
        (None, Some(stored)) => Some(stored.clone()),
        (None, None) => None,
    };

    // Validated WITH THE STORE AS CONTEXT (aegis-fp17f). `/knot` is the chunked
    // write path, so it is where payload-only `sh:class` did its damage: a
    // caller that splits one graph across several posts had every chunk judged
    // as if the others did not exist. See `shacl_context` for why the store
    // supplies only types, and only for nodes the payload already references.
    #[cfg(feature = "shacl")]
    if let Some(shapes) = &combined_shapes {
        let feedback = crate::shacl_context::validate_with_store_context(store, shapes, turtle)?;
        if !feedback.conforms {
            let issues: Vec<JsonValue> = feedback
                .results
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "severity": r.severity,
                        "focus_node": r.focus_node,
                        "component": r.component,
                        "path": r.path,
                        "value": r.value,
                        "source_shape": r.source_shape,
                        "message": r.message
                    })
                })
                .collect();
            return Ok(serde_json::json!({
                "conforms": false,
                "violations": feedback.violations,
                "warnings": feedback.warnings,
                "issues": issues,
                "hint": "propose a schema change via quipu_propose_schema_change"
            }));
        }
    }

    let (tx_id, count) = if replace_snapshot {
        let source_tag = format!("snapshot:{}", snapshot.unwrap());
        let mut datums = store.plan_source_retraction(&source_tag, 0)?;
        let mut assertions = crate::rdf::parse_rdf(
            store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            timestamp,
        )?;
        let count = assertions.len();
        datums.retain(|old| {
            !assertions.iter().any(|new| {
                old.entity == new.entity && old.attribute == new.attribute && old.value == new.value
            })
        });
        datums.append(&mut assertions);
        let tx_id = store.transact_to_graph(&datums, timestamp, actor, Some(&source_tag), 0)?;
        (tx_id, count)
    } else {
        crate::rdf::ingest_rdf(
            store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            timestamp,
            actor,
            source,
        )?
    };

    Ok(serde_json::json!({
        "conforms": true,
        "tx_id": tx_id,
        "count": count,
        "snapshot": snapshot,
        "replaced": replace_snapshot
    }))
}

/// MCP tool definitions as JSON schemas for registration with Bobbin.
pub fn tool_definitions() -> Vec<JsonValue> {
    #[allow(unused_mut)]
    let mut defs = vec![
        serde_json::json!({
            "name": "quipu_query",
            "description": "Execute a SPARQL SELECT query against the knowledge graph (supports time-travel via valid_at/tx)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "SPARQL SELECT query" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time filtering (ISO-8601). Omit for current state." },
                    "tx": { "type": "integer", "description": "Maximum transaction ID to consider. Omit for all transactions." }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_export",
            "description": "Export a scoped SUBSET of the graph as RDF: one named graph's facts (quipu #36), or the ROOT default graph when 'graph' is omitted. The 'pull a scoped slice' primitive for subset-export and federation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "graph": { "type": "string", "description": "Named-graph IRI to export. Omit for the ROOT/default graph. Unknown IRI is an error." },
                    "format": { "type": "string", "enum": ["turtle", "ntriples"], "description": "RDF serialization (default: turtle)." }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_knot",
            "description": "Assert facts into the knowledge graph (with optional SHACL validation)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "turtle": { "type": "string", "description": "RDF data in Turtle format to assert" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the assertion" },
                    "actor": { "type": "string", "description": "Who is making the assertion" },
                    "source": { "type": "string", "description": "Provenance source (episode, file, etc.)" },
                    "shapes": { "type": "string", "description": "Optional SHACL shapes in Turtle for validation" }
                },
                "required": ["turtle"]
            }
        }),
        serde_json::json!({
            "name": "quipu_cord",
            "description": "List entities in the knowledge graph, optionally filtered by type or predicate",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Filter by rdf:type IRI" },
                    "predicate": { "type": "string", "description": "Filter by predicate IRI" },
                    "limit": { "type": "integer", "description": "Maximum number of entities (default: 100)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_unravel",
            "description": "Time-travel query: see facts as they were at a given point",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tx": { "type": "integer", "description": "Maximum transaction ID to consider" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time filtering (ISO-8601)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_validate",
            "description": "Validate RDF data against SHACL shapes without writing",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "shapes": { "type": "string", "description": "SHACL shapes in Turtle format" },
                    "data": { "type": "string", "description": "RDF data in Turtle format to validate" }
                },
                "required": ["shapes", "data"]
            }
        }),
        serde_json::json!({
            "name": "quipu_shapes",
            "description": "Manage persistent SHACL shapes (load, list, remove). Loaded shapes auto-validate on writes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["load", "list", "remove"], "description": "Action to perform (default: list)" },
                    "name": { "type": "string", "description": "Shape graph name (required for load/remove)" },
                    "turtle": { "type": "string", "description": "SHACL shapes in Turtle format (required for load)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_queries",
            "description": "Manage STORED named queries — competency questions a consumer ships with its domain, callable through quipu_ask alongside the compiled-in catalog. Definitions are validated at LOAD (template must parse; every {placeholder} needs a spec; an optional param needs a default) and versioned: re-loading a name closes the prior version rather than overwriting it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["load", "list", "get", "remove"], "description": "Action to perform (default: list)" },
                    "name": { "type": "string", "description": "Query name (required for load/get/remove)" },
                    "description": { "type": "string", "description": "What the query answers" },
                    "template": { "type": "string", "description": "SPARQL template with {param} placeholders (required for load)" },
                    "dataset": { "type": "string", "description": "Optional dataset IRI this query is scoped to; activates it unless the caller passes `graph`" },
                    "params": { "type": "array", "description": "Ordered param specs: {name, type: iri|text|int, required, default, description}", "items": {} },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_datasets",
            "description": "Manage named datasets — a reusable NAME for an arbitrary set of graphs, so it can be labelled, governed and handed to another agent. `FROM <dataset-iri>` then means FROM over its members. Datasets overlap freely and are never implicitly active.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["create", "list", "show", "remove"], "description": "Action to perform (default: list)" },
                    "name": { "type": "string", "description": "Dataset IRI (required for create/show/remove)" },
                    "members": { "type": "array", "description": "Graph IRIs, or {\"graph\": \"<iri>\", \"ord\": N} objects for a declared ordering. Duplicate ranks are refused.", "items": {} },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" },
                    "actor": { "type": "string", "description": "Who is creating the dataset" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_retract",
            "description": "Retract facts for an entity: all of them, or narrowed by predicate and/or value. Entity + predicate + value retracts exactly ONE (e,a,v) statement — use this to remove a stray edge instead of retracting a whole episode and rebuilding it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "IRI of the entity to retract" },
                    "predicate": { "type": "string", "description": "Optional: only retract facts with this predicate IRI" },
                    "value": { "description": "Optional: only retract facts with this object value. A bare string is a literal; use {\"iri\": \"...\"} for a reference, or {\"int\"|\"float\"|\"bool\": ...}. With entity + predicate this pins a single triple." },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the retraction" },
                    "actor": { "type": "string", "description": "Who is performing the retraction" }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "quipu_set",
            "description": "Atomically SET (entity, predicate) to exactly one value: retracts every current object on that predicate and asserts the new one in a single transaction. The supersede primitive — re-parenting (reports_to A -> B) is one call, with no empty-predicate window and no way to end up with two supervisors by forgetting the retract half. SINGLE-VALUE semantics: replaces ALL current objects; to add without removing, assert via quipu_knot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "IRI of the entity (must exist)" },
                    "predicate": { "type": "string", "description": "Predicate IRI to set (may be new)" },
                    "value": { "description": "New object value. A bare string is a literal; use {\"iri\": \"...\"} for an edge, or {\"int\"|\"float\"|\"bool\": ...} / {\"value\", \"lang\"|\"datatype\"}. A bare string aimed at an IRI-valued predicate is refused loudly." },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the supersede" },
                    "actor": { "type": "string", "description": "Who is performing the set" }
                },
                "required": ["entity", "predicate", "value"]
            }
        }),
        serde_json::json!({
            "name": "quipu_retract_episode",
            "description": "Episode-scoped logical retraction: retract all currently-active facts an episode's ingest contributed (activity node, entities, edges, reified statements), via the bitemporal valid_to close path. Logical, not physical — time-travel history is preserved. Entities and other episodes' facts are untouched. Idempotent. Node IDENTITY is protected: by default (on_orphan=preserve) rdfs:label/rdf:type survive for any node that other episodes still reference, so retraction never leaves an unlabelled, untyped ghost. The response always reports identity_orphans.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "episode": { "type": "string", "description": "Episode name/identifier to retract (aliases: episode_id, name)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the retraction" },
                    "actor": { "type": "string", "description": "Who is performing the retraction" },
                    "on_orphan": { "type": "string", "enum": ["preserve", "refuse", "allow"], "description": "What to do when retraction would strip rdfs:label/rdf:type from a node other episodes still reference. preserve (default): keep its identity alive. refuse: reject the whole retraction. allow: legacy strict scope — creates ghosts, reported in identity_orphans." }
                },
                "required": ["episode"]
            }
        }),
        serde_json::json!({
            "name": "quipu_episode",
            "description": "Ingest structured knowledge from an agent episode (nodes + edges)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Episode name/identifier" },
                    "episode_body": { "type": "string", "description": "Natural language description of the knowledge" },
                    "source": { "type": "string", "description": "Who/what produced this episode" },
                    "group_id": { "type": "string", "description": "Knowledge graph group (e.g. aegis-ontology)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the assertion" },
                    "nodes": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "type": { "type": "string" }, "description": { "type": "string" }, "properties": { "type": "object" } }, "required": ["name"] }, "description": "Entity nodes to create" },
                    "edges": { "type": "array", "items": { "type": "object", "properties": { "source": { "type": "string" }, "target": { "type": "string" }, "relation": { "type": "string" } }, "required": ["source", "target", "relation"] }, "description": "Relationship edges between nodes" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "quipu_search",
            "description": "Semantic vector search over entity embeddings. Accepts a pre-computed embedding vector or a natural-language query (auto-embedded when an EmbeddingProvider is configured).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query (auto-embedded when EmbeddingProvider is attached)" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Pre-computed query embedding vector (f32 array). Takes precedence over query." },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "valid_at": { "type": "string", "description": "Point-in-time for temporal filtering (ISO-8601)" },
                    "group_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional: best-effort filter to entities from these provenance groups (episode-scoped label, NOT an isolation boundary; `/knot` facts are ungrouped and dropped from a group scope)" },
                    "entity_type": { "type": "string", "description": "Optional: restrict to entities of this rdf:type IRI" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_hybrid_search",
            "description": "Combined SPARQL + vector similarity search with predicate pushdown. Accepts a pre-computed embedding or natural-language query. Simple type constraints (e.g. ?s a <Type>) are pushed into the vector index for O(log n) filtered ANN.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query (auto-embedded when EmbeddingProvider is attached)" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Pre-computed query embedding vector (f32 array). Takes precedence over query." },
                    "sparql": { "type": "string", "description": "SPARQL SELECT query returning entity IRIs in the first variable. Simple type patterns (e.g. ?s a <Type>) enable predicate pushdown." },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "valid_at": { "type": "string", "description": "Point-in-time for temporal filtering (ISO-8601)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_search_nodes",
            "description": "Search for entities in the knowledge graph by natural language query. Uses text matching on entity names, labels, and values. Replaces Graphiti's search_nodes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "group_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional: filter to entities from these knowledge graph groups" },
                    "max_results": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "entity_type_filter": { "type": "string", "description": "Optional: filter by rdf:type IRI" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_search_facts",
            "description": "Search for relationships/edges in the knowledge graph by natural language query. Finds facts where the predicate or value matches the query. Replaces Graphiti's search_memory_facts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "group_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional: filter to facts from these knowledge graph groups" },
                    "max_results": { "type": "integer", "description": "Maximum results (default: 10)" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_episodes_complete",
            "description": "Graphiti-compatible flat episode ingestion. Accepts name, body text, group, and source — converts to Quipu episode and ingests.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Episode name/identifier" },
                    "episode_body": { "type": "string", "description": "Natural language body of the episode" },
                    "group_id": { "type": "string", "description": "Knowledge graph group (e.g. aegis-ontology)" },
                    "source_description": { "type": "string", "description": "Who/what produced this episode" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the assertion" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "quipu_impact",
            "description": "Impact analysis: walk downstream from an entity. With remove=true, speculatively retracts the entity first (counterfactual: 'what would break if I removed this?'). The store is never mutated.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entity": { "type": "string", "description": "IRI of the entity to analyse" },
                    "remove": { "type": "boolean", "description": "If true, speculatively retract the entity before walking (counterfactual mode). Default: false." },
                    "hops": { "type": "integer", "description": "Maximum edge hops to follow (default: 5)" },
                    "rank_by_ppr": { "type": "boolean", "description": "Order the reached set by Personalized PageRank seeded at the root (each entry gains a 'ppr' score) instead of BFS discovery order (default: false)" },
                    "predicates": { "type": "array", "items": { "type": "string" }, "description": "Restrict walk to these predicate IRIs. Empty = all edges." },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp for the speculative retraction (used only when remove=true)" }
                },
                "required": ["entity"]
            }
        }),
        serde_json::json!({
            "name": "quipu_unified_search",
            "description": "Unified knowledge search for Bobbin integration. Combines text and optional vector search, returning results tagged with source='knowledge' and normalized 0-1 scores. When an EmbeddingProvider is attached, the query is auto-embedded for semantic search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "embedding": { "type": "array", "items": { "type": "number" }, "description": "Optional pre-computed query embedding. When omitted and EmbeddingProvider is attached, query is auto-embedded." },
                    "limit": { "type": "integer", "description": "Maximum results (default: 10)" },
                    "expand_links": { "type": "boolean", "description": "Expand results via graph links (default: true)" },
                    "max_facts_per_entity": { "type": "integer", "description": "Maximum facts per entity (default: 10)" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_ask",
            "description": "Run a curated, parameterized named query by name instead of hand-writing SPARQL. Call with no 'name' (or name='list') to discover the self-describing catalog of available queries and their parameters (e.g. service_deps, references_to, entity_facts, entities_of_type, labeled_like).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Named query to run; omit (or 'list') to list the catalog." },
                    "params": { "type": "object", "description": "Parameter map for the named query (see catalog for names/types)." }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_propose_schema_change",
            "description": "Submit a schema evolution proposal (new shape, class, property, or ontology change). Proposals require explicit acceptance before taking effect.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["shape", "ontology", "class", "property"], "description": "Kind of schema change" },
                    "target": { "type": "string", "description": "Shape name, class IRI, or property IRI being changed" },
                    "diff": { "type": "string", "description": "Turtle fragment or JSON patch describing the change" },
                    "rationale": { "type": "string", "description": "Why this change is needed" },
                    "proposer": { "type": "string", "description": "Identity of the proposing agent" },
                    "trigger_ref": { "type": "string", "description": "Validation failure ref or bead id that triggered this proposal" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                },
                "required": ["kind", "target", "diff", "proposer"]
            }
        }),
        serde_json::json!({
            "name": "quipu_list_proposals",
            "description": "List schema evolution proposals, optionally filtered by status (pending, accepted, rejected)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["pending", "accepted", "rejected"], "description": "Filter by proposal status (default: all)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_accept_proposal",
            "description": "Accept a pending schema proposal. For shape proposals, validates the Turtle before writing to the shapes table.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Proposal ID to accept" },
                    "decided_by": { "type": "string", "description": "Identity of the approver (default: aegis/crew/braino)" },
                    "note": { "type": "string", "description": "Optional acceptance note" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                },
                "required": ["id"]
            }
        }),
        serde_json::json!({
            "name": "quipu_reject_proposal",
            "description": "Reject a pending schema proposal with a reason",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Proposal ID to reject" },
                    "decided_by": { "type": "string", "description": "Identity of the rejector (default: aegis/crew/braino)" },
                    "note": { "type": "string", "description": "Reason for rejection" },
                    "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
                },
                "required": ["id", "note"]
            }
        }),
        serde_json::json!({
            "name": "quipu_resolve_entity",
            "description": "Check for existing near-duplicate entities before writing. Uses vector similarity (embedding) and canonical name matching (Jaro-Winkler) to find entities that may be duplicates. Returns candidates with similarity scores and match explanations.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Canonical name of the proposed entity" },
                    "properties": { "type": "object", "description": "Optional key-value properties of the entity (used for embedding context)" },
                    "top_k": { "type": "integer", "description": "Maximum number of candidates to return (default: 3)" },
                    "threshold": { "type": "number", "description": "Similarity threshold 0.0-1.0 (default: 0.85)" }
                },
                "required": ["name"]
            }
        }),
        serde_json::json!({
            "name": "quipu_graph",
            "description": "Project the knowledge graph into a render-ready node-link payload in ONE response: nodes (iri, label, rdf:type, degree), edges as [source_index, target_index, predicate] into that node array, and a type census ordered by prevalence. Excludes prov:Activity episodes and rdf/rdfs/prov scaffolding predicates by default so the domain graph is not buried in provenance. Nodes are ranked by degree and capped; the response states what was dropped.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max nodes to return, ranked by degree (default: 250, hard max: 2000)" },
                    "type": { "type": "string", "description": "Restrict to nodes of this rdf:type IRI (edges are scoped to the filtered set too)" },
                    "include_episodes": { "type": "boolean", "description": "Include prov:Activity episode nodes (default: false)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_project",
            "description": "Project the knowledge graph and run a graph algorithm over it: stats (node/edge counts), in_degree (most-referenced entities), pagerank/ppr (global or personalized PageRank from seed entities), components (weakly-connected components), louvain (modularity community detection), or shortest_path. Optionally restrict the projection to a node type or predicate. Read-only by default; louvain with persist:true writes quipu:memberOfCommunity facts (superseding any prior derivation).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "algorithm": { "type": "string", "enum": ["stats", "in_degree", "pagerank", "ppr", "components", "louvain", "shortest_path"], "description": "Algorithm to run (default: stats)" },
                    "type": { "type": "string", "description": "Restrict the projection to nodes of this rdf:type IRI" },
                    "predicate": { "type": "string", "description": "Restrict the projection to edges with this predicate IRI" },
                    "graph": { "type": "string", "description": "Project ONE named graph's own facts instead of ROOT (quipu-tz5) — cheap against a small derived layer even when the episode log is large" },
                    "limit": { "type": "integer", "description": "Max results for in_degree/pagerank (default: 20)" },
                    "seeds": { "type": "array", "items": { "type": "string" }, "description": "Seed entity IRIs (or raw term IDs) for personalized PageRank; non-empty switches pagerank to PPR" },
                    "damping": { "type": "number", "description": "PageRank damping factor (default: 0.85)" },
                    "max_iters": { "type": "integer", "description": "PageRank max iterations (default: 100)" },
                    "tolerance": { "type": "number", "description": "PageRank convergence tolerance (default: 1e-6)" },
                    "from": { "type": "string", "description": "Source entity IRI for shortest_path" },
                    "to": { "type": "string", "description": "Target entity IRI for shortest_path" },
                    "persist": { "type": "boolean", "description": "louvain only: persist quipu:memberOfCommunity facts (emergent clustering, NOT an access boundary), bitemporally superseding any prior derivation (default: false)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_context",
            "description": "Query the knowledge graph for context around a natural-language query: returns relevant entities and their facts, ready to prime an agent. Optionally expand to linked entities.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language query to find relevant knowledge context" },
                    "max_entities": { "type": "integer", "description": "Maximum entities to return (default from pipeline config)" },
                    "expand_links": { "type": "boolean", "description": "Whether to expand to entities linked from the matches" },
                    "ppr_rerank": { "type": "boolean", "description": "Re-order candidates by Personalized PageRank seeded at the direct hits before truncation (default: false)" }
                },
                "required": ["query"]
            }
        }),
        serde_json::json!({
            "name": "quipu_report",
            "description": "Generate a live graph report (graphify's GRAPH_REPORT.md equivalent, but queryable): top hubs / 'god-nodes' (by PageRank with in-degree as a secondary signal), surprising connections (low-prior edges that bridge two otherwise-separate Louvain communities — rarer bridges rank first), and auto-suggested questions seeded by those hubs and bridges. Read-only; derived from current graph structure. Communities here are emergent clustering for surfacing, NOT an access boundary.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "type": { "type": "string", "description": "Restrict the projection to nodes of this rdf:type IRI" },
                    "predicate": { "type": "string", "description": "Restrict the projection to edges with this predicate IRI" },
                    "hubs": { "type": "integer", "description": "Number of top hubs to return (default: 10)" },
                    "surprises": { "type": "integer", "description": "Number of surprising connections to return (default: 10)" },
                    "questions": { "type": "integer", "description": "Number of suggested questions to return (default: 8)" }
                }
            }
        }),
        serde_json::json!({
            "name": "quipu_policy_check",
            "description": "Committed-tier evaluation of a governance Policy over the graph of record: evaluates the policy's aegis:claim (a SPARQL ASK, optionally with a $target placeholder) against the committed graph and returns a Verdict — outcome ∈ {satisfied | unsatisfied | unknown} bound to a reproducible evidence_hash. Deterministic and reproducible: any verifier re-running the same ASK over the same committed evidence gets the same verdict (checked, not trusted). Returns the verdict UNSIGNED (signing + persistence is the Phase-0 verifier registry's job).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "policy": { "type": "string", "description": "Policy IRI whose aegis:claim to evaluate (alternative to inline 'claim')" },
                    "claim": { "type": "string", "description": "Inline SPARQL ASK claim (alternative to 'policy')" },
                    "target": { "type": "string", "description": "Target IRI bound to the $target placeholder" },
                    "predicate_id": { "type": "string", "description": "Predicate identifier recorded in the verdict (inline claims only; default: 'inline')" },
                    "evidence_probe": { "type": "string", "description": "Inline ASK for 'does the evidence exist?' — false yields outcome 'unknown' instead of 'unsatisfied'" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time evaluation (ISO-8601). Omit for current state." }
                },
                "required": ["target"]
            }
        }),
        serde_json::json!({
            "name": "quipu_verdict_verify",
            "description": "Verify a signed Verdict against the Phase-0 root of trust: the signature must be valid under the verifier's REGISTERED public key AND the verifier must be authorized to attest the predicate. 'trusted' is the conjunction — the property a consumer should gate on (checked, not trusted-by-assertion).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "predicate_id": { "type": "string", "description": "Predicate the verdict attests" },
                    "target_ref": { "type": "string", "description": "Target the verdict is about" },
                    "outcome": { "type": "string", "description": "Verdict outcome (satisfied | unsatisfied | unknown)" },
                    "evidence_hash": { "type": "string", "description": "Evidence hash the signature seals" },
                    "tier": { "type": "string", "description": "Evidence tier (default: committed)" },
                    "verifier": { "type": "string", "description": "Verifier IRI whose registered key verifies the signature" },
                    "signature": { "type": "string", "description": "Hex ed25519 signature over the verdict message" }
                },
                "required": ["predicate_id", "target_ref", "outcome", "evidence_hash", "verifier", "signature"]
            }
        }),
        serde_json::json!({
            "name": "quipu_verifier_authorized",
            "description": "Check the Phase-0 verifier registry: may this verifier attest this predicate? The discovery half of the governance gate — lets an agent learn who is authorized before trusting (or requesting) an attestation.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "verifier": { "type": "string", "description": "Verifier IRI" },
                    "predicate": { "type": "string", "description": "Predicate IRI to attest" }
                },
                "required": ["verifier", "predicate"]
            }
        }),
        serde_json::json!({
            "name": "quipu_cooccurrence",
            "description": "Deterministic, auditable work-item co-occurrence: given a work-item (Bead) IRI, returns the other work-items that share at least one touched code entity via the provenance chain Bead <-implements- GitCommit -modifies-> entity. A graph query over typed provenance edges, not a statistical mine; ordered by overlap strength. Bitemporal: pass valid_at for 'which work co-occurred as of <date>'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "work_item": { "type": "string", "description": "Work-item (Bead) IRI" },
                    "valid_at": { "type": "string", "description": "Point-in-time for valid-time filtering (ISO-8601)" },
                    "tx": { "type": "integer", "description": "Maximum transaction ID to consider" }
                },
                "required": ["work_item"]
            }
        }),
        serde_json::json!({
            "name": "quipu_overlay_create",
            "description": "Register an overlay-class named graph bound (bind-once) to a committed parent branch. Overlays are scratch layers over the committed graph: write hypotheses into an overlay, read the composed view, and the committed layer stays untouched.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "overlay": { "type": "string", "description": "Overlay graph IRI to register" },
                    "parent_branch": { "type": "string", "description": "Committed parent-branch IRI (omit or null for ROOT)" }
                },
                "required": ["overlay"]
            }
        }),
        serde_json::json!({
            "name": "quipu_overlay_write",
            "description": "Write one overlay primitive: assert, retract, or tombstone a triple in an overlay graph. Tombstone masks the parent branch's fact in the composed view without touching the committed layer.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "overlay": { "type": "string", "description": "Overlay graph IRI" },
                    "op": { "type": "string", "enum": ["assert", "retract", "tombstone"], "description": "Overlay primitive to apply" },
                    "subject": { "type": "string", "description": "Subject IRI" },
                    "predicate": { "type": "string", "description": "Predicate IRI" },
                    "object": { "description": "Object value (IRI string, literal, or typed JSON value)" },
                    "timestamp": { "type": "string", "description": "ISO-8601 valid-time (default: now)" }
                },
                "required": ["overlay", "op", "subject", "predicate", "object"]
            }
        }),
        serde_json::json!({
            "name": "quipu_overlay_compose",
            "description": "Resolve an overlay's composed view over [overlay > parent-branch-root]: asserted-and-not-tombstoned, nearest wins. Read-only.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "overlay": { "type": "string", "description": "Overlay graph IRI" }
                },
                "required": ["overlay"]
            }
        }),
    ];

    // OWL reasoning is gated behind the (non-default) `owl` feature; only
    // advertise the tool when its handler is actually compiled in, otherwise
    // agents see a tool whose call always fails (hq-8wd).
    #[cfg(feature = "owl")]
    defs.push(serde_json::json!({
        "name": "quipu_load_ontology",
        "description": "Manage OWL ontologies: load (parse + materialize entailments), list, or remove. Loaded ontologies enforce class hierarchy, disjoint-class constraints, and property characteristics (inverse, symmetric, functional).",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["load", "list", "remove"], "description": "Action to perform (default: list)" },
                "name": { "type": "string", "description": "Ontology name (required for load/remove)" },
                "turtle": { "type": "string", "description": "OWL ontology in Turtle format (required for load)" },
                "timestamp": { "type": "string", "description": "ISO-8601 timestamp" }
            }
        }
    }));

    defs
}

// ── Helpers ──────────────────────────────────────────────────────

// The Value <-> JSON converters live in `value` (size ratchet split); the
// public path stays `mcp::value_to_json`.
use value::json_to_value;
pub use value::value_to_json;
