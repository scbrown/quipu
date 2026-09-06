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

    // `entailment` (aegis-1gp76j): answer under a named entailment regime by
    // composing each graph in scope with its companion inferred graph.
    //
    // The regime is a claim about what the default graph ENTAILS, so the
    // closure belongs IN the default graph for the duration of the answer —
    // `GraphScope::Default` is already an RDF merge of a set, so this needs no
    // evaluator change.
    //
    // This surface READS a closure; it never materialises one. `tool_query`
    // takes `&Store` because it runs on the WAL read pool, and taking `&mut`
    // here to materialise on demand would put every SPARQL read behind the
    // single writer lock. So an absent companion graph is REFUSED rather than
    // quietly answered under simple entailment: returning asserted-only rows to
    // a caller who asked for RDFS is the silent-wrong direction, and it is
    // indistinguishable from a correct empty answer. Materialise with
    // `quipu query --entailment rdfs` (or the reasoner) first.
    let graph = match entailment_regime(input)? {
        None => graph,
        Some(regime) => {
            let sparql::GraphScope::Default(ids) = &graph else {
                return Err(Error::InvalidValue(format!(
                    "entailment {regime:?} applies to the default graph; this request \
                     scopes to named graphs"
                )));
            };
            let mut composed: Vec<i64> = Vec::with_capacity(ids.len() * 2);
            for &g in ids {
                if !composed.contains(&g) {
                    composed.push(g);
                }
                let iri = store.companion_inferred_iri(g)?;
                let Some(companion) = store.lookup(&iri)? else {
                    return Err(Error::InvalidValue(format!(
                        "entailment {regime:?} requested but graph {g} has no materialised \
                         closure at <{iri}>; run `quipu query --entailment rdfs` against it \
                         first. Answering without the closure would silently return \
                         asserted-only rows."
                    )));
                };
                if !composed.contains(&companion) {
                    composed.push(companion);
                }
            }
            sparql::GraphScope::Default(composed)
        }
    };
    // Whether an entailment regime was requested, for the evaluator's expansion
    // gate (aegis-g6bu6d). Re-read rather than threaded out of the match above
    // so the two cannot drift: the flag means exactly "the scope was composed".
    let entails_rdfs = entailment_regime(input)?.is_some();

    Ok((
        query_str,
        TemporalContext {
            valid_at: input
                .get("valid_at")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            as_of_tx: input.get("tx").and_then(serde_json::Value::as_i64),
            graph,
            entails_rdfs,
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

/// The requested entailment regime, validated (aegis-1gp76j).
///
/// `None` when the caller did not ask — silence never widens the dataset, so
/// the simple-entailment path is unchanged and pays nothing. An unrecognised
/// regime is an error, never a silently-ignored parameter: a caller who asks
/// for a regime this build does not implement must not be handed an answer that
/// looks like it honoured them.
fn entailment_regime(input: &JsonValue) -> Result<Option<&str>> {
    let Some(v) = input.get("entailment") else {
        return Ok(None);
    };
    let s = v.as_str().ok_or_else(|| {
        Error::InvalidValue("'entailment' must be a string, e.g. \"rdfs\"".into())
    })?;
    if s.eq_ignore_ascii_case("rdfs") {
        Ok(Some("rdfs"))
    } else {
        Err(Error::InvalidValue(format!(
            "unknown entailment regime {s:?}; this build implements \"rdfs\""
        )))
    }
}

/// Announce the entailment regime the answer was computed under.
///
/// A SEPARATE key from `inference`, not an extra field on it: `inference`
/// reports that a constant `rdf:type` pattern was expanded over subclasses,
/// which is a different mechanism with a different remedy. Folding both into
/// one `applied: true` would leave a reader unable to tell which one widened
/// their answer.
///
/// Present only when a regime was requested, so — as with `inference` — the
/// field's PRESENCE is the signal.
/// One composed graph's closure lag: how far its companion trails its premises.
struct GraphLag {
    graph: String,
    base_newest: Option<String>,
    companion_newest: Option<String>,
    /// Seconds the closure trails the base. `None` is UNKNOWN, never zero.
    lag_seconds: Option<i64>,
}

/// Per-graph closure freshness for the entailment marker (aegis-ab4m51).
///
/// Per-graph rather than two store-wide maxima, and that is not a refinement —
/// the scalar version is WRONG. Difference-of-maxima is not maximum-of-
/// differences: with graph A at base 10:00 / companion 10:00 and graph B at
/// base 09:00 / companion 08:00, both maxima are 10:00, so the marker reports
/// no lag while B is an hour stale. A fresh companion masks a stale one,
/// because a maximum hides its minimum — and the marker would then say
/// "nothing to worry about" on exactly the composition it exists to warn
/// about. Separating FRESH from STALE wrongly is worse than the prose note it
/// replaces (malcolm, ruling on aegis-ab4m51).
///
/// `composed` is the flat post-composition scope, so the base/companion
/// pairing has to be recovered. A graph is a BASE graph iff its companion IRI
/// resolves AND that companion is itself in scope; a companion's own companion
/// does not exist, so companions drop out without needing to be recognised.
fn entailment_freshness(store: &Store, composed: &[i64]) -> Vec<GraphLag> {
    let mut lags = Vec::new();
    for &g in composed {
        let Ok(iri) = store.companion_inferred_iri(g) else {
            continue;
        };
        let Ok(Some(companion)) = store.lookup(&iri) else {
            continue;
        };
        if !composed.contains(&companion) {
            continue;
        }
        let base = store.graph_newest_valid_from(g).ok().flatten();
        let comp = store.graph_newest_valid_from(companion).ok().flatten();
        // A base graph holding no facts cannot be stale; anything else whose
        // seconds we cannot read is UNKNOWN, which must not render as fresh.
        let lag_seconds = match (&base, &comp) {
            (None, _) => Some(0),
            (Some((_, Some(b))), Some((_, Some(c)))) => Some((b - c).max(0)),
            _ => None,
        };
        lags.push(GraphLag {
            graph: store.resolve(g).unwrap_or_else(|_| format!("graph:{g}")),
            base_newest: base.map(|(t, _)| t),
            companion_newest: comp.map(|(t, _)| t),
            lag_seconds,
        });
    }
    lags
}

/// Announce the entailment regime the answer was computed under.
///
/// A SEPARATE key from `inference`, not an extra field on it: `inference`
/// reports that a constant `rdf:type` pattern was expanded over subclasses,
/// which is a different mechanism with a different remedy. Folding both into
/// one `applied: true` would leave a reader unable to tell which one widened
/// their answer.
///
/// Present only when a regime was requested, so — as with `inference` — the
/// field's PRESENCE is the signal.
///
/// `worstLagSeconds` (aegis-ab4m51) is the marker's verdict: the largest lag
/// over the composed graphs, so it cannot read fresh while any graph is stale.
/// `null` means UNKNOWN — at least one graph's lag could not be computed — and
/// is deliberately distinct from `0`. This is NOT a refusal, per wu: refusing a
/// stale closure would fail `--entailment` on any graph written since
/// materialisation, which is most of them, and the response would be to stop
/// using the flag. A guard that gets disabled protects nothing. Report the
/// fact; let the caller judge.
///
/// The per-graph `graphs` list is emitted only when there is something to
/// explain — a caller who sees a lag needs the culprit, and `composedGraphs` is
/// an integer that cannot name it. In the healthy case the list is silence.
fn add_entailment(out: &mut JsonValue, regime: Option<&str>, graphs: usize, lags: &[GraphLag]) {
    let Some(regime) = regime else {
        return;
    };
    let worst = if lags.iter().any(|l| l.lag_seconds.is_none()) {
        None
    } else {
        lags.iter().filter_map(|l| l.lag_seconds).max().or(Some(0))
    };
    if let Some(obj) = out.as_object_mut() {
        let mut marker = serde_json::json!({
            "regime": regime,
            "composedGraphs": graphs,
            "worstLagSeconds": worst,
            "note": "answered over the RDF merge of the requested graph(s) and their companion inferred graph(s); the closure is materialised out of band. worstLagSeconds is how far the stalest companion trails its premises: 0 is up to date, null means at least one graph could not be measured and must not be read as fresh",
        });
        if worst != Some(0)
            && let Some(m) = marker.as_object_mut()
        {
            m.insert(
                "graphs".to_string(),
                lags.iter()
                    .map(|l| {
                        serde_json::json!({
                            "graph": l.graph,
                            "baseNewest": l.base_newest,
                            "companionNewest": l.companion_newest,
                            "lagSeconds": l.lag_seconds,
                        })
                    })
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        obj.insert("entailment".to_string(), marker);
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
    // Same input the dataset composition read, so the marker cannot claim a
    // regime the answer was not computed under.
    let regime = entailment_regime(input).unwrap_or(None);
    // The ids, not just their count: the freshness marker has to pair each base
    // graph with its companion, and a bare length cannot name a stale one.
    let composed_ids: Vec<i64> = match query_context(store, input) {
        Ok((_, ctx)) => match &ctx.graph {
            sparql::GraphScope::Default(ids) => ids.clone(),
            _ => Vec::new(),
        },
        Err(_) => Vec::new(),
    };
    let composed_graphs = composed_ids.len();
    // Computed once, not per result arm: all three arms report the same answer
    // over the same dataset, so a per-arm read could only introduce a skew.
    let entail_lags = if regime.is_some() {
        entailment_freshness(store, &composed_ids)
    } else {
        Vec::new()
    };
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
            add_entailment(&mut out, regime, composed_graphs, &entail_lags);
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
            add_entailment(&mut out, regime, composed_graphs, &entail_lags);
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
            add_entailment(&mut out, regime, composed_graphs, &entail_lags);
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
