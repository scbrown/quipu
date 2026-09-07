//! MCP tool: `quipu_retract_source` -- retract-only repair of facts owned by a
//! legacy transaction source.
//!
//! ## Why this exists at all (aegis-rz75m6)
//!
//! Retraction in this store is SOURCE-scoped: `plan_source_retraction` closes
//! every currently-live fact whose transaction source equals a given string.
//! The primitive takes an arbitrary string, but until now the only surface that
//! reached it was `/knot`, which composes the tag itself:
//!
//! ```text
//! let source_tag = format!("snapshot:{}", snapshot.unwrap());   // knot.rs
//! ```
//!
//! So a producer could only ever clear facts it had written under a
//! `snapshot:<key>` tag. Facts written under any OTHER source string were
//! unreachable by ANY retraction, forever. That is not hypothetical: 181 of
//! quipu's own 416 `CodeModule` entities (43.5%) sit on six free-form source
//! strings minted by hand-run `yupana promote` invocations, and no promote,
//! subset promote, or snapshot replacement can remove them. Their deletions
//! have not propagated for up to 36 days.
//!
//! ## Why this is NOT "let `/knot` name a raw source tag"
//!
//! That was the first proposal and it is a privilege grant, not a repair path.
//! `knot.rs` stamps ONE tag onto a transaction that carries both the planned
//! retractions AND the new assertions:
//!
//! ```text
//! datums.append(&mut assertions);
//! let tx_id = store.transact_to_graph(&datums, timestamp, actor, Some(&source_tag), graph)?;
//! ```
//!
//! A raw tag input there therefore lets any caller WRITE facts attributed to
//! any producer — including `snapshot:code:quipu` — and the forged attribution
//! lands in the exact field every ownership decision depends on. Retraction is
//! the half we need; impersonation is the half we would have shipped with it.
//!
//! This tool removes that by CONSTRUCTION rather than by discipline:
//!
//! 1. **Retract only.** There is no `turtle` input. The transaction cannot
//!    carry an assertion because the caller has no way to supply one.
//! 2. **The legacy source is named explicitly** — the thing the repair needs.
//! 3. **The retraction is stamped with its OWN source**, `repair:<ticket>`,
//!    never with the source being cleared. The transaction log says who did the
//!    repair and why; cleared facts are not silently re-attributed to a
//!    producer that did not ask for their removal.
//! 4. **Plan before apply.** `apply` defaults to false, so the default call
//!    reports what WOULD be retracted and writes nothing. `count: 0` then means
//!    "that source owns nothing live", which was previously unanswerable.
//!
//! ## The response reports the POST-STATE, not the request
//!
//! `/knot` answers a retraction that matched nothing with `{"replaced": true,
//! "count": 0}` — it describes the request. Both arms of a rehearsal that
//! destroyed a graph and a rehearsal that changed nothing returned that same
//! body. So an applied call here re-reads the store afterwards and reports
//! `remaining`: how many live facts still carry the named source. That is a
//! measurement, not an echo. It is still not a substitute for an independent
//! audit of the store, and callers doing a real re-key should run one.
//!
//! ## Ordering, for anyone re-keying a producer
//!
//! RETRACT FIRST, then re-promote under the canonical key. The store dedups an
//! identical triple to one row carrying one source, and the existence check
//! ignores the transaction's source — so asserting canonically FIRST is skipped
//! as a duplicate, every row keeps its legacy source, and the subsequent
//! retraction then deletes all of them. Measured both arms (aegis-rz75m6):
//! assert-then-retract took a 59-triple graph to 0.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::mcp::value_to_json;
use crate::store::{Datum, Store};

/// How many planned statements the response echoes back. Enough to eyeball the
/// shape of a plan; never enough to be mistaken for the plan itself.
const SAMPLE_LIMIT: usize = 20;

/// Resolve an optional named-graph IRI to its `g` id, with `/knot`'s rules.
///
/// Kept deliberately identical in behaviour to [`crate::mcp::knot`]'s resolver:
/// a repair must be able to reach exactly the graphs a snapshot write can
/// reach, and no others.
fn resolve_committed_graph(store: &Store, graph: Option<&str>) -> Result<i64> {
    let Some(iri) = graph.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(0);
    };
    let Some(g) = store.lookup(iri)? else {
        return Err(Error::InvalidValue(format!(
            "unknown graph: {iri} — create and register it first via graph_create"
        )));
    };
    match store.graph_class(g)?.as_deref() {
        Some("committed") => Ok(g),
        Some("overlay") => Err(Error::InvalidValue(format!(
            "graph {iri} is an overlay — write through overlay_write, not retract_source"
        ))),
        Some(other) => Err(Error::InvalidValue(format!(
            "graph {iri} has unsupported class '{other}' for repair retraction"
        ))),
        None => Err(Error::InvalidValue(format!(
            "graph {iri} is interned but not registered — register it via graph_create"
        ))),
    }
}

