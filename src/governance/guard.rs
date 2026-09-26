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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledPolicy {
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
    /// `aegis:exemplar` — the record that motivated this policy, when it was
    /// drafted from one (docs/design/policy-by-example.md). Carried in the
    /// registry so a refusal can cite its motivating case WITHOUT a per-denial
    /// SELECT: the citation belongs in the refusal message, and the refusal
    /// path must stay as cheap as the allow path.
    exemplar: Option<String>,
}

impl CompiledPolicy {
    /// The policy's IRI.
    pub(crate) fn iri(&self) -> &str {
        &self.policy_iri
    }

    /// Whether the effect blocks at the write gate.
    pub(crate) fn blocks(&self) -> bool {
        effect_blocks(&self.effect)
    }

    /// The exemplar citation appended to this policy's refusals, or `""`.
    ///
    /// A refusal under a drafted rule arrives EXPLAINED BY EXAMPLE — the case
    /// that birthed the rule is named, so the refused party reads why the rule
    /// exists rather than only that it fired. Empty (never a placeholder) for
    /// a hand-authored policy: citing an absent exemplar would be forged
    /// provenance in the message channel.
    fn exemplar_citation(&self) -> String {
        match &self.exemplar {
            Some(iri) => format!(
                " This rule's motivating case: {iri} (aegis:exemplar) — the \
                 refusal is similar to the example that motivated the rule."
            ),
            None => String::new(),
        }
    }
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
        Self::build_at(store, &TemporalContext::default())
    }

    /// [`Self::build`] as the store stood at `at`: the policy set that governed
    /// a historical transaction, read through the same compile path.
    pub(crate) fn build_at(store: &Store, at: &TemporalContext) -> Result<Self> {
        let q = format!(
            "PREFIX a: <{DEFAULT_BASE_NS}> \
             SELECT ?p ?t ?c ?e ?probe ?window ?exemplar WHERE {{ \
                ?p a a:Policy ; a:targets ?t ; a:claim ?c ; a:boundary \"action\" . \
                OPTIONAL {{ ?p a:effect ?e }} \
                OPTIONAL {{ ?p a:evidenceProbe ?probe }} \
                OPTIONAL {{ ?p a:reversibilityWindowSeconds ?window }} \
                OPTIONAL {{ ?p a:exemplar ?exemplar }} \
             }}"
        );
        let mut by_type: HashMap<String, Vec<CompiledPolicy>> = HashMap::new();
        if let QueryResult::Select { rows, .. } = sparql::query_temporal(store, &q, at)? {
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
                let exemplar = str_of(row.get("exemplar"));
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
                        exemplar,
                    });
            }
        }
        Ok(Self { by_type })
    }

    /// The policies targeting `type_iri`, in compile order.
    pub(crate) fn policies_for(&self, type_iri: &str) -> &[CompiledPolicy] {
        self.by_type.get(type_iri).map_or(&[], Vec::as_slice)
    }

    /// Every compiled policy, sorted by (IRI, target type) so two registries
    /// compare by content rather than by hash-map order.
    pub(crate) fn sorted(&self) -> Vec<&CompiledPolicy> {
        let mut all: Vec<&CompiledPolicy> = self.by_type.values().flatten().collect();
        all.sort_by(|a, b| {
            (a.policy_iri.as_str(), a.target_type_iri.as_str())
                .cmp(&(b.policy_iri.as_str(), b.target_type_iri.as_str()))
        });
        all
    }

    /// `self` with `overlay` layered on top: a policy IRI present in `overlay`
    /// replaces every entry of that IRI in `self`.
    pub(crate) fn overlaid(&self, overlay: &Self) -> Self {
        let replaced: std::collections::HashSet<&str> = overlay
            .by_type
            .values()
            .flatten()
            .map(|p| p.policy_iri.as_str())
            .collect();
        let mut by_type: HashMap<String, Vec<CompiledPolicy>> = HashMap::new();
        for (t, ps) in &self.by_type {
            for p in ps
                .iter()
                .filter(|p| !replaced.contains(p.policy_iri.as_str()))
            {
                by_type.entry(t.clone()).or_default().push(p.clone());
            }
        }
        for (t, ps) in &overlay.by_type {
            by_type
                .entry(t.clone())
                .or_default()
                .extend(ps.iter().cloned());
        }
        Self { by_type }
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

        let ctx = EvalCtx::live(store);
        for e in touched {
            let type_iris = entity_type_iris(&ctx, e, rdf_type_id, graph)?;
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
                    evaluate_one(&ctx, &eiri, policy, verdicts, requests)?;
                }
            }
        }
        Ok(())
    }
}

