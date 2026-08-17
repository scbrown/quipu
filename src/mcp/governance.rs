//! Governance and overlay MCP tools -- the agent-discoverable half of the
//! governance gate (quipu-227).
//!
//! Policy evaluation (`quipu_policy_check`), verdict verification
//! (`quipu_verdict_verify`), the verifier registry check
//! (`quipu_verifier_authorized`), provenance co-occurrence
//! (`quipu_cooccurrence`), and the overlay write surface
//! (`quipu_overlay_create` / `_write` / `_compose`). All seven are advertised
//! in [`super::tool_definitions`] and mirrored as REST routes by quipu-server.

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::sparql::{self, QueryResult, TemporalContext};
use crate::store::Store;

use super::{json_to_value, value_to_json};

/// FNV-1a, deterministic across processes and Rust versions (unlike
/// `DefaultHasher`) — the evidence hash is persisted/compared, so it must be
/// stable. Same construction as `episode::fnv1a_64`.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Reject an IRI that could break out of an inlined `<...>` and inject SPARQL.
fn guard_iri(iri: &str) -> Result<()> {
    if iri
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '{' | '}' | '\\'))
    {
        return Err(Error::InvalidValue(
            "IRI must be bare (no whitespace or < > \" { } \\)".into(),
        ));
    }
    Ok(())
}

/// Run a SPARQL ASK over the committed graph at `ctx`, returning its boolean.
fn run_ask(store: &Store, ask: &str, ctx: &TemporalContext) -> Result<bool> {
    match sparql::query_temporal(store, ask, ctx)? {
        QueryResult::Ask(b) => Ok(b),
        _ => Err(Error::InvalidValue(
            "claim/probe must be a SPARQL ASK query".into(),
        )),
    }
}

/// Fetch a single string-literal value of `<subject> <predicate> ?o` from the
/// committed graph (used to read a Policy's stored claim / evidence probe).
fn fetch_scalar(store: &Store, subject: &str, predicate: &str) -> Result<Option<String>> {
    guard_iri(subject)?;
    guard_iri(predicate)?;
    let q = format!("SELECT ?o WHERE {{ <{subject}> <{predicate}> ?o }} LIMIT 1");
    match sparql::query_temporal(store, &q, &TemporalContext::default())? {
        QueryResult::Select { rows, .. } => {
            Ok(rows.first().and_then(|r| r.get("o")).and_then(|v| match v {
                crate::types::Value::Str(s) => Some(s.clone()),
                _ => None,
            }))
        }
        _ => Ok(None),
    }
}

/// Escape a value for safe inlining as a SPARQL string literal (reject the
/// characters that could break out of the quotes or inject).
fn sparql_string_literal(value: &str) -> Result<String> {
    if value.contains(['"', '\n', '\r', '\\']) {
        return Err(Error::InvalidValue(
            "value must not contain a quote, backslash, or newline".into(),
        ));
    }
    Ok(format!("\"{value}\""))
}

/// Is `verifier` registered (Phase-0 root of trust) to attest `predicate_id`?
/// True iff an `aegis:VerifierRegistration` names both. A governed authority
/// check — independent of (and prior to) cryptographic signing.
fn is_registered_verifier(store: &Store, verifier: &str, predicate_id: &str) -> Result<bool> {
    let v = sparql_string_literal(verifier)?;
    let p = sparql_string_literal(predicate_id)?;
    let ask = format!(
        "PREFIX a: <http://aegis.gastown.local/ontology/> \
         ASK {{ ?r a a:VerifierRegistration ; a:verifier {v} ; a:attests {p} }}"
    );
    run_ask(store, &ask, &TemporalContext::default())
}

/// The hex public key a verifier is registered with (Phase-0 root of trust), or
/// `None` if it has no `aegis:VerifierRegistration` carrying a key.
fn registered_public_key(store: &Store, verifier: &str) -> Result<Option<String>> {
    let v = sparql_string_literal(verifier)?;
    let q = format!(
        "PREFIX a: <http://aegis.gastown.local/ontology/> \
         SELECT ?k WHERE {{ ?r a a:VerifierRegistration ; a:verifier {v} ; a:publicKey ?k }} LIMIT 1"
    );
    match sparql::query_temporal(store, &q, &TemporalContext::default())? {
        QueryResult::Select { rows, .. } => {
            Ok(rows.first().and_then(|r| r.get("k")).and_then(|v| match v {
                crate::types::Value::Str(s) => Some(s.clone()),
                _ => None,
            }))
        }
        _ => Ok(None),
    }
}

