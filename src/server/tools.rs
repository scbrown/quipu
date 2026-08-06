//! MCP-tool HTTP handlers.
//!
//! The three macros here define the lock/threading discipline once, so every
//! tool endpoint inherits it: `ro_handler` for reads, `rw_handler` for writes
//! (with the deferred ONNX embed), and `embed_handler` for vector search that
//! embeds the query text outside the store lock.

use axum::extract::State;
use serde_json::{Value as JsonValue, json};

use super::SharedStore;
use super::base::{AppError, blocking};

macro_rules! ro_handler {
    ($name:ident, $tool:path) => {
        pub(crate) async fn $name(
            State(s): State<SharedStore>,
            axum::Json(i): axum::Json<JsonValue>,
        ) -> Result<axum::Json<JsonValue>, AppError> {
            // POOLED READ.
            //
            // ⚠ `&Store` does NOT prove read-only, and assuming it does is the
            // mistake this comment exists to stop. `Store::intern` takes
            // `&self` and runs an INSERT; so do `load_shapes`, `remove_shapes`,
            // `load_ontology` and `remove_ontology`. The borrow checker cannot
            // tell a pooled-safe tool from a writing one, and this codebase has
            // already caught two handlers mis-registered as `ro_handler!` for
            // exactly that reason (see the notes on `set_predicate` and the
            // named-graphs registry below).
            //
            // So the guarantee is EMPIRICAL, not type-level, and it is pinned by
            // `every_pooled_tool_survives_a_read_only_connection` in
            // `server/tests.rs`. A tool that writes fails LOUDLY here — the
            // connection is `SQLITE_OPEN_READ_ONLY`, so it returns "attempt to
            // write a readonly database" rather than racing the writer. That is
            // a 500 on a read endpoint, which is why the test exists.
            blocking(move || Ok(axum::Json($tool(&s.read(), &i)?))).await
        }
    };
}

/// Finish deferred auto-embed work drained from a write handler: run the
/// multi-second ONNX `embed_batch` OUTSIDE the store lock, then
/// relock briefly to write the vectors. Entities whose text changed in the
/// unlocked window are skipped by `apply_deferred_embed` — the later writer's
/// own embed pass owns them — so interleaving readers/writers between the two
/// lock windows (the whole point of deferring) cannot regress an embedding.
pub(crate) fn finish_deferred_embed(
    s: &SharedStore,
    work: &quipu::DeferredEmbed,
) -> Result<(), AppError> {
    if work.is_empty() {
        return Ok(());
    }
    // Brief lock: clone the Arc provider and read the batch bound, then DROP
    // the guard.
    let (provider, batch_size) = {
        let st = s.lock();
        (
            st.embedding_provider(),
            st.embedding_config().embed_batch_size,
        )
    };
    let Some(provider) = provider else {
        return Ok(());
    };
    // CHUNK the ONNX call. One embed_batch(N) builds a single
    // [N, seq_len, dim] tensor and, far more expensively, N sequences' worth of
    // ONNX Runtime internal attention activations — and ORT's arena allocator
    // NEVER returns that memory to the OS. So an unbounded N set a permanent
    // RSS high-water mark proportional to the entities one drain touched.
    //
    // MEASURED (all-MiniLM-L6-v2, seq 256, intra_threads 1, the deployed
    // config): ~7.4 MB of retained arena PER TEXT, linear in N and never
    // released — 2048 texts in one call cost +15.1 GB that five subsequent
    // single-text calls did not shrink. The same 2048 texts in chunks of 32
    // cost +492 MB and plateaued flat from the 512th text on. That is the whole
    // bug: the high-water is set by the LARGEST SINGLE BATCH, not by total
    // volume, so bounding the batch bounds resident memory in store size.
    //
    // This path is the reason the bound was missing in practice: every other
    // embed path already chunks (`auto_embed_entities` by embed_batch_size,
    // `backfill_embeddings` by 32). Only the deferred path — the one the SERVER
    // uses for every write — passed the whole work set straight through, and it
    // MERGES across transactions before draining, so the batch grew without
    // limit. In production that ballooned quipu-server from 350 MB to 5.83 GB
    // on a single drain and held it there for hours.
    let batch_size = if batch_size == 0 { 32 } else { batch_size };
    let texts = work.texts();
    let mut embeddings = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(batch_size) {
        // CPU work, LOCK-FREE.
        embeddings.extend(provider.embed_batch(chunk)?);
    }
    // apply_deferred_embed still receives every vector in item order, so its
    // per-entity staleness check is unchanged by the chunking.
    s.lock().apply_deferred_embed(work, &embeddings)?;
    Ok(())
}