/// The active `rdf:type` IRIs of `entity` in its own graph or ROOT, read from
/// the pending post-state (same connection sees the open savepoint). Under an
/// as-of context the same graph-scope rule applies to the rows live at that
/// transaction (the SPARQL as-of predicate, quipu #83).
pub(crate) fn entity_type_iris(
    ctx: &EvalCtx<'_>,
    entity: i64,
    rdf_type_id: i64,
    graph: i64,
) -> Result<Vec<String>> {
    let store = ctx.store;
    let raw: Vec<Vec<u8>> = match ctx.at.as_of_tx {
        None => {
            let mut stmt = store.prepare(
                "SELECT DISTINCT v FROM facts \
                     WHERE e = ?1 AND a = ?2 AND op = 1 AND valid_to IS NULL AND (g = ?3 OR g = 0)",
            )?;
            stmt.query_map(params![entity, rdf_type_id, graph], |row| row.get(0))?
                .collect::<std::result::Result<_, _>>()?
        }
        Some(tx) => {
            let mut stmt = store.prepare(
                "SELECT DISTINCT v FROM facts \
                     WHERE e = ?1 AND a = ?2 AND op = 1 AND tx <= ?4 \
                     AND (valid_to IS NULL OR retracted_tx > ?4) AND (g = ?3 OR g = 0)",
            )?;
            stmt.query_map(params![entity, rdf_type_id, graph, tx], |row| row.get(0))?
                .collect::<std::result::Result<_, _>>()?
        }
    };
    let mut out = Vec::new();
    for v in raw {
        if let Value::Ref(type_id) = Value::from_bytes(&v)?
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

/// Where and when a policy is judged: the store, the temporal context its
/// ASKs read through, and the clock the router compares expiry against.
///
/// The live gate judges the pending post-state now ([`EvalCtx::live`]). The
/// shadow gate judges history ([`EvalCtx::as_of`]): the post-state of
/// transaction N, and that transaction's own time. ONE evaluator serves both,
/// so the shadow cannot drift from the gate it models (aegis-xfuch4.2).
pub(crate) struct EvalCtx<'a> {
    pub(crate) store: &'a Store,
    pub(crate) at: TemporalContext,
    /// `None` = the wall clock, read at the moment the router needs it.
    pub(crate) now: Option<i64>,
}

impl<'a> EvalCtx<'a> {
    /// The live gate: current (pending) state, wall-clock time.
    pub(crate) fn live(store: &'a Store) -> Self {
        Self {
            store,
            at: TemporalContext::default(),
            now: None,
        }
    }

    /// History: the post-state of transaction `tx`, judged at `now`.
    pub(crate) fn as_of(store: &'a Store, tx: i64, now: i64) -> Self {
        Self {
            store,
            at: TemporalContext {
                as_of_tx: Some(tx),
                ..TemporalContext::default()
            },
            now: Some(now),
        }
    }

    fn now(&self) -> i64 {
        self.now.unwrap_or_else(now_secs)
    }

    fn ask(&self, ask: &str) -> Result<bool> {
        match sparql::query_temporal(self.store, ask, &self.at)? {
            QueryResult::Ask(b) => Ok(b),
            _ => Err(Error::InvalidValue(
                "policy claim/probe must be a SPARQL ASK query".into(),
            )),
        }
    }
}

/// What a policy's claim concluded about one target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClaimOutcome {
    /// A non-blocking effect: the gate runs no ASK and stages no verdict.
    NotEnforced,
    /// The evidence probe found nothing to judge. Never blocks.
    Unknown,
    /// The claim held.
    Satisfied,
    /// The claim failed. Whether the write is refused depends on the effect
    /// and, for an escalating effect, on [`escalation`].
    Unsatisfied,
}

impl ClaimOutcome {
    /// The verdict outcome the gate stages, or `None` for no verdict.
    pub(crate) fn verdict(self) -> Option<&'static str> {
        match self {
            Self::NotEnforced => None,
            Self::Unknown => Some("unknown"),
            Self::Satisfied => Some("satisfied"),
            Self::Unsatisfied => Some("unsatisfied"),
        }
    }
}

/// How an unsatisfied claim resolves at the gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Escalation {
    /// `deny`: refused outright.
    Refused,
    /// An escalating effect with an existing request: the router's ruling.
    Ruled {
        ruling: super::router::Ruling,
        now: i64,
    },
    /// An escalating effect and no request yet: refused, and a request opens.
    Unrequested { now: i64 },
}

