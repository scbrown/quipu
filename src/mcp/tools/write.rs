//! Mutating tools: `quipu_retract`, `quipu_set`, `quipu_retract_episode`,
//! `quipu_episode`.

use serde_json::Value as JsonValue;

use crate::episode::{self, Episode};
use crate::error::{Error, Result};
use crate::store::Store;
use crate::store::ops::OrphanPolicy;
use crate::types::Fact;

use crate::mcp::value_to_json;

/// MCP tool: `quipu_retract` -- Retract facts for an entity.
///
/// Input: `{ "entity": "<IRI>", "predicate"?: "<IRI>", "value"?: <object-value>,
///           "timestamp"?: "...", "actor"?: "..." }`. `predicate` and `value`
/// narrow the scope; supplying all three retracts exactly ONE `(e, a, v)`
/// statement (aegis-arup ask 1 — episode granularity was the finest handle
/// available, which is what forced a 16x blast radius and the identity-losing
/// rebuild that followed).
pub fn tool_retract(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let entity_iri = input
        .get("entity")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'entity' IRI parameter".into()))?;

    let entity_id = store
        .lookup(entity_iri)?
        .ok_or_else(|| Error::InvalidValue(format!("entity not found: {entity_iri}")))?;

    let predicate_id = if let Some(pred_iri) = input.get("predicate").and_then(|v| v.as_str()) {
        Some(
            store
                .lookup(pred_iri)?
                .ok_or_else(|| Error::InvalidValue(format!("predicate not found: {pred_iri}")))?,
        )
    } else {
        None
    };

    let now = crate::time::now_iso();
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(&now);

    let actor = input.get("actor").and_then(|v| v.as_str());

    // With entity + predicate + value this is TRIPLE-LEVEL retraction: exactly
    // one statement, instead of "retract the whole episode and rebuild"
    // (aegis-arup).
    let value = match input.get("value") {
        Some(v) => Some(crate::mcp::json_to_value(store, v)?),
        None => None,
    };

    let (tx_id, count) = store.retract_triples(
        entity_id,
        predicate_id,
        value.as_ref(),
        timestamp,
        actor,
        input
            .get("allow_orphan")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    )?;

    Ok(serde_json::json!({
        "tx_id": tx_id,
        "retracted": count,
        "entity": entity_iri
    }))
}

/// MCP tool: `quipu_set` -- Atomic single-call supersede.
///
/// Sets `(entity, predicate)` to exactly `value`: every current object on that
/// predicate is retracted and the new value asserted in ONE transaction, so
/// re-parenting (`reports_to` A -> B) is one call with no window where the
/// predicate is empty and no way to end up multi-valued by forgetting the
/// retract half.
///
/// Input: `{ "entity": "<iri>", "predicate": "<iri>", "value": <object>,
///           "timestamp"?: "...", "actor"?: "..." }`.
/// The value uses the same shape discipline as `/retract`:
/// a bare string is a LITERAL; an edge must be `{"iri": "..."}`. A bare
/// string aimed at a Ref-holding or IRI-shaped target is a loud error, not a
/// mis-shaped write.
///
/// Output: `{ "tx_id", "retracted": N, "asserted": 0|1, "entity", "predicate" }`.
/// Setting the already-sole-current value is an idempotent no-op
/// (`tx_id: 0, retracted: 0, asserted: 0`).
///
/// SINGLE-VALUE semantics: ALL current objects are replaced. For
/// add-without-remove, assert via `/knot`.
///
/// The entity must already exist (same rule as `/retract` — /set on a typo'd
/// IRI must not mint an unlabelled orphan node). The predicate MAY be new:
/// first-time `set` of a property is a legitimate write.
pub fn tool_set(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let entity_iri = input
        .get("entity")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'entity' IRI parameter".into()))?;

    let entity_id = store
        .lookup(entity_iri)?
        .ok_or_else(|| Error::InvalidValue(format!("entity not found: {entity_iri}")))?;

    let predicate_iri = input
        .get("predicate")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::InvalidValue("missing 'predicate' IRI parameter".into()))?;
    let predicate_id = store.intern(predicate_iri)?;

    let value_json = input
        .get("value")
        .ok_or_else(|| Error::InvalidValue("missing 'value' parameter".into()))?;
    let value = crate::mcp::json_to_value(store, value_json)?;
    // {"str": ...} is a STATED literal intent — it disarms the bare-string
    // IRI-shape heuristic (json_to_value collapses both spellings, so the
    // distinction must be carried explicitly).
    let explicit_str = value_json
        .get("str")
        .and_then(serde_json::Value::as_str)
        .is_some();

    let now = crate::time::now_iso();
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(&now);
    let actor = input.get("actor").and_then(|v| v.as_str());

    let (tx_id, retracted, asserted) = store.set_triple(
        entity_id,
        predicate_id,
        value,
        timestamp,
        actor,
        explicit_str,
    )?;

    Ok(serde_json::json!({
        "tx_id": tx_id,
        "retracted": retracted,
        "asserted": asserted,
        "entity": entity_iri,
        "predicate": predicate_iri
    }))
}

