//! Write-path policy enforcement — "edit hooks for policy" (the loom).
//!
//! See `docs/design/policy-edit-hooks.md`. A [`PolicyRegistry`] caches the
//! active `boundary:"action"` governance policies indexed by target-type IRI;
//! [`PolicyRegistry::evaluate_write`] runs the applicable claims against the
//! **pending post-state** (the datums are already staged in the open savepoint
//! when the guard runs) and returns `Err(PolicyDenied)` when a blocking policy's
//! claim is unsatisfied for a touched target (see [`effect_blocks`]).
//!
//! Runtime-gated by `[quipu.governance] enforce_on_write` (default off). The
//! registry is built once and cached on the [`Store`]; a write that defines or
//! amends a policy invalidates it (see [`is_governance_write`]).

use std::collections::HashMap;

use rusqlite::params;

use crate::error::{Error, Result};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::sparql::{self, QueryResult, TemporalContext};
use crate::store::{Datum, Store};
use crate::types::Value;

/// A governance policy compiled into the registry for fast write-time checks.
#[derive(Debug, Clone)]
struct CompiledPolicy {
    /// The policy's IRI (for diagnostics).
    policy_iri: String,
    /// The target entity type IRI (the string carried by `aegis:targets`).
    target_type_iri: String,
    /// The `aegis:claim` SPARQL ASK (the compliant condition), with `$target`.
    claim: String,
    /// The `aegis:effect`. `deny`/`require-approval`/`escalate` block at the
    /// write gate (see [`effect_blocks`]); absent effect defaults to `"deny"`
    /// (fail-closed for an action-boundary policy that carries a claim). See the
    /// design doc for the full table.
    effect: String,
    /// Optional `aegis:evidenceProbe` ASK ("does the evidence exist yet?"). When
    /// present and false, the outcome is `unknown` and the write is NOT blocked.
    evidence_probe: Option<String>,
    /// `aegis:reversibilityWindowSeconds`, for an escalating effect. The
    /// placement check requires it on an escalation-class policy at definition
    /// time, so a missing one here means that check was off — and the router
    /// treats a zero window as already expired rather than inventing a bound
    /// SARC I4 requires be declared.
    reversibility_window: Option<i64>,
}

/// The active `boundary:"action"` policies, indexed by target-type IRI for a
/// fast touched-type pre-filter. A write that touches no governed type runs
/// zero claim ASKs.
#[derive(Debug, Clone, Default)]
pub struct PolicyRegistry {
    by_type: HashMap<String, Vec<CompiledPolicy>>,
}

impl PolicyRegistry {
    /// Load every active action-boundary policy from `store` and index it by
    /// target-type IRI. Metadata (claim/effect/probe) is captured here so the
    /// per-edit path never re-`SELECT`s it.
    pub fn build(store: &Store) -> Result<Self> {
        let q = format!(
            "PREFIX a: <{DEFAULT_BASE_NS}> \
             SELECT ?p ?t ?c ?e ?probe ?window WHERE {{ \
                ?p a a:Policy ; a:targets ?t ; a:claim ?c ; a:boundary \"action\" . \
                OPTIONAL {{ ?p a:effect ?e }} \
                OPTIONAL {{ ?p a:evidenceProbe ?probe }} \
                OPTIONAL {{ ?p a:reversibilityWindowSeconds ?window }} \
             }}"
        );
        let mut by_type: HashMap<String, Vec<CompiledPolicy>> = HashMap::new();
        if let QueryResult::Select { rows, .. } = sparql::query(store, &q)? {
            for row in rows {
                let policy_iri = iri_of(store, row.get("p"))?;
                let (Some(target_type_iri), Some(claim)) =
                    (str_of(row.get("t")), str_of(row.get("c")))
                else {
                    continue;
                };
                let effect = str_of(row.get("e")).unwrap_or_else(|| "deny".to_string());
                let evidence_probe = str_of(row.get("probe"));
                let reversibility_window = match row.get("window") {
                    Some(Value::Int(i)) => Some(*i),
                    Some(Value::Str(s)) => s.parse().ok(),
                    _ => None,
                };
                by_type
                    .entry(target_type_iri.clone())
                    .or_default()
                    .push(CompiledPolicy {
                        policy_iri,
                        target_type_iri,
                        claim,
                        effect,
                        evidence_probe,
                        reversibility_window,
                    });
            }
        }
        Ok(Self { by_type })
    }