impl Escalation {
    /// Whether the write is admitted despite the unsatisfied claim.
    pub(crate) fn admits(&self) -> bool {
        matches!(self, Self::Ruled { ruling, .. } if ruling.permits())
    }
}

/// Judge one policy's claim for one target. The shared core of the live gate,
/// the backtest and the shadow gate: it reads, and never stages or writes.
pub(crate) fn judge_claim(
    ctx: &EvalCtx<'_>,
    entity_iri: &str,
    policy: &CompiledPolicy,
) -> Result<ClaimOutcome> {
    if !effect_blocks(&policy.effect) {
        return Ok(ClaimOutcome::NotEnforced);
    }
    judge_claim_as_enforced(ctx, entity_iri, policy)
}

/// [`judge_claim`] without the effect filter: what the claim concludes as if
/// the policy were enforced. The backtest and the shadow gate ask this of an
/// advisory rule, because "would it fire?" is their question, not "does the
/// gate check it?".
pub(crate) fn judge_claim_as_enforced(
    ctx: &EvalCtx<'_>,
    entity_iri: &str,
    policy: &CompiledPolicy,
) -> Result<ClaimOutcome> {
    guard_iri(entity_iri)?;
    let target = format!("<{entity_iri}>");

    // Evidence probe: if the evidence does not exist yet the outcome is
    // `unknown` (distinct from unsatisfied) and the write is NOT blocked.
    if let Some(probe) = &policy.evidence_probe {
        let bound_probe = probe.replace("$target", &target);
        if !ctx.ask(&bound_probe)? {
            // Recorded as `unknown`, not skipped. "No evidence yet" and "never
            // evaluated" are different facts, and an absent verdict makes the
            // gate look as though the policy did not apply.
            return Ok(ClaimOutcome::Unknown);
        }
    }

    let bound_claim = policy.claim.replace("$target", &target);
    let held = ctx.ask(&bound_claim)?;
    // Test-only evaluator mutation: the shadow gate's sabotage arm proves a
    // wrong evaluator SURFACES against recorded verdicts (aegis-xfuch4.2).
    #[cfg(test)]
    let held = held != sabotage::inverted(&policy.policy_iri);
    if held {
        return Ok(ClaimOutcome::Satisfied);
    }
    Ok(ClaimOutcome::Unsatisfied)
}

/// How an unsatisfied claim under `policy` resolves: refused outright, or
/// routed through the escalation router as it stood in `ctx`.
pub(crate) fn escalation(
    ctx: &EvalCtx<'_>,
    entity_iri: &str,
    policy: &CompiledPolicy,
) -> Result<Escalation> {
    if !effect_escalates(&policy.effect) {
        return Ok(Escalation::Refused);
    }
    let now = ctx.now();
    Ok(
        match super::router::resolve_at(ctx.store, &policy.policy_iri, entity_iri, now, &ctx.at)? {
            Some(ruling) => Escalation::Ruled { ruling, now },
            None => Escalation::Unrequested { now },
        },
    )
}