/// MCP tool: `quipu_verdict_verify` -- verify a signed Verdict against the
/// Phase-0 root of trust: the signature must be valid under the verifier's
/// REGISTERED public key, AND the verifier must be authorized to attest the
/// predicate. `trusted` is the conjunction — the property a consumer should gate
/// on (checked, not trusted-by-assertion).
///
/// Input: the verdict fields `{ predicate_id, target_ref, outcome, evidence_hash,
/// tier?, verifier, signature }`. Output: `{ signature_valid, verifier_registered,
/// verifier_authorized, trusted }`.
pub fn tool_verdict_verify(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let field = |k: &str| -> Result<String> {
        input
            .get(k)
            .and_then(JsonValue::as_str)
            .map(std::string::ToString::to_string)
            .ok_or_else(|| Error::InvalidValue(format!("missing '{k}' parameter")))
    };
    let predicate_id = field("predicate_id")?;
    let target_ref = field("target_ref")?;
    let outcome = field("outcome")?;
    let evidence_hash = field("evidence_hash")?;
    let verifier = field("verifier")?;
    let signature = field("signature")?;
    let tier = input
        .get("tier")
        .and_then(JsonValue::as_str)
        .unwrap_or("committed");

    let message = crate::signing::verdict_message(
        &predicate_id,
        &target_ref,
        &outcome,
        &evidence_hash,
        tier,
        &verifier,
    );
    let pubkey = registered_public_key(store, &verifier)?;
    let signature_valid = pubkey
        .as_deref()
        .is_some_and(|pk| crate::signing::verify_hex(pk, &message, &signature));
    let verifier_authorized = is_registered_verifier(store, &verifier, &predicate_id)?;

    Ok(serde_json::json!({
        "signature_valid": signature_valid,
        "verifier_registered": pubkey.is_some(),
        "verifier_authorized": verifier_authorized,
        "trusted": signature_valid && verifier_authorized
    }))
}

/// MCP tool: `quipu_verifier_authorized` -- check the Phase-0 verifier registry:
/// may `verifier` attest `predicate`? Input: `{ "verifier": "...",
/// "predicate": "..." }`. Output: `{ "authorized": bool }`.
pub fn tool_verifier_authorized(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let verifier = input
        .get("verifier")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'verifier' parameter".into()))?;
    let predicate = input
        .get("predicate")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'predicate' parameter".into()))?;
    Ok(serde_json::json!({
        "authorized": is_registered_verifier(store, verifier, predicate)?
    }))
}

