//! Write-path policy enforcement — "edit hooks for policy" (the loom).
//!
//! See `docs/design/policy-edit-hooks.md`. A [`PolicyRegistry`] caches the
//! active `boundary:"action"` governance policies indexed by target-type IRI;
//! [`PolicyRegistry::evaluate_write`] runs the applicable claims against the
//! **pending post-state** (the datums are already staged in the open savepoint
//! when the guard runs) and returns `Err(PolicyDenied)` when a `deny` policy's
//! claim is unsatisfied for a touched target.
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
    /// The `aegis:effect`. Only `"deny"` blocks at the write gate in v1;
    /// absent effect defaults to `"deny"` (fail-closed for an action-boundary
    /// policy that carries a claim). See the design doc for the full table.
    effect: String,
    /// Optional `aegis:evidenceProbe` ASK ("does the evidence exist yet?"). When
    /// present and false, the outcome is `unknown` and the write is NOT blocked.
    evidence_probe: Option<String>,
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
             SELECT ?p ?t ?c ?e ?probe WHERE {{ \
                ?p a a:Policy ; a:targets ?t ; a:claim ?c ; a:boundary \"action\" . \
                OPTIONAL {{ ?p a:effect ?e }} \
                OPTIONAL {{ ?p a:evidenceProbe ?probe }} \
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
                by_type
                    .entry(target_type_iri.clone())
                    .or_default()
                    .push(CompiledPolicy {
                        policy_iri,
                        target_type_iri,
                        claim,
                        effect,
                        evidence_probe,
                    });
            }
        }
        Ok(Self { by_type })
    }

    /// Evaluate the applicable action-boundary policies for a write. Returns
    /// `Err(PolicyDenied)` on the first `deny` policy whose claim is unsatisfied
    /// for a touched target; otherwise `Ok(())`.
    pub fn evaluate_write(&self, store: &Store, datums: &[Datum], graph: i64) -> Result<()> {
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
                    evaluate_one(store, &eiri, policy)?;
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

/// Evaluate a single policy against a single target entity. Only `deny` blocks
/// in v1; other effects run no ASK (nothing to enforce at the write gate).
fn evaluate_one(store: &Store, entity_iri: &str, policy: &CompiledPolicy) -> Result<()> {
    if policy.effect != "deny" {
        return Ok(());
    }
    guard_iri(entity_iri)?;
    let target = format!("<{entity_iri}>");

    // Evidence probe: if the evidence does not exist yet the outcome is
    // `unknown` (distinct from unsatisfied) and the write is NOT blocked.
    if let Some(probe) = &policy.evidence_probe {
        let bound_probe = probe.replace("$target", &target);
        if !run_ask(store, &bound_probe)? {
            return Ok(());
        }
    }

    let bound_claim = policy.claim.replace("$target", &target);
    if run_ask(store, &bound_claim)? {
        Ok(())
    } else {
        Err(Error::PolicyDenied(format!(
            "'{entity_iri}' violates policy '{}' (target type '{}'): claim unsatisfied",
            policy.policy_iri, policy.target_type_iri
        )))
    }
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
mod tests {
    use crate::error::Error;
    use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
    use crate::sparql::{self, QueryResult};
    use crate::store::{Datum, Store};
    use crate::types::{Op, Value};

    const TS: &str = "2026-01-01T00:00:00Z";
    const RDFS_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
    const DOC_TYPE: &str = "http://ex/Doc";
    const NOTE_TYPE: &str = "http://ex/Note";
    /// A `deny` claim: the target must carry an `rdfs:label`.
    const REQUIRE_LABEL: &str = "ASK { $target <http://www.w3.org/2000/01/rdf-schema#label> ?l }";

    fn assert_datum(store: &Store, s: &str, p: &str, v: Value) -> Datum {
        Datum {
            entity: store.intern(s).unwrap(),
            attribute: store.intern(p).unwrap(),
            value: v,
            valid_from: TS.to_string(),
            valid_to: None,
            op: Op::Assert,
        }
    }

    fn type_ref(store: &Store, type_iri: &str) -> Value {
        Value::Ref(store.intern(type_iri).unwrap())
    }

    /// Define an action-boundary `deny` policy: entities of `target_type` must
    /// satisfy `claim`.
    fn define_policy(store: &mut Store, policy_iri: &str, target_type: &str, claim: &str) {
        let policy_class = format!("{DEFAULT_BASE_NS}Policy");
        let datums = vec![
            assert_datum(store, policy_iri, RDF_TYPE, type_ref(store, &policy_class)),
            assert_datum(
                store,
                policy_iri,
                &format!("{DEFAULT_BASE_NS}targets"),
                Value::Str(target_type.to_string()),
            ),
            assert_datum(
                store,
                policy_iri,
                &format!("{DEFAULT_BASE_NS}claim"),
                Value::Str(claim.to_string()),
            ),
            assert_datum(
                store,
                policy_iri,
                &format!("{DEFAULT_BASE_NS}boundary"),
                Value::Str("action".to_string()),
            ),
            assert_datum(
                store,
                policy_iri,
                &format!("{DEFAULT_BASE_NS}effect"),
                Value::Str("deny".to_string()),
            ),
        ];
        store.transact(&datums, TS, None, None).unwrap();
    }

    fn has_any_fact(store: &Store, subject: &str) -> bool {
        let q = format!("ASK {{ <{subject}> ?p ?o }}");
        matches!(sparql::query(store, &q).unwrap(), QueryResult::Ask(true))
    }

    #[test]
    fn deny_blocks_noncompliant_write() {
        let mut store = Store::open_in_memory().unwrap();
        store.governance_config_mut().enforce_on_write = true;
        define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

        // A Doc with no label violates the deny policy.
        let bad = vec![assert_datum(
            &store,
            "http://ex/d1",
            RDF_TYPE,
            type_ref(&store, DOC_TYPE),
        )];
        let err = store.transact(&bad, TS, None, None);
        assert!(
            matches!(err, Err(Error::PolicyDenied(_))),
            "expected policy denial, got {err:?}"
        );
        assert!(
            !has_any_fact(&store, "http://ex/d1"),
            "a denied write must leave the store byte-identical (no facts)"
        );

        // A Doc WITH a label, staged in one txn, satisfies the claim.
        let good = vec![
            assert_datum(&store, "http://ex/d2", RDF_TYPE, type_ref(&store, DOC_TYPE)),
            assert_datum(&store, "http://ex/d2", RDFS_LABEL, Value::Str("hi".into())),
        ];
        store
            .transact(&good, TS, None, None)
            .expect("a compliant write passes the gate");
        assert!(has_any_fact(&store, "http://ex/d2"));
    }

    #[test]
    fn enforcement_off_is_a_noop() {
        let mut store = Store::open_in_memory().unwrap();
        store.governance_config_mut().enforce_on_write = false;
        define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

        // The same non-compliant write succeeds when enforcement is disabled.
        let bad = vec![assert_datum(
            &store,
            "http://ex/d1",
            RDF_TYPE,
            type_ref(&store, DOC_TYPE),
        )];
        store
            .transact(&bad, TS, None, None)
            .expect("no enforcement → write is not gated");
        assert!(has_any_fact(&store, "http://ex/d1"));
    }

    #[test]
    fn ungoverned_type_is_not_checked() {
        let mut store = Store::open_in_memory().unwrap();
        store.governance_config_mut().enforce_on_write = true;
        define_policy(&mut store, "http://ex/P1", DOC_TYPE, REQUIRE_LABEL);

        // A Note has no policy targeting it — the pre-filter skips it entirely.
        let note = vec![assert_datum(
            &store,
            "http://ex/n1",
            RDF_TYPE,
            type_ref(&store, NOTE_TYPE),
        )];
        store
            .transact(&note, TS, None, None)
            .expect("a write touching no governed type is not gated");
        assert!(has_any_fact(&store, "http://ex/n1"));
    }

    #[test]
    fn registry_invalidated_when_a_policy_is_added() {
        let mut store = Store::open_in_memory().unwrap();
        store.governance_config_mut().enforce_on_write = true;

        // First enforced write builds an (empty) registry and caches it.
        let n1 = vec![assert_datum(
            &store,
            "http://ex/n1",
            RDF_TYPE,
            type_ref(&store, NOTE_TYPE),
        )];
        store.transact(&n1, TS, None, None).unwrap();

        // Add a policy governing Note — this must invalidate the cache.
        define_policy(&mut store, "http://ex/P2", NOTE_TYPE, REQUIRE_LABEL);

        // A new non-compliant Note is now denied (registry was rebuilt).
        let n2 = vec![assert_datum(
            &store,
            "http://ex/n2",
            RDF_TYPE,
            type_ref(&store, NOTE_TYPE),
        )];
        let err = store.transact(&n2, TS, None, None);
        assert!(
            matches!(err, Err(Error::PolicyDenied(_))),
            "a newly-added policy must be honored on the next write, got {err:?}"
        );
    }
}