/// Collapse a retraction plan to at most one datum per distinct `(e, a, v)`.
///
/// The store is append-only, so ONE logical triple can be backed by MANY fact
/// rows under the same source — a producer that ran twice re-asserts, and both
/// rows stay live. `facts` PRIMARY KEY is `(e, a, v, tx)`, so putting two rows
/// with the same `(e, a, v)` into a single transaction fails outright with a
/// UNIQUE constraint error. `retraction_datums` already exists in the store for
/// exactly this, but it is `pub(super)` there and the shared
/// `plan_source_retraction` does NOT dedupe.
///
/// ## What is and is not established about this happening
///
/// MEASURED on the live graph (aegis-a0ne, recorded on `retraction_datums`):
/// 86 duplicate groups over 29 of 972 entities, worst group 132 rows — and
/// retraction failed outright on exactly those, because being old and
/// re-asserted is what creates duplicates. So the condition is real in the
/// store this tool will be pointed at.
///
/// It cannot be reproduced through today's assert path: `stage_and_guard` skips
/// an assertion when an active `(e, a, v)` already exists in the graph, so a
/// producer re-running under the same source writes nothing the second time.
/// The live duplicates predate that skip. `dedupe` is therefore tested as a
/// function against a constructed plan rather than end-to-end — an end-to-end
/// fixture PASSES WITHOUT THIS FUNCTION and would be a vacuous guard.
/// Retracting a triple once is also the correct semantics: the duplicate rows
/// are one statement, not N.
pub(super) fn dedupe(plan: Vec<Datum>) -> Vec<Datum> {
    let mut out: Vec<Datum> = Vec::with_capacity(plan.len());
    for d in plan {
        if out
            .iter()
            .any(|k| k.entity == d.entity && k.attribute == d.attribute && k.value == d.value)
        {
            continue;
        }
        out.push(d);
    }
    out
}

/// MCP tool: `quipu_retract_source` -- plan or apply a retract-only repair.
///
/// Input: `{ "source": "<exact legacy transaction source>",
///           "repair": "<ticket or reason>",
///           "apply": false, "expect": <n>,
///           "graph"?: "<registered committed-graph IRI>",
///           "timestamp"?: "...", "actor"?: "..." }`
///
/// Output: `{ "source", "graph", "planned", "entities", "applied", "tx_id",
///            "retracted", "remaining", "repair_source", "sample" }`
pub fn tool_retract_source(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let source = input
        .get("source")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::InvalidValue(
                "missing 'source': the EXACT transaction source string to retract. \
                 Read it from GET /transactions or entity history — it is matched \
                 literally, never by prefix or pattern."
                    .into(),
            )
        })?;

    // The repair ticket is mandatory and is NOT cosmetic: it becomes the
    // transaction source of the retraction itself, which is the only reason
    // this operation is auditable rather than an unattributed disappearance.
    let ticket = input
        .get("repair")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            Error::InvalidValue(
                "missing 'repair': a ticket or reason for this retraction. It is \
                 stamped on the retraction transaction as repair:<ticket>, so the \
                 log records who cleared these facts and why."
                    .into(),
            )
        })?;
    let repair_source = format!("repair:{ticket}");

    let now = crate::time::now_iso();
    let timestamp = input
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .unwrap_or(&now);
    let actor = input.get("actor").and_then(JsonValue::as_str);
    let apply = input
        .get("apply")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);

    let graph = resolve_committed_graph(store, input.get("graph").and_then(JsonValue::as_str))?;
    let graph_iri = store.graph_iri_of(graph);

    let plan = dedupe(store.plan_source_retraction(source, graph)?);
    let planned = plan.len();
    let mut entities: Vec<i64> = plan.iter().map(|d| d.entity).collect();
    entities.sort_unstable();
    entities.dedup();

    let sample: Vec<JsonValue> = plan
        .iter()
        .take(SAMPLE_LIMIT)
        .map(|d| {
            serde_json::json!({
                "entity": store.resolve(d.entity).unwrap_or_default(),
                "predicate": store.resolve(d.attribute).unwrap_or_default(),
                "value": value_to_json(store, &d.value),
            })
        })
        .collect();

    let mut out = serde_json::json!({
        "source": source,
        "graph": graph_iri,
        "planned": planned,
        "entities": entities.len(),
        "applied": false,
        "tx_id": JsonValue::Null,
        "retracted": JsonValue::Null,
        "repair_source": repair_source,
        "sample": sample,
        "sample_truncated": planned > SAMPLE_LIMIT,
    });

    if !apply {
        return Ok(out);
    }

    // Plan-then-apply handshake. `expect` is the count the caller SAW in the
    // plan; a mismatch means either the store moved underneath them or they
    // named a different source than the one they measured. Both are reasons to
    // stop: this operation's blast radius is "every live fact one producer
    // wrote", and a typo in a free-form source string is silent in every other
    // respect. Refusing costs one extra round trip and is the only thing
    // standing between a mis-typed source and an unnoticed mass retraction.
    let expect = input
        .get("expect")
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| {
            Error::InvalidValue(format!(
                "apply requires 'expect': the planned count you are confirming. \
                 This source currently owns {planned} live statement(s) in <{graph_iri}>. \
                 Re-run without 'apply' to see the plan, then pass expect={planned}."
            ))
        })?;
    if expect != planned as i64 {
        return Err(Error::InvalidValue(format!(
            "expect={expect} does not match the current plan of {planned} statement(s) \
             for source {source:?} in <{graph_iri}> — refusing. The store moved since \
             you planned, or this is not the source you measured. Re-plan and confirm."
        )));
    }

    let tx_id = store.transact_to_graph(&plan, timestamp, actor, Some(&repair_source), graph)?;

    // Measure the POST-STATE rather than echoing the request (aegis-rz75m6:
    // `/knot` answers `replaced: true, count: 0` for a retraction that removed
    // nothing, and answered it identically for one that emptied a graph).
    // `remaining` is a fresh read of live facts still carrying the named source.
    let remaining = store.plan_source_retraction(source, graph)?.len();

    out["applied"] = JsonValue::Bool(true);
    out["tx_id"] = JsonValue::from(tx_id);
    out["retracted"] = JsonValue::from(planned);
    out["remaining"] = JsonValue::from(remaining);
    Ok(out)
}