/// MCP tool: `quipu_policy_check` -- committed-tier evaluation of a governance
/// Policy over the graph of record (the loom's Phase-1 runtime).
///
/// Given a Policy (its `aegis:claim` = a SPARQL ASK, optionally with a `$target`
/// placeholder) and a target IRI, evaluates the claim over the committed graph
/// and returns a **Verdict**: `outcome` ∈ {satisfied | unsatisfied | unknown},
/// bound to a reproducible `evidence_hash` of (predicate, target, `valid_at`,
/// bound claim).
///
/// This is the deterministic, reproducible half: any verifier re-runs the same
/// ASK over the same committed evidence and MUST get the same verdict — checked,
/// not trusted. `unknown` (no evidence yet, distinct from unsatisfied) is
/// returned when the policy carries an `aegis:evidenceProbe` (an ASK for "does
/// the evidence exist?") that is false. **Signing + persistence as a stored,
/// bitemporal Verdict fact is Phase 0** (verifier registry + keys); this returns
/// the evaluated verdict UNSIGNED (`"signed": false`).
///
/// Input: `{ "policy": "<iri>", "target": "<iri>", "valid_at"?: "..." }` or inline
/// `{ "claim": "<ASK>", "target": "<iri>", "predicate_id"?: "...",
///    "evidence_probe"?: "<ASK>" }`.
pub fn tool_policy_check(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let target = input
        .get("target")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'target' parameter".into()))?;
    guard_iri(target)?;

    let (claim, predicate_id, evidence_probe) = if let Some(claim) =
        input.get("claim").and_then(JsonValue::as_str)
    {
        (
            claim.to_string(),
            input
                .get("predicate_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("inline")
                .to_string(),
            input
                .get("evidence_probe")
                .and_then(JsonValue::as_str)
                .map(std::string::ToString::to_string),
        )
    } else if let Some(policy) = input.get("policy").and_then(JsonValue::as_str) {
        let claim = fetch_scalar(store, policy, "http://aegis.gastown.local/ontology/claim")?
            .ok_or_else(|| Error::InvalidValue(format!("policy '{policy}' has no aegis:claim")))?;
        let probe = fetch_scalar(
            store,
            policy,
            "http://aegis.gastown.local/ontology/evidenceProbe",
        )?;
        (claim, policy.to_string(), probe)
    } else {
        return Err(Error::InvalidValue(
            "provide either 'policy' or inline 'claim'".into(),
        ));
    };

    let ctx = TemporalContext {
        valid_at: input
            .get("valid_at")
            .and_then(JsonValue::as_str)
            .map(std::string::ToString::to_string),
        as_of_tx: input.get("tx").and_then(serde_json::Value::as_i64),
        ..Default::default()
    };

    let bound_claim = claim.replace("$target", &format!("<{target}>"));
    let outcome = match &evidence_probe {
        Some(probe) => {
            let bound_probe = probe.replace("$target", &format!("<{target}>"));
            if !run_ask(store, &bound_probe, &ctx)? {
                "unknown"
            } else if run_ask(store, &bound_claim, &ctx)? {
                "satisfied"
            } else {
                "unsatisfied"
            }
        }
        None => {
            if run_ask(store, &bound_claim, &ctx)? {
                "satisfied"
            } else {
                "unsatisfied"
            }
        }
    };

    let evidence_hash = format!(
        "fnv1a:{:016x}",
        fnv1a_64(
            format!(
                "{predicate_id}|{target}|{}|{bound_claim}",
                ctx.valid_at.as_deref().unwrap_or("")
            )
            .as_bytes()
        )
    );

    // Sign the verdict if a signing identity is attached (Phase 0 v1). The
    // signature authenticates the attestation; without a key, the verdict is
    // evaluated but unsigned (honestly `signed: false`).
    let tier = "committed";
    let (verifier, signed, signature, verifier_public_key) = match store.signing_identity() {
        Some(id) => {
            let sig = id.sign_verdict(&predicate_id, target, outcome, &evidence_hash, tier);
            (
                id.verifier.clone(),
                true,
                Some(sig),
                Some(id.public_key_hex()),
            )
        }
        None => ("quipu".to_string(), false, None, None),
    };

    // Phase-0 authority: is this verifier registered to attest this predicate?
    // The registry (aegis:VerifierRegistration) is the governed root of trust —
    // a signature is only TRUSTED once a human registers the verifier's key.
    let verifier_authorized = is_registered_verifier(store, &verifier, &predicate_id)?;

    Ok(serde_json::json!({
        "predicate_id": predicate_id,
        "target_ref": target,
        "outcome": outcome,
        "evidence_hash": evidence_hash,
        "tier": tier,
        "verifier": verifier,
        "verifier_authorized": verifier_authorized,
        "signed": signed,
        "signature": signature,
        "verifier_public_key": verifier_public_key
    }))
}

/// MCP tool: `quipu_cooccurrence` -- deterministic, auditable work-item
/// co-occurrence (quipu#37). Given a work-item (`Bead`) IRI, returns the other
/// work-items that share at least one touched code entity, via the provenance
/// chain `Bead <-implements- GitCommit -modifies-> entity`. This is a **graph
/// query, not a statistical mine** — set-overlap over typed provenance edges
/// (Hank promotes the raw `commit -> touched` edge per #18; Quipu aggregates).
///
/// Bitemporal: pass `valid_at` for "which work co-occurred as of `<date>`, and
/// via which commit/actor". Input: `{ "work_item": "<iri>", "valid_at"?: "...",
/// "tx"?: N }`. Output: `{ "work_item": iri, "cooccurring":
/// [{ "work_item": iri, "shared_entities": N }] }` ordered by overlap strength.
pub fn tool_cooccurrence(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let item = input
        .get("work_item")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'work_item' parameter".into()))?;
    // The IRI is inlined into the query, so reject anything that could break out
    // of the `<...>` and inject SPARQL. A real IRI contains none of these.
    if item
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '{' | '}' | '\\'))
    {
        return Err(Error::InvalidValue(
            "'work_item' must be a bare IRI (no whitespace or < > \" { } \\)".into(),
        ));
    }

    let query = format!(
        "PREFIX a: <http://aegis.gastown.local/ontology/> \
         SELECT ?other (COUNT(DISTINCT ?e) AS ?shared) WHERE {{ \
           ?cA a:implements <{item}> . ?cA a:modifies ?e . \
           ?cB a:modifies ?e . ?cB a:implements ?other FILTER(?other != <{item}>) \
         }} GROUP BY ?other ORDER BY DESC(?shared)"
    );

    let ctx = TemporalContext {
        valid_at: input
            .get("valid_at")
            .and_then(JsonValue::as_str)
            .map(std::string::ToString::to_string),
        as_of_tx: input.get("tx").and_then(serde_json::Value::as_i64),
        ..Default::default()
    };
    let result = sparql::query_temporal(store, &query, &ctx)?;

    let cooccurring: Vec<JsonValue> = match result {
        QueryResult::Select { rows, .. } => rows
            .iter()
            .filter_map(|row| {
                let other = row.get("other").map(|v| value_to_json(store, v))?;
                let shared = row.get("shared").map(|v| value_to_json(store, v))?;
                Some(serde_json::json!({ "work_item": other, "shared_entities": shared }))
            })
            .collect(),
        _ => Vec::new(),
    };

    Ok(serde_json::json!({
        "work_item": item,
        "count": cooccurring.len(),
        "cooccurring": cooccurring
    }))
}