/// Evaluate a single policy against a single target entity, as the live gate:
/// judge it, stage its verdict and any request, and refuse when it blocks. A
/// non-blocking effect runs no ASK (nothing to enforce at the write gate).
fn evaluate_one(
    ctx: &EvalCtx<'_>,
    entity_iri: &str,
    policy: &CompiledPolicy,
    verdicts: &mut Vec<super::verdict_facts::PendingVerdict>,
    requests: &mut Vec<super::router::PendingRequest>,
) -> Result<()> {
    let outcome = judge_claim(ctx, entity_iri, policy)?;
    if let Some(v) = outcome.verdict() {
        verdicts.push(super::verdict_facts::PendingVerdict {
            predicate_id: policy.policy_iri.clone(),
            target_ref: entity_iri.to_string(),
            outcome: v.to_string(),
        });
    }
    if outcome != ClaimOutcome::Unsatisfied {
        return Ok(());
    }

    // An escalating effect consults the router before refusing. A standing
    // approval bound to this evidence lets the write through — that is the
    // channel `require-approval` never had, and the reason it is no longer a
    // dead end.
    let open_request = |requests: &mut Vec<super::router::PendingRequest>, now: i64| {
        requests.push(super::router::PendingRequest {
            policy_iri: policy.policy_iri.clone(),
            target_iri: entity_iri.to_string(),
            window_secs: policy.reversibility_window.unwrap_or(0),
            now,
        });
    };
    match escalation(ctx, entity_iri, policy)? {
        Escalation::Ruled { ruling, .. } if ruling.permits() => Ok(()),
        // An expired request is a DENIAL of that request, not a permanent
        // dead end for the (policy, target) pair: this attempt re-mints,
        // superseding the expired request with a fresh window a human can
        // still act in. Without this, resolve returns Expired forever and
        // no retry ever reopens the channel (quipu-fu0). A recorded
        // rejection is different — that is an answer, and it stands.
        Escalation::Ruled {
            ruling: super::router::Ruling::Expired,
            now,
        } => {
            open_request(requests, now);
            Err(Error::PolicyDenied(format!(
                "'{entity_iri}' blocked by policy '{}': the previous \
                 DecisionRequest expired with no ruling and was denied \
                 (declared default-deny). This attempt has opened a fresh \
                 request; have an authorized operator record a signed \
                 aegis:Decision with outcome \"approve\" bound to its \
                 evidenceHash, then retry.{}",
                policy.policy_iri,
                policy.exemplar_citation()
            )))
        }
        Escalation::Ruled { ruling, .. } => Err(Error::PolicyDenied(format!(
            "'{entity_iri}' blocked by policy '{}': {}{}",
            policy.policy_iri,
            ruling.reason(&policy.policy_iri, entity_iri),
            policy.exemplar_citation()
        ))),
        // No request yet: this attempt is what opens one. The request itself is
        // staged rather than written here — the gate runs inside the savepoint
        // this refusal is about to roll back, so a request written now would
        // vanish with it. Same ordering problem, same answer, as the verdicts.
        Escalation::Unrequested { now } => {
            open_request(requests, now);
            Err(Error::PolicyDenied(format!(
                "'{entity_iri}' needs a human decision under policy '{}'. A \
                 DecisionRequest has been opened; have a registered decider record \
                 a signed aegis:Decision with outcome \"approve\" bound to its \
                 evidenceHash, then retry.{}",
                policy.policy_iri,
                policy.exemplar_citation()
            )))
        }
        Escalation::Refused => Err(Error::PolicyDenied(format!(
            "'{entity_iri}' blocked by policy '{}' (effect '{}', target type '{}'): claim unsatisfied.{}",
            policy.policy_iri,
            policy.effect,
            policy.target_type_iri,
            policy.exemplar_citation()
        ))),
    }
}

/// Unix seconds, or 0 before the epoch.
fn now_secs() -> i64 {
    // Through the wasm-safe clock shim (quipu-gsg): SystemTime panics on
    // wasm32, and the guard sits on the write path a wasm store still runs.
    i64::try_from(crate::time::epoch_secs()).unwrap_or(i64::MAX)
}

/// Reject an IRI that could break out of an inlined `<...>` and inject SPARQL.
///
/// Crate-visible because the backtest inlines target IRIs into the same claim
/// contract; two copies of an injection filter would eventually differ, and
/// the difference would be an injection.
pub(crate) fn guard_iri(iri: &str) -> Result<()> {
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
/// `aegis:{targets,claim,boundary,effect,evidenceProbe,exemplar,appliesTo}`
/// fact or asserts an `rdf:type aegis:Policy`. When true, the cached [`PolicyRegistry`]
/// is stale. (`exemplar` is in the list because the registry carries it into
/// refusal messages — a citation added after the cache was built must not stay
/// invisible until an unrelated policy write.) Cheap: integer term-id compares
/// over the datums after a handful of interned lookups.
pub fn is_governance_write(store: &Store, datums: &[Datum]) -> Result<bool> {
    let mut pred_ids = Vec::new();
    for p in [
        "targets",
        "claim",
        "boundary",
        "effect",
        "evidenceProbe",
        "exemplar",
        // A tripwire's whole condition is its path scope, so re-scoping one is
        // amending the policy — the cache must not keep serving the old span.
        "appliesTo",
    ] {
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

/// Test-only evaluator mutation (see [`judge_claim_as_enforced`]).
#[cfg(test)]
pub(crate) mod sabotage {
    use std::cell::RefCell;

    thread_local! {
        static INVERT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// Invert the claim result of `policy` on this thread until reset.
    pub(crate) fn invert(policy: Option<&str>) {
        INVERT.with(|s| *s.borrow_mut() = policy.map(str::to_string));
    }

    pub(super) fn inverted(policy: &str) -> bool {
        INVERT.with(|s| s.borrow().as_deref() == Some(policy))
    }
}

#[cfg(test)]
#[path = "guard_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "guard_golden_tests.rs"]
mod golden_tests;