macro_rules! rw_handler {
    ($name:ident, $tool:path) => {
        pub(crate) async fn $name(
            State(s): State<SharedStore>,
            axum::Json(i): axum::Json<JsonValue>,
        ) -> Result<axum::Json<JsonValue>, AppError> {
            blocking(move || {
                // Phase 1 under the lock: the transaction itself (plus cheap
                // embed-work collection). Phase 2 (ONNX embed) runs after the
                // guard drops; phase 3 relocks to write vectors. See
                // finish_deferred_embed.
                let (out, work) = {
                    let mut st = s.lock();
                    let out = $tool(&mut st, &i)?;
                    (out, st.take_deferred_embed())
                };
                if let Some(work) = work {
                    finish_deferred_embed(&s, &work)?;
                }
                Ok(axum::Json(out))
            })
            .await
        }
    };
}

/// A vector-search handler that embeds the query text OUTSIDE the store lock.
///
/// The ONNX query-embed is CPU-bound (tens of ms) and was run *inside* the global
/// `Store` mutex — every concurrent /search serialized across it, so a burst drained
/// in sum-of-embeds wall time, and (before store work moved to the blocking pool)
/// it parked the async workers and stalled even lock-free /health. `blocking()`
/// already keeps the work off the reactor; this additionally shrinks the lock
/// hold-time to the vector/SPARQL step so concurrent embeds run in PARALLEL.
///
/// Mechanism: take a brief lock only to clone the cheap `Arc<dyn EmbeddingProvider>`,
/// release it, run `embed_text` lock-free, then inject the vector as the
/// pre-computed `embedding` the tool already accepts — so the tool skips its own
/// in-lock embed and the second lock covers only the search. A caller-supplied
/// `embedding`, or a missing provider, is passed straight through unchanged (the
/// tool then embeds or errors exactly as before), so behaviour is identical.
macro_rules! embed_handler {
    ($name:ident, $tool:path) => {
        pub(crate) async fn $name(
            State(s): State<SharedStore>,
            axum::Json(i): axum::Json<JsonValue>,
        ) -> Result<axum::Json<JsonValue>, AppError> {
            blocking(move || {
                let mut i = i;
                if i.get("embedding").is_none() {
                    if let Some(text) = i.get("query").and_then(|v| v.as_str()).map(str::to_owned) {
                        // Brief lock: clone the Arc provider, then DROP the guard.
                        let provider = { s.lock().embedding_provider() };
                        if let Some(provider) = provider {
                            let vec = provider.embed_text(&text)?; // CPU work, LOCK-FREE
                            if let Some(obj) = i.as_object_mut() {
                                obj.insert("embedding".to_string(), serde_json::json!(vec));
                            }
                        }
                    }
                }
                Ok(axum::Json($tool(&s.lock(), &i)?))
            })
            .await
        }
    };
}

ro_handler!(cord, quipu::tool_cord);
ro_handler!(graph_view, quipu::tool_graph_view);
ro_handler!(unravel, quipu::tool_unravel);
embed_handler!(search, quipu::tool_search);
embed_handler!(hybrid_search, quipu::tool_hybrid_search);
ro_handler!(unified_search, quipu::tool_unified_search);
ro_handler!(ask, quipu::tool_ask);
ro_handler!(search_nodes, quipu::tool_search_nodes);
ro_handler!(search_facts, quipu::tool_search_facts);
// /resolve: genuine read — resolve_entity scans labels and runs a
// vector search; its only compute is embedding the QUERY text (one short name,
// tens of ms — not the episode-embed class), and it commits nothing. The
// read-only claim is asserted by test, not just this comment.
ro_handler!(resolve_probe, quipu::tool_resolve_entity);
ro_handler!(
    graphiti_search_nodes,
    quipu::mcp::graphiti::tool_search_nodes
);
// aegis-e163: /shapes WRITES (tool_shapes -> store.load_shapes / remove_shapes),
// so it is rw_handler!, not the ro_handler! it was mis-registered as.
rw_handler!(shapes, quipu::tool_shapes);