/// MCP tool: `quipu_overlay_create` -- register an overlay-class named graph
/// bound (bind-once) to a committed parent branch (aegis-g1al / #36).
///
/// Input: `{ "overlay": "<iri>", "parent_branch": "<iri>" | null }` (null/absent
/// → ROOT). Output: `{ "g": N, "parent_branch": M }`.
pub fn tool_overlay_create(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let overlay = input
        .get("overlay")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'overlay' parameter".into()))?;
    let parent_branch = match input.get("parent_branch").and_then(JsonValue::as_str) {
        Some(iri) => store.intern(iri)?,
        None => 0,
    };
    let g = store.overlay_create(overlay, parent_branch)?;
    Ok(serde_json::json!({ "g": g, "parent_branch": parent_branch }))
}

/// MCP tool: `quipu_graph_create` -- register a committed-class named graph.
///
/// Input: `{ "graph": "<iri>" }`. Output: `{ "g": N, "created": bool }`.
///
/// This exists because `Store::graph_create` had no caller outside its own
/// tests (camayoc-s0h): the registration `set_graph_label` presumes was
/// written and unreachable, so a consumer could write into a named graph via
/// `/episode` — which interns the IRI without registering it — and then find
/// the graph could not be labelled. Routing without labelling is worse than
/// neither, because it produces separate graphs every query still reads at
/// equal trust.
///
/// Idempotent. Re-creating an existing committed graph is a no-op, and
/// `created` reports which happened rather than leaving the caller to guess
/// from a bare success.
pub fn tool_graph_create(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let graph = input
        .get("graph")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'graph' parameter".into()))?;
    let existed = store.lookup(graph)?.is_some();
    let g = store.graph_create(graph)?;
    Ok(serde_json::json!({ "g": g, "created": !existed }))
}