/// MCP tool: `quipu_retract_episode` -- Episode-scoped logical retraction (aegis-hxb).
///
/// Retracts every currently-active fact contributed by an episode's ingest,
/// using the existing bitemporal `Op::Retract`/`valid_to` close path. Logical,
/// not physical: time-travel queries still show the retracted facts. Entities
/// and facts from other episodes are untouched (see [`Store::retract_episode`]).
///
/// Input: `{ "episode": "<name>", "timestamp"?: "...", "actor"?: "...",
///            "on_orphan"?: "preserve" | "refuse" | "allow" }`.
/// `episode_id` and `name` are accepted as aliases for `episode`;
/// `orphan_policy` for `on_orphan`.
/// Output: `{ "tx_id", "retracted": <count>, "episode": "<name>",
///            "statements": [{ "entity", "predicate", "value" }, ...],
///            "on_orphan", "identity_preserved": <count>,
///            "identity_preserved_statements": [...],
///            "identity_orphans": <count>,
///            "identity_orphan_entities": [{ "entity", "lost_label",
///                                           "lost_type" }, ...] }`.
///
/// **Ghost nodes (aegis-arup).** Identity triples are ordinary facts, so
/// scope-only retraction strips `rdfs:label` / `rdf:type` from any node this
/// episode named while edges from other episodes keep it alive — a node that
/// answers predicate queries and is invisible to every label scan and type
/// query. `on_orphan` decides the contract; it defaults to `preserve`, which
/// keeps identity alive for nodes that retain surviving references. Whatever
/// the policy, `identity_orphans` names the affected nodes: a caller can now
/// tell a cleanup from a mutilation, which a bare `{"retracted": N}` could not.
///
/// Idempotent: retracting an already-retracted (or unknown) episode returns
/// `retracted: 0` and changes nothing.
///
/// **Auth (hq-azs / hq-otm).** Retraction is a write, and a *more* sensitive one
/// than assertion — it removes facts from current views. The HTTP route
/// (`/episode/retract`) is registered in `http_auth::WRITE_ENDPOINTS`, so today
/// it honours read-only mode and the bearer token exactly like other writes.
/// When per-principal scopes (hq-azs) and crew identity (hq-otm) land,
/// retraction should require an elevated/authorized principal rather than the
/// same token that gates assertion.
pub fn tool_retract_episode(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let episode_name = input
        .get("episode")
        .or_else(|| input.get("episode_id"))
        .or_else(|| input.get("name"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::InvalidValue("missing 'episode' (or 'episode_id') parameter".into())
        })?;

    let now = crate::time::now_iso();
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(&now);

    let actor = input.get("actor").and_then(|v| v.as_str());

    let policy = match input
        .get("on_orphan")
        .or_else(|| input.get("orphan_policy"))
        .and_then(|v| v.as_str())
    {
        Some(s) => OrphanPolicy::parse(s).ok_or_else(|| {
            Error::InvalidValue(format!(
                "invalid 'on_orphan': {s:?} (expected preserve | refuse | allow)"
            ))
        })?,
        None => OrphanPolicy::default(),
    };

    let outcome = store.retract_episode_with_policy(episode_name, timestamp, actor, policy)?;

    fn to_statement(store: &Store, f: &Fact) -> JsonValue {
        serde_json::json!({
            "entity": store.resolve(f.entity).unwrap_or_default(),
            "predicate": store.resolve(f.attribute).unwrap_or_default(),
            "value": value_to_json(store, &f.value),
        })
    }

    let statements: Vec<JsonValue> = outcome
        .retracted
        .iter()
        .map(|f| to_statement(store, f))
        .collect();
    let preserved: Vec<JsonValue> = outcome
        .preserved_identity
        .iter()
        .map(|f| to_statement(store, f))
        .collect();
    let orphans: Vec<JsonValue> = outcome
        .orphans
        .iter()
        .map(|o| {
            serde_json::json!({
                "entity": store.resolve(o.entity).unwrap_or_default(),
                "lost_label": o.lost_label,
                "lost_type": o.lost_type,
            })
        })
        .collect();

    Ok(serde_json::json!({
        "tx_id": outcome.tx_id,
        "retracted": outcome.retracted.len(),
        "episode": episode_name,
        "statements": statements,
        // aegis-arup: the caller must be able to tell a cleanup from a
        // mutilation. Under `preserve` these nodes kept their identity; under
        // `allow` they are now ghosts and need an identity re-post.
        "on_orphan": outcome.policy.as_str(),
        "identity_preserved": preserved.len(),
        "identity_preserved_statements": preserved,
        "identity_orphans": orphans.len(),
        "identity_orphan_entities": orphans
    }))
}