// quipu #69: named datasets. create/remove WRITE (the datasets tables plus the
// meta-graph mirror), so rw_handler! and a WRITE_ENDPOINTS entry — not the
// ro_handler! that /overlay/create was mis-registered as (aegis-2f4n).
rw_handler!(datasets, quipu::tool_datasets);

// aegis-06q1r: the OWL ontology route. `tool_load_ontology` already both
// persists the ontology AND calls `Ontology::materialize`, which is the whole
// point: measured on aegis-qgqci, asserting `runs_on owl:inverseOf hosts` as a
// bare triple leaves the runs_on/hosts overlap at ZERO, because — unlike
// rdfs:subClassOf, which src/sparql/rdfs.rs backward-chains at query time —
// owl:inverseOf is not chained at all. A load-only route would have registered
// axioms, reported success, and entailed nothing (the same shape as POST
// /shapes, whose triples never reach the queryable store). So this is
// rw_handler! and /ontology is in WRITE_ENDPOINTS.
#[cfg(feature = "owl")]
rw_handler!(ontology, quipu::tool_load_ontology);

// Registered even without the `owl` feature, and deliberately NOT left to 404.
// The parent bead (aegis-1xb10) was slowed by exactly this ambiguity: /ontology
// 404'd, /version reports only `shacl`/`onnx`, and neither signal distinguished
// "not compiled in" from "no route" — so an engine that WAS compiled in read as
// absent. An explicit error names which of the two it is.
#[cfg(not(feature = "owl"))]
pub(crate) async fn ontology(
    State(_s): State<SharedStore>,
    axum::Json(_i): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    Err(quipu::Error::InvalidValue(
        "/ontology is registered but this binary was built WITHOUT the `owl` feature, \
         so no ontology can be loaded or materialized. Rebuild with --features owl \
         (release builds use --features full, which includes it)."
            .into(),
    )
    .into())
}

// Event push P2: subscription registry (create/list/delete via action, the
// /shapes pattern — one POST route, one WRITE_ENDPOINTS entry).
rw_handler!(subscriptions, quipu::tool_subscriptions);
// Named-graph overlays (aegis-g1al / #36). aegis-e163: overlay_create WRITES the
// graphs registry (through a `&self` method — hence it was wrongly ro_handler!),
// so it is rw_handler! now. compose is a genuine read (no mutating call). write
// is a hand-written &mut handler defined below like `knot`.
rw_handler!(overlay_create, quipu::tool_overlay_create);
ro_handler!(overlay_compose, quipu::tool_overlay_compose);
ro_handler!(cooccurrence, quipu::tool_cooccurrence);
// policy_check is the one read handler with its own counter: the metric uses
// /policy/check's OWN outcome vocabulary (satisfied|unsatisfied|unknown) —
// never a block/warn tiering, which is the caller's downstream mapping.
pub(crate) async fn policy_check(
    State(s): State<SharedStore>,
    axum::Json(i): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    let result = quipu::tool_policy_check(&s.lock(), &i)?;
    if let Some(outcome) = result.get("outcome").and_then(|v| v.as_str()) {
        quipu::metrics::metrics().observe_policy_outcome(outcome);
    }
    Ok(axum::Json(result))
}
ro_handler!(verifier_authorized, quipu::tool_verifier_authorized);
ro_handler!(verdict_verify, quipu::tool_verdict_verify);

pub(crate) async fn overlay_write(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        // Hand-written write handler: same drain discipline as rw_handler!.
        let (result, work) = {
            let mut st = store.lock();
            let result = quipu::tool_overlay_write(&mut st, &input)?;
            (result, st.take_deferred_embed())
        };
        if let Some(work) = work {
            finish_deferred_embed(&store, &work)?;
        }
        Ok(axum::Json(result))
    })
    .await
}
// `tool_project` is read-only by default but the `louvain` algorithm can write
// `quipu:memberOfCommunity` facts when `persist:true`, so it needs a mutable store.
rw_handler!(project_graph, quipu::tool_project);
// `/report` is read-only (god-nodes + surprising connections + suggested
// questions; hq-ct27). POST takes a JSON body of options; GET returns the
// report with defaults so a browser/skill can fetch it with no payload.
ro_handler!(report, quipu::tool_report);
pub(crate) async fn report_get(
    State(s): State<SharedStore>,
) -> Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        Ok(axum::Json(quipu::tool_report(
            &s.lock(),
            &serde_json::json!({}),
        )?))
    })
    .await
}
ro_handler!(context, quipu::tool_context);