    /// Evaluate the applicable action-boundary policies for a write. Returns
    /// `Err(PolicyDenied)` on the first blocking policy whose claim is
    /// unsatisfied for a touched target; otherwise `Ok(())`.
    pub fn evaluate_write(
        &self,
        store: &Store,
        datums: &[Datum],
        graph: i64,
        verdicts: &mut Vec<super::verdict_facts::PendingVerdict>,
        requests: &mut Vec<super::router::PendingRequest>,
    ) -> Result<()> {
        if self.by_type.is_empty() {
            return Ok(());
        }
        // No types interned at all → nothing can match a target type.
        let Some(rdf_type_id) = store.lookup(RDF_TYPE)? else {
            return Ok(());
        };

        // Touched entities (assert + retract), deduplicated.
        let mut touched: Vec<i64> = datums.iter().map(|d| d.entity).collect();
        touched.sort_unstable();
        touched.dedup();

        for e in touched {
            let type_iris = entity_type_iris(store, e, rdf_type_id, graph)?;
            let mut entity_iri: Option<String> = None;
            for tiri in &type_iris {
                let Some(policies) = self.by_type.get(tiri.as_str()) else {
                    continue;
                };
                // Resolve the entity IRI lazily, once, only if a type matched.
                let eiri = match &entity_iri {
                    Some(s) => s.clone(),
                    None => {
                        let s = store.resolve(e)?;
                        entity_iri = Some(s.clone());
                        s
                    }
                };
                for policy in policies {
                    evaluate_one(store, &eiri, policy, verdicts, requests)?;
                }
            }
        }
        Ok(())
    }
}

/// The active `rdf:type` IRIs of `entity` in its own graph or ROOT, read from
/// the pending post-state (same connection sees the open savepoint).
fn entity_type_iris(
    store: &Store,
    entity: i64,
    rdf_type_id: i64,
    graph: i64,
) -> Result<Vec<String>> {
    let mut stmt = store.prepare(
        "SELECT DISTINCT v FROM facts \
             WHERE e = ?1 AND a = ?2 AND op = 1 AND valid_to IS NULL AND (g = ?3 OR g = 0)",
    )?;
    let rows = stmt.query_map(params![entity, rdf_type_id, graph], |row| {
        let v: Vec<u8> = row.get(0)?;
        Ok(v)
    })?;
    let mut out = Vec::new();
    for r in rows {
        if let Value::Ref(type_id) = Value::from_bytes(&r?)?
            && let Ok(iri) = store.resolve(type_id)
        {
            out.push(iri);
        }
    }
    Ok(out)
}

/// Whether an effect blocks the write at the action boundary.
///
/// `deny` blocks outright. `require-approval` and `escalate` block too, but they
/// now block THROUGH THE ROUTER (`super::router`): the refusal mints a
/// `DecisionRequest` naming what would un-refuse it and when the absence of a
/// ruling becomes a denial, and an approval bound to the same evidence lets the
/// next attempt through. Before that channel existed they failed closed with no
/// way forward, which is a refusal an operator cannot act on.
///
/// `allow`, `warn`, `record` and `throttle` are advisory and never block here.
/// `throttle` is the soft-class PAA response and has no meaning at a write gate
/// that cannot act on a successor.
fn effect_blocks(effect: &str) -> bool {
    matches!(effect, "deny" | "require-approval" | "escalate")
}

/// Whether an effect routes to a human rather than refusing outright.
fn effect_escalates(effect: &str) -> bool {
    matches!(effect, "require-approval" | "escalate")
}