/// MCP tool: `quipu_episode` -- Ingest structured knowledge from an agent episode.
pub fn tool_episode(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    let ep: Episode = serde_json::from_value(input.clone())
        .map_err(|e| Error::InvalidValue(format!("invalid episode JSON: {e}")))?;

    let now = crate::time::now_iso();
    let timestamp = input
        .get("timestamp")
        .and_then(|v| v.as_str())
        .unwrap_or(&now);

    // Apply the configured entity-resolution policy so dedup fires on ingest
    // (hq-uye). Opts are cloned out of the store before the &mut borrow.
    let opts = episode::IngestResolutionOpts::from_config(store.resolution_config());
    // Mint IRIs under the CONFIGURED namespace, not the hardcoded aegis default
    // (aegis-4h3x). Read before the &mut borrow, same as opts.
    let base_ns = store.base_ns().to_string();
    let result =
        episode::ingest_episode_with_resolution(store, &ep, timestamp, &base_ns, Some(&opts))?;

    let mut response = serde_json::json!({
        "tx_id": result.tx_id,
        "count": result.count,
        // BRANCH ON THIS, NOT ON `count`. `unchanged` means the
        // identical episode was already recorded — success, and the answer a
        // caller retrying after a lost response needs. `count: 0` alone cannot
        // say that, and reads as a failed write.
        "outcome": result.outcome.as_str(),
        "episode": ep.name,
        "resolution_hints": crate::mcp::resolution_hints_json(&result.resolution_hints),
        "resolution_contentions":
            crate::mcp::resolution_contentions_json(&result.resolution_contentions)
    });
    // Present ONLY when the episode typed something no shape governs, so the
    // field's mere existence is the signal (aegis-7n1ya). Always-present
    // fields get skimmed; this one has to be noticed the once it appears.
    if let Some(hint) = crate::vocabulary::hint_json(result.vocabulary_hints) {
        response["vocabulary_hint"] = hint;
    }
    Ok(response)
}