// aegis-e163: propose/accept/reject all WRITE (insert_proposal / accept_proposal
// / reject_proposal persist proposal state through `&self`), so they are
// rw_handler!, not the ro_handler! they were mis-registered as. list_proposals is
// a genuine read.
rw_handler!(propose_schema_change, quipu::tool_propose_schema_change);
ro_handler!(list_proposals, quipu::tool_list_proposals);
rw_handler!(accept_proposal, quipu::tool_accept_proposal);
rw_handler!(reject_proposal, quipu::tool_reject_proposal);

rw_handler!(episode, quipu::tool_episode);
rw_handler!(episodes_complete, quipu::tool_episodes_complete);
rw_handler!(impact_analysis, quipu::tool_impact);
rw_handler!(retract, quipu::tool_retract);
rw_handler!(set_predicate, quipu::tool_set);
rw_handler!(retract_episode, quipu::tool_retract_episode);

pub(crate) async fn validate(
    State(store): State<SharedStore>,
    axum::Json(input): axum::Json<JsonValue>,
) -> Result<axum::Json<JsonValue>, AppError> {
    // quipu #71: when the request carries no `shapes`, fall back to the STORED
    // ones — optionally as they stood at `valid_at` / `as_of_tx`, defaulting to
    // now. The lock is taken ONLY to fetch the turtle and dropped immediately:
    // SHACL validation is CPU-bound and unbounded in the size of the payload,
    // so validating under it would serialize every other request behind an
    // arbitrary caller's data. That property is why this handler had no lock at
    // all before, and it is preserved rather than traded away.
    let mut input = input;
    let resolved = {
        let guard = store.lock();
        quipu::resolve_validation_shapes(&guard, &input)?
    };
    if let Some(turtle) = resolved
        && let Some(obj) = input.as_object_mut()
    {
        obj.insert("shapes".to_string(), JsonValue::String(turtle));
    }
    blocking(move || Ok(axum::Json(quipu::tool_validate(&input)?))).await
}

pub(crate) fn backfill_embeddings(store: &mut quipu::Store) -> std::result::Result<usize, String> {
    let provider = store.embedding_provider().ok_or(quipu::NO_PROVIDER_HELP)?;
    let result = quipu::sparql_query(store, "SELECT DISTINCT ?s WHERE { ?s ?p ?o }")
        .map_err(|e| format!("{e}"))?;
    let entity_ids: Vec<i64> = result
        .rows()
        .iter()
        .filter_map(|row| match row.get("s") {
            Some(quipu::Value::Ref(id)) => Some(*id),
            _ => None,
        })
        .collect();
    if entity_ids.is_empty() {
        return Ok(0);
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .to_string();
    let mut embedded = 0;
    for chunk in entity_ids.chunks(32) {
        let pairs: Vec<(i64, String)> = chunk
            .iter()
            .filter_map(|&eid| {
                quipu::build_entity_text(store, eid)
                    .ok()
                    .filter(|t| !t.is_empty())
                    .map(|t| (eid, t))
            })
            .collect();
        if pairs.is_empty() {
            continue;
        }
        let texts: Vec<&str> = pairs.iter().map(|(_, t)| t.as_str()).collect();
        let embs = provider.embed_batch(&texts).map_err(|e| e.to_string())?;
        let vs = store.vector_store();
        for ((eid, text), emb) in pairs.iter().zip(embs.iter()) {
            vs.embed_entity(*eid, text, emb, &ts)
                .map_err(|e| e.to_string())?;
            embedded += 1;
        }
    }
    Ok(embedded)
}

pub(crate) async fn embed_backfill(
    State(store): State<SharedStore>,
) -> std::result::Result<axum::Json<JsonValue>, AppError> {
    blocking(move || {
        let mut s = store.lock();
        match backfill_embeddings(&mut s) {
            Ok(n) => Ok(axum::Json(json!({"status": "ok", "entities_embedded": n}))),
            Err(e) => Ok(axum::Json(json!({"status": "error", "error": e}))),
        }
    })
    .await
}