/// MCP tool: `quipu_graph_label` -- declare a named graph's lattice label.
///
/// Input: `{ "graph": "<iri>", "timestamp": "<utc>", "trust": {"iri","chain","rank"}?,
///           "freshness": "<v>"?, "durability": "<v>"?, "policy": ["<token>", ...]?,
///           "valid_to": "<utc>"?, "actor": "<iri>"? }`.
/// Output: `{ "tx_id": N }`.
///
/// The other half of camayoc-s0h. The whole lattice — composition, per-row
/// annotation, query-time floors — was built and reachable only from Rust, so
/// a plane a consumer created could not be labelled `low-trust` even by hand.
///
/// An empty label is REFUSED by the store rather than silently written, and
/// that refusal is deliberately not softened here: a label declaring nothing
/// is almost certainly not what the caller meant, and writing it would look
/// like the graph had been labelled.
pub fn tool_graph_label(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    use crate::lattice::{Durability, Freshness, PolicyClass, Trust};
    use crate::store::labels::GraphLabel;

    let graph = input
        .get("graph")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'graph' parameter".into()))?
        .to_string();
    let timestamp = input
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'timestamp' parameter".into()))?
        .to_string();

    // Each axis is parsed strictly: an unrecognised value is an ERROR, never a
    // dropped axis. A silently ignored `trust` would leave the graph labelled
    // on the other axes and look fully declared.
    let freshness = match input.get("freshness").and_then(JsonValue::as_str) {
        Some(v) => Some(Freshness::parse(v).ok_or_else(|| {
            Error::InvalidValue(format!("unrecognised freshness value '{v}'"))
        })?),
        None => None,
    };
    let durability = match input.get("durability").and_then(JsonValue::as_str) {
        Some(v) => Some(Durability::parse(v).ok_or_else(|| {
            Error::InvalidValue(format!("unrecognised durability value '{v}'"))
        })?),
        None => None,
    };
    let trust = match input.get("trust") {
        Some(t) => {
            let iri = t.get("iri").and_then(JsonValue::as_str).ok_or_else(|| {
                Error::InvalidValue("trust requires 'iri'".into())
            })?;
            let chain = t.get("chain").and_then(JsonValue::as_str).ok_or_else(|| {
                Error::InvalidValue("trust requires 'chain' — a rank without the chain that ranks it is not comparable".into())
            })?;
            let rank = t.get("rank").and_then(JsonValue::as_i64).ok_or_else(|| {
                Error::InvalidValue("trust requires an integer 'rank'".into())
            })?;
            Some(Trust::new(iri, chain, rank))
        }
        None => None,
    };
    let policy = match input.get("policy").and_then(JsonValue::as_array) {
        Some(tokens) => {
            let toks: Vec<String> = tokens
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
            if toks.len() != tokens.len() {
                return Err(Error::InvalidValue(
                    "every policy token must be a string".into(),
                ));
            }
            Some(PolicyClass::new(toks))
        }
        None => None,
    };

    let label = GraphLabel {
        freshness,
        durability,
        trust,
        policy,
    };
    let valid_to = input.get("valid_to").and_then(JsonValue::as_str);
    let actor = input.get("actor").and_then(JsonValue::as_str);

    let tx_id = store.set_graph_label_until(&graph, &label, &timestamp, valid_to, actor)?;
    Ok(serde_json::json!({ "tx_id": tx_id }))
}

/// MCP tool: `quipu_overlay_write` -- write one overlay primitive.
///
/// Input: `{ "overlay": "<iri>", "op": "assert"|"retract"|"tombstone",
///           "subject": "<iri>", "predicate": "<iri>", "object": <value> }`.
/// Output: `{ "tx_id": N }`.
pub fn tool_overlay_write(store: &mut Store, input: &JsonValue) -> Result<JsonValue> {
    use crate::types::Op;
    let overlay_iri = input
        .get("overlay")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'overlay' parameter".into()))?;
    let op = match input.get("op").and_then(JsonValue::as_str) {
        Some("assert") => Op::Assert,
        Some("retract") => Op::Retract,
        Some("tombstone") => Op::Tombstone,
        _ => {
            return Err(Error::InvalidValue(
                "'op' must be one of: assert | retract | tombstone".into(),
            ));
        }
    };
    let subject = input
        .get("subject")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'subject' parameter".into()))?;
    let predicate = input
        .get("predicate")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'predicate' parameter".into()))?;
    let object = input
        .get("object")
        .ok_or_else(|| Error::InvalidValue("missing 'object' parameter".into()))?;

    let overlay_g = store.intern(overlay_iri)?;
    let e = store.intern(subject)?;
    let a = store.intern(predicate)?;
    let value = json_to_value(store, object)?;
    let now = crate::time::now_iso();
    let ts = input
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .unwrap_or(&now);
    let tx_id = store.overlay_write(overlay_g, op, e, a, value, ts)?;
    Ok(serde_json::json!({ "tx_id": tx_id }))
}

/// MCP tool: `quipu_overlay_compose` -- resolve an overlay's composed view over
/// `[overlay > parent-branch-root]` (asserted-and-not-tombstoned, nearest wins).
///
/// Input: `{ "overlay": "<iri>" }`. Output:
/// `{ "triples": [{subject, predicate, object}], "count": N }`.
pub fn tool_overlay_compose(store: &Store, input: &JsonValue) -> Result<JsonValue> {
    let overlay_iri = input
        .get("overlay")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| Error::InvalidValue("missing 'overlay' parameter".into()))?;
    let overlay_g = store
        .lookup(overlay_iri)?
        .ok_or_else(|| Error::InvalidValue(format!("overlay '{overlay_iri}' is not interned")))?;
    let facts = store.compose_view(overlay_g)?;
    let triples: Vec<JsonValue> = facts
        .iter()
        .map(|f| {
            serde_json::json!({
                "subject": store.resolve(f.entity).unwrap_or_else(|_| format!("ref:{}", f.entity)),
                "predicate": store.resolve(f.attribute).unwrap_or_else(|_| format!("ref:{}", f.attribute)),
                "object": value_to_json(store, &f.value),
            })
        })
        .collect();
    Ok(serde_json::json!({ "count": triples.len(), "triples": triples }))
}