/// Evaluate a single policy against a single target entity. A non-blocking
/// effect runs no ASK (nothing to enforce at the write gate).
fn evaluate_one(
    store: &Store,
    entity_iri: &str,
    policy: &CompiledPolicy,
    verdicts: &mut Vec<super::verdict_facts::PendingVerdict>,
    requests: &mut Vec<super::router::PendingRequest>,
) -> Result<()> {
    if !effect_blocks(&policy.effect) {
        return Ok(());
    }
    guard_iri(entity_iri)?;
    let target = format!("<{entity_iri}>");

    let mut stage = |outcome: &str| {
        verdicts.push(super::verdict_facts::PendingVerdict {
            predicate_id: policy.policy_iri.clone(),
            target_ref: entity_iri.to_string(),
            outcome: outcome.to_string(),
        });
    };

    // Evidence probe: if the evidence does not exist yet the outcome is
    // `unknown` (distinct from unsatisfied) and the write is NOT blocked.
    if let Some(probe) = &policy.evidence_probe {
        let bound_probe = probe.replace("$target", &target);
        if !run_ask(store, &bound_probe)? {
            // Recorded as `unknown`, not skipped. "No evidence yet" and "never
            // evaluated" are different facts, and an absent verdict makes the
            // gate look as though the policy did not apply.
            stage("unknown");
            return Ok(());
        }
    }

    let bound_claim = policy.claim.replace("$target", &target);
    if run_ask(store, &bound_claim)? {
        stage("satisfied");
        return Ok(());
    }
    stage("unsatisfied");

    // An escalating effect consults the router before refusing. A standing
    // approval bound to this evidence lets the write through — that is the
    // channel `require-approval` never had, and the reason it is no longer a
    // dead end.
    if effect_escalates(&policy.effect) {
        let now = now_secs();
        if let Some(ruling) = super::router::resolve(store, &policy.policy_iri, entity_iri, now)? {
            if ruling.permits() {
                return Ok(());
            }
            return Err(Error::PolicyDenied(format!(
                "'{entity_iri}' blocked by policy '{}': {}",
                policy.policy_iri,
                ruling.reason(&policy.policy_iri, entity_iri)
            )));
        }
        // No request yet: this attempt is what opens one. The request itself is
        // staged rather than written here — the gate runs inside the savepoint
        // this refusal is about to roll back, so a request written now would
        // vanish with it. Same ordering problem, same answer, as the verdicts.
        requests.push(super::router::PendingRequest {
            policy_iri: policy.policy_iri.clone(),
            target_iri: entity_iri.to_string(),
            window_secs: policy.reversibility_window.unwrap_or(0),
            now,
        });
        return Err(Error::PolicyDenied(format!(
            "'{entity_iri}' needs a human decision under policy '{}'. A \
             DecisionRequest has been opened; have an authorized operator record \
             an aegis:Decision with outcome \"approve\" bound to its \
             evidenceHash, then retry.",
            policy.policy_iri
        )));
    }

    Err(Error::PolicyDenied(format!(
        "'{entity_iri}' blocked by policy '{}' (effect '{}', target type '{}'): claim unsatisfied",
        policy.policy_iri, policy.effect, policy.target_type_iri
    )))
}

/// Unix seconds, or 0 before the epoch.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Run a SPARQL ASK and return its boolean, erroring if the query is not an ASK.
fn run_ask(store: &Store, ask: &str) -> Result<bool> {
    match sparql::query_temporal(store, ask, &TemporalContext::default())? {
        QueryResult::Ask(b) => Ok(b),
        _ => Err(Error::InvalidValue(
            "policy claim/probe must be a SPARQL ASK query".into(),
        )),
    }
}

/// Reject an IRI that could break out of an inlined `<...>` and inject SPARQL.
fn guard_iri(iri: &str) -> Result<()> {
    if iri
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '{' | '}' | '\\'))
    {
        return Err(Error::InvalidValue(
            "target IRI must be bare (no whitespace or < > \" { } \\)".into(),
        ));
    }
    Ok(())
}

fn str_of(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn iri_of(store: &Store, v: Option<&Value>) -> Result<String> {
    match v {
        Some(Value::Ref(id)) => store.resolve(*id),
        Some(Value::Str(s)) => Ok(s.clone()),
        _ => Err(Error::InvalidValue("policy subject is not bound".into())),
    }
}

/// True if any datum defines or amends a governance policy — i.e. writes an
/// `aegis:{targets,claim,boundary,effect,evidenceProbe}` fact or asserts an
/// `rdf:type aegis:Policy`. When true, the cached [`PolicyRegistry`] is stale.
/// Cheap: integer term-id compares over the datums after a handful of interned
/// lookups.
pub fn is_governance_write(store: &Store, datums: &[Datum]) -> Result<bool> {
    let mut pred_ids = Vec::new();
    for p in ["targets", "claim", "boundary", "effect", "evidenceProbe"] {
        if let Some(id) = store.lookup(&format!("{DEFAULT_BASE_NS}{p}"))? {
            pred_ids.push(id);
        }
    }
    let rdf_type_id = store.lookup(RDF_TYPE)?;
    let policy_type_id = store.lookup(&format!("{DEFAULT_BASE_NS}Policy"))?;
    for d in datums {
        if pred_ids.contains(&d.attribute) {
            return Ok(true);
        }
        if rdf_type_id == Some(d.attribute)
            && let Value::Ref(v) = &d.value
            && policy_type_id == Some(*v)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;
