//! MCP tool handlers for Quipu -- the agent-facing API surface.
//!
//! Each function takes JSON input and returns JSON output, matching the
//! Model Context Protocol tool calling convention. Bobbin's MCP server
//! delegates knowledge graph operations to these handlers.

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

use serde_json::Value as JsonValue;

use crate::error::{Error, Result};
use crate::resolution::EntityCandidate;
use crate::sparql::{self, QueryResult, TemporalContext, rdfs};
use crate::store::Store;
use crate::store::labels;
use crate::types::Value;

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

    let result = sparql::query_temporal(store, query_str, &ctx)?;
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
    let graph = match input.get("graph").and_then(|v| v.as_str()) {
        None => sparql::GraphScope::default(),
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

/// Which type constants in this request were widened by subclass inference.
///
/// Empty when the answer is asserted-only, which is the common case — the field
/// is omitted entirely from the response then, so a reader who sees it knows it
/// means something. See [`crate::sparql::rdfs::expanded_types`] for why a count
/// that looks fine can be answering the other question.
pub fn query_inference(store: &Store, input: &JsonValue) -> Result<Vec<rdfs::ExpandedType>> {
    let (query_str, ctx) = query_context(store, input)?;
    Ok(rdfs::expanded_types(store, query_str, &ctx))
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

fn add_inference(out: &mut JsonValue, expanded: &[rdfs::ExpandedType]) {
    if expanded.is_empty() {
        return;
    }
    let types: Vec<JsonValue> = expanded
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
                // "counts" would be wrong on the two shapes this marker now also
                // reaches: an ASK returns a boolean and CONSTRUCT returns triples.
                "note": "rdfs:subClassOf expansion widened a type constant. For an \
            asserted-only answer use the variable form: ?s a ?t . FILTER(?t = <Type>)",
            }),
        );
    }
}

/// The inference marker as an HTTP header value, or `None` when nothing was
/// inferred.
///
/// The W3C-negotiated response shapes (`application/sparql-results+json|xml`,
/// `text/turtle`) are fixed by spec: there is no place in a `{"head":{},
/// "boolean":true}` document or a Turtle graph to put a marker without emitting
/// something that is no longer the format the caller asked for. So on that path
/// the signal goes out of band, in a header, where it annotates the response
/// without touching the body a conformant parser will read.
///
/// Names only the widened type constants, not their subclass sets — a header is
/// a bounded place and the parents are enough to know the answer is not
/// asserted-only. The full `expandedTypes` detail is one Accept-free request
/// away, in the bespoke JSON shape.
pub fn inference_header(expanded: &[rdfs::ExpandedType]) -> Option<String> {
    if expanded.is_empty() {
        return None;
    }
    Some(
        expanded
            .iter()
            .map(|e| e.type_iri.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// Parse a JSON object-position value into a stored `Value`. Accepts a bare
/// string (treated as a plain literal) or a tagged object
/// `{iri|str|int|float|bool: ...}`. IRIs are interned to a `Value::Ref`.
fn json_to_value(store: &Store, v: &JsonValue) -> Result<crate::types::Value> {
    use crate::types::Value;
    if let Some(s) = v.as_str() {
        return Ok(Value::Str(s.to_string()));
    }
    if let Some(o) = v.as_object() {
        if let Some(iri) = o.get("iri").and_then(JsonValue::as_str) {
            return Ok(Value::Ref(store.intern(iri)?));
        }
        if let Some(s) = o.get("str").and_then(JsonValue::as_str) {
            return Ok(Value::Str(s.to_string()));
        }
        if let Some(n) = o.get("int").and_then(JsonValue::as_i64) {
            return Ok(Value::Int(n));
        }
        if let Some(f) = o.get("float").and_then(JsonValue::as_f64) {
            return Ok(Value::Float(f));
        }
        if let Some(b) = o.get("bool").and_then(JsonValue::as_bool) {
            return Ok(Value::Bool(b));
        }
        // A literal's tag/datatype rides in its own field. There is deliberately
        // no way to smuggle one into the lexical string (aegis-fmyi).
        if let Some(lexical) = o.get("value").and_then(JsonValue::as_str) {
            if let Some(lang) = o.get("lang").and_then(JsonValue::as_str) {
                return Ok(Value::Lang {
                    lexical: lexical.to_string(),
                    lang: lang.to_string(),
                });
            }
            if let Some(datatype) = o.get("datatype").and_then(JsonValue::as_str) {
                return Ok(Value::Typed {
                    lexical: lexical.to_string(),
                    datatype: datatype.to_string(),
                });
            }
        }
    }
    Err(Error::InvalidValue(
        "object must be a string literal, a tagged {iri|str|int|float|bool: ...}, \
         or {value, lang} / {value, datatype}"
            .into(),
    ))
}

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
///           "source": "...", "shapes": "<optional shapes turtle>" }`
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

    let (tx_id, count) = crate::rdf::ingest_rdf(
        store,
        turtle.as_bytes(),
        oxrdfio::RdfFormat::Turtle,
        None,
        timestamp,
        actor,
        source,
    )?;

    Ok(serde_json::json!({
        "conforms": true,
        "tx_id": tx_id,
        "count": count
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
                    "expand_links": { "type": "boolean", "description": "Whether to expand to entities linked from the matches" }
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

pub fn value_to_json(store: &Store, val: &Value) -> JsonValue {
    match val {
        Value::Ref(id) => {
            let iri = store.resolve(*id).unwrap_or_else(|_| format!("ref:{id}"));
            JsonValue::String(iri)
        }
        Value::Str(s) => JsonValue::String(s.clone()),
        // Lang/Typed serialize as objects so the caller gets the CORRECT lexical
        // value AND can recover the tag/datatype. Emitting a bare "hello@en"
        // string here is what aegis-fmyi was filed against; a bare "hello" that
        // silently drops the tag is the same loss one step later.
        Value::Lang { lexical, lang } => serde_json::json!({"value": lexical, "lang": lang}),
        Value::Typed { lexical, datatype } => {
            serde_json::json!({"value": lexical, "datatype": datatype})
        }
        Value::Int(n) => serde_json::json!(n),
        Value::Float(f) => serde_json::json!(f),
        Value::Bool(b) => JsonValue::Bool(*b),
        Value::Bytes(b) => JsonValue::String(format!("<{} bytes>", b.len())),
    }
}