#[cfg(test)]
mod graph_registry_tool_tests {
    use super::*;
    use crate::Store;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    /// The pair camayoc-s0h is blocked on, exercised in the order that matters:
    /// register, then label. Before these tools the store could do both and
    /// nothing outside Rust could reach either.
    #[test]
    fn a_registered_graph_can_then_be_labelled_through_the_tools() {
        let mut s = store();
        let created = tool_graph_create(&s, &serde_json::json!({ "graph": "http://ex/g/inferred" }))
            .expect("graph_create");
        assert_eq!(created["created"], serde_json::json!(true));

        let out = tool_graph_label(
            &mut s,
            &serde_json::json!({
                "graph": "http://ex/g/inferred",
                "timestamp": "2026-01-01T00:00:00Z",
                "trust": {"iri": "ex:low", "chain": "ex:chain", "rank": 1},
            }),
        )
        .expect("graph_label on a registered graph");
        assert!(out["tx_id"].as_i64().unwrap() > 0);
    }

    /// The ordering constraint itself: labelling an UNREGISTERED graph must
    /// fail. If it silently succeeded, a consumer could believe a quarantine
    /// plane was labelled when the label had nowhere to live.
    #[test]
    fn labelling_an_unregistered_graph_is_refused() {
        let mut s = store();
        let err = tool_graph_label(
            &mut s,
            &serde_json::json!({
                "graph": "http://ex/g/never-registered",
                "timestamp": "2026-01-01T00:00:00Z",
                "trust": {"iri": "ex:low", "chain": "ex:chain", "rank": 1},
            }),
        );
        assert!(err.is_err(), "an unregistered graph must not be labelable");
    }

    /// Re-registration is idempotent and SAYS SO. A bare success would leave
    /// the caller unable to tell "I created this" from "it was already there",
    /// which is the difference between a fresh plane and one someone else set
    /// up with different labels.
    #[test]
    fn re_creating_reports_that_it_already_existed() {
        let s = store();
        let first = tool_graph_create(&s, &serde_json::json!({ "graph": "http://ex/g/x" })).unwrap();
        let second = tool_graph_create(&s, &serde_json::json!({ "graph": "http://ex/g/x" })).unwrap();
        assert_eq!(first["created"], serde_json::json!(true));
        assert_eq!(second["created"], serde_json::json!(false));
        assert_eq!(first["g"], second["g"], "same graph, same id");
    }

    /// An unrecognised axis value is an ERROR, not a dropped axis. A silently
    /// ignored freshness would leave the graph labelled on its other axes and
    /// read as fully declared.
    #[test]
    fn an_unrecognised_axis_value_is_refused_rather_than_dropped() {
        let mut s = store();
        tool_graph_create(&s, &serde_json::json!({ "graph": "http://ex/g/y" })).unwrap();
        let err = tool_graph_label(
            &mut s,
            &serde_json::json!({
                "graph": "http://ex/g/y",
                "timestamp": "2026-01-01T00:00:00Z",
                "freshness": "sort-of-recent",
            }),
        );
        assert!(err.is_err(), "an unparseable freshness must not be dropped");
    }

    /// A trust rank without its chain is refused: a rank is only comparable
    /// within the chain that declares it.
    #[test]
    fn trust_without_its_chain_is_refused() {
        let mut s = store();
        tool_graph_create(&s, &serde_json::json!({ "graph": "http://ex/g/z" })).unwrap();
        let err = tool_graph_label(
            &mut s,
            &serde_json::json!({
                "graph": "http://ex/g/z",
                "timestamp": "2026-01-01T00:00:00Z",
                "trust": {"iri": "ex:low", "rank": 1},
            }),
        );
        assert!(err.is_err(), "a rank without a chain is not comparable");
    }

    /// An empty label declares nothing. The store refuses it and the tool does
    /// not soften that — writing it would look like the graph had been
    /// labelled when no axis was declared.
    #[test]
    fn an_empty_label_is_refused() {
        let mut s = store();
        tool_graph_create(&s, &serde_json::json!({ "graph": "http://ex/g/w" })).unwrap();
        let err = tool_graph_label(
            &mut s,
            &serde_json::json!({ "graph": "http://ex/g/w", "timestamp": "2026-01-01T00:00:00Z" }),
        );
        assert!(err.is_err(), "a label declaring no axis must be refused");
    }
}
