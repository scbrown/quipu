//! Class↔placement conformance — the cross-field rules SHACL core cannot state.
//!
//! `shapes/governance.ttl` types every SARC constraint field individually
//! (`constraintClass` is one of three strings, `verificationPoint` one of five,
//! and so on). What it cannot express is the part that actually matters:
//! **which class may be enforced at which point**, SARC §4.2 Table 3.
//!
//! ```text
//! hard        PAG, ATM, tool_layer, policy_layer   admissibility decided before
//!                                                  or during dispatch
//! soft        ATM, PAA                             cost-weighted over partial or
//!                                                  completed action data
//! escalation  PAG, PAA                             predictive on the signature,
//!                                                  or reactive on the post-state
//! ```
//!
//! A hard constraint declared at the PAA is the failure this module exists to
//! catch, and it is not a typo an operator would notice: it reads as governed,
//! validates against every shape, and is evaluated only *after* the action it
//! was meant to prevent has already happened. That is the same shape as a CI
//! gate configured to continue-on-error — present, plausible, and incapable of
//! failing. SHACL would need `sh:sparql` or a per-class node shape and an
//! `sh:or` over the cross-product to say it; a hundred lines of Rust says it
//! once, legibly, and can explain itself in the rejection message.
//!
//! Two further rules travel with it, both SARC I2 ("a constraint missing any
//! field is not a constraint; it is a comment"):
//!
//! - an **action-boundary** policy must declare a class and a verification
//!   point — otherwise nothing can place it;
//! - an **escalation** must declare a reversibility window and `onTimeout`,
//!   because an escalation without a bound is not oversight, it is deferred
//!   autonomy (SARC I4, §5.3).
//!
//! The entity-grounded and similarity-tier vocabulary
//! (docs/design/semantic-grounded-edit-policies.md) adds two more families:
//!
//! - a predicate with a **grounded match type** (`must-ground` /
//!   `must-not-ground`) must declare an `aegis:groundingQuery` — without one
//!   there is no id set for candidates to be members OF, and the failure mode
//!   is not an error but silence: an evaluator with no projection sees an
//!   empty set, nothing grounds, and a deny-on-grounding rule allows
//!   everything. Missing grounding must be loud at definition time, never
//!   discovered as an allow at evaluation time;
//! - a policy whose predicate carries tier **`embedding`** or **`model`** is a
//!   classifier, and a classifier does not hard-deny before the action: effect
//!   `deny` at the PAG is refused (the PAA and the escalation router are its
//!   seams), and the policy must carry an `OperatingPoint` with a NONZERO
//!   false-positive tolerance — 0.0/0.0 is the exact tiers' claim, and an
//!   approximate predicate declaring it is asserting an error rate it cannot
//!   have.
//!
//! ## Definition-time, not evaluation-time
//!
//! This runs when a write DEFINES OR AMENDS a policy, against the pending
//! post-state, on the same signal [`super::is_governance_write`] already
//! computes for cache invalidation. Policies already in the graph are not
//! re-validated — turning the check on cannot retroactively break a store, only
//! reject the next malformed definition. That is the same
//! definition-time-verification model `shacl.validate_on_write` uses.
//!
//! Gated by `[quipu.governance] validate_placement`, default **false**. Opt-in,
//! never switched on beneath a running deployment.

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::namespace::{DEFAULT_BASE_NS, RDF_TYPE};
use crate::sparql::{self, QueryResult};
use crate::store::{Datum, Store};
use crate::types::Value;

/// Enforcement points a **hard** constraint may occupy: admissibility has to be
/// decided before or during dispatch, so the PAA — which runs after the action
/// completed — is not among them.
const HARD_POINTS: &[&str] = &["PAG", "ATM", "tool_layer", "policy_layer"];
/// Enforcement points a **soft** constraint may occupy: it attaches cost to
/// partial or completed action data, which the PAG does not have.
const SOFT_POINTS: &[&str] = &["ATM", "PAA"];
/// Enforcement points an **escalation** constraint may occupy: predictive at
/// the PAG (fires on the action signature) or reactive at the PAA (fires on the
/// observed post-state).
const ESCALATION_POINTS: &[&str] = &["PAG", "PAA"];

/// The permitted points for a class, or `None` if the class is unknown (which
/// the shape's `sh:in` has already rejected — this is belt and braces).
///
/// Crate-visible because the audit checker applies the SAME table to what a
/// trace recorded. Two copies of SARC Table 3 would eventually disagree, and
/// the disagreement would be between the definition-time check and the
/// audit-time one — the two places that must not.
pub(crate) fn points_for(class: &str) -> Option<&'static [&'static str]> {
    match class {
        "hard" => Some(HARD_POINTS),
        "soft" => Some(SOFT_POINTS),
        "escalation" => Some(ESCALATION_POINTS),
        _ => None,
    }
}

/// The SARC constraint metadata of one policy, as read back from the pending
/// post-state. Every field is optional here because the shape makes them
/// optional; the rules below decide which absences are fatal.
#[derive(Debug, Default, Clone)]
struct Placement {
    boundary: Option<String>,
    class: Option<String>,
    point: Option<String>,
    effect: Option<String>,
    reversibility_window: Option<String>,
    on_timeout: Option<String>,
    hosted_at_layer: Option<String>,
    /// The tier(s) of the predicate this policy composes, read through
    /// `aegis:predicate`. A Vec, not an Option: a policy mid-re-composition can
    /// see two predicates (or one re-tiered predicate) in the pending state,
    /// and the inexact-tier rules apply if ANY of them is a classifier — the
    /// conservative reading, since the policy would evaluate that predicate.
    predicate_tiers: Vec<String>,
    /// Whether the policy carries an `aegis:operatingPoint` at all.
    has_operating_point: bool,
    /// Every distinct `aegis:falsePositiveTolerance` reachable through the
    /// policy's operating point(s). All must be nonzero for an inexact-tier
    /// predicate: one zero among several is still a claim of exactness.
    fp_tolerances: Vec<f64>,
    /// Fields that resolved to more than one distinct value, as
    /// `(field, values)`. See [`Placement::violation`] — this is checked first.
    ambiguous: Vec<(&'static str, Vec<String>)>,
}

impl Placement {
    /// Check this policy against the placement rules, returning the reason it
    /// is malformed. `None` means conformant.
    ///
    /// A policy that is not at the action boundary is exempt: a
    /// `boundary:"transition"` or boundary-less policy has no dispatch-time
    /// placement to get wrong, and demanding one would reject every
    /// committed-tier policy that legitimately carries only a SPARQL claim.
    fn violation(&self, iri: &str) -> Option<String> {
        // Ambiguity first, and BEFORE the boundary exemption. A policy carrying
        // two constraint classes is not a policy with one of them — asserting
        // `hard` over an existing `soft` leaves both facts active, and a
        // resolver that silently picks one would let a re-class land while
        // reporting the old placement as conformant. Rejecting is the only
        // reading that cannot be wrong.
        if let Some((field, values)) = self.ambiguous.first() {
            let mut sorted = values.clone();
            sorted.sort();
            return Some(format!(
                "policy '{iri}' has {} distinct values for aegis:{field} ({}). A \
                 constraint with two classes or two enforcement points is not a \
                 constraint with one of them — nothing can decide which governs. \
                 Retract the stale value in the same transaction that asserts \
                 the new one.",
                sorted.len(),
                sorted.join(", ")
            ));
        }

        // VALUE checks for the two safety-critical enums, HERE rather than left
        // to the shape.
        //
        // `shapes/governance.ttl` gives `onTimeout` a one-value `sh:in` and
        // `hostedAtLayer` no "prompt" value, and it is tempting to call those
        // settings unrepresentable. They are not. SHACL runs only when
        // `shacl.validate_on_write` is set — it defaults to FALSE, and it
        // validates episode ingest rather than `Store::transact` generally — so
        // a policy written through `/knot` or a direct transact could carry
        // `onTimeout "allow"` and nothing would object. "The shape rejects it"
        // and "the store cannot hold it" are different claims, and shipping the
        // second while only the first is true is how a control comes to be
        // believed in without being invoked.
        //
        // These two are re-checked on the path that is actually on, because
        // both failures are silent and load-bearing: default-allow turns an
        // escalation into a no-op exactly under the load that made it matter
        // (SARC §5.3), and a hard constraint hosted at the prompt layer is what
        // I6 exists to forbid.
        //
        // NB this still cannot make a value unwritable — a raw SQL write, or a
        // process that opens the store directly, bypasses every check quipu has.
        // The honest claim is "refused on quipu's write path", and the vocabulary
        // omission is defence in depth behind it, not a guarantee above it.
        if let Some(timeout) = self.on_timeout.as_deref()
            && timeout != "deny"
        {
            return Some(format!(
                "policy '{iri}' declares aegis:onTimeout \"{timeout}\". The only \
                 permitted value is \"deny\": when no operator services an \
                 escalation inside its reversibility window the action must be \
                 treated as denied. Default-allow under operator unavailability \
                 converts the constraint into a no-op exactly when load makes it \
                 matter, which is the opposite of oversight."
            ));
        }
        if let Some(layer) = self.hosted_at_layer.as_deref()
            && !matches!(layer, "orchestration" | "tool" | "policy")
        {
            return Some(format!(
                "policy '{iri}' declares aegis:hostedAtLayer \"{layer}\". Permitted: \
                 orchestration, tool, policy. There is deliberately no \"prompt\" \
                 layer — a constraint expressed as an instruction the model may \
                 reinterpret or route around is not enforcement, and hosting one \
                 there is what SARC I6 forbids."
            ));
        }

        // ── Inexact-tier predicates (design B/C: "embedding", "model") ──
        //
        // BEFORE the boundary exemption, like the value checks above, because
        // both rules are about honesty rather than placement geometry: a
        // classifier claiming exactness, or hard-denying on a score, is wrong
        // wherever the policy binds. Keyed on the PREDICATE's tier read
        // through `aegis:predicate` — the policy is where effect, point and
        // operating point live, so the policy is what gets refused.
        if let Some(tier) = self
            .predicate_tiers
            .iter()
            .find(|t| matches!(t.as_str(), "embedding" | "model"))
        {
            // A similarity score or a generative judgment trades FP against
            // FN; letting it hard-deny before the action makes a threshold
            // choice into an unappealable refusal. The latency budget agrees:
            // neither an embedding pass nor a model call fits a 5 ms hook.
            if self.effect.as_deref() == Some("deny") && self.point.as_deref() == Some("PAG") {
                return Some(format!(
                    "policy '{iri}' hard-denies at the Pre-Action Gate on a \
                     \"{tier}\"-tier predicate. A classifier's verdict is a \
                     score against a threshold, not a fact, and a hard PAG \
                     denial makes its false positives unappealable before the \
                     action even exists to inspect. Place it at the PAA \
                     (advisory over the completed action) or use an escalating \
                     effect through the router — exact tiers alone may deny at \
                     the gate."
                ));
            }
            if !self.has_operating_point {
                return Some(format!(
                    "policy '{iri}' composes a \"{tier}\"-tier predicate but \
                     declares no aegis:operatingPoint. An approximate predicate \
                     without a declared FP/FN posture is uncalibrated — nobody \
                     decided what error rate this deployment accepts. Declare \
                     an aegis:OperatingPoint with a NONZERO \
                     falsePositiveTolerance."
                ));
            }
            if self.fp_tolerances.is_empty() {
                return Some(format!(
                    "policy '{iri}' composes a \"{tier}\"-tier predicate but \
                     its operating point declares no \
                     aegis:falsePositiveTolerance. A classifier has a false- \
                     positive rate whether or not it is written down; declare \
                     a tolerant, NONZERO number rather than leaving the \
                     trade-off to whoever tuned the threshold."
                ));
            }
            if self.fp_tolerances.contains(&0.0) {
                return Some(format!(
                    "policy '{iri}' composes a \"{tier}\"-tier predicate whose \
                     operating point claims falsePositiveTolerance 0.0. \
                     Exactness belongs to the exact tiers — an exact predicate \
                     carries 0.0/0.0 because it has no classifier error, and a \
                     similarity or model judgment declaring the same number is \
                     asserting an error rate it cannot have. Declare a \
                     tolerant, nonzero falsePositiveTolerance."
                ));
            }
        }

        if self.boundary.as_deref() != Some("action") {
            return None;
        }

        let Some(class) = self.class.as_deref() else {
            return Some(format!(
                "policy '{iri}' is at boundary \"action\" but declares no \
                 aegis:constraintClass. Without a class it cannot be placed at an \
                 enforcement point, and `effect` alone cannot say whether a \
                 violation is inadmissible (hard), costed (soft), or a human's \
                 call (escalation). Declare one of: hard, soft, escalation."
            ));
        };

        let Some(point) = self.point.as_deref() else {
            return Some(format!(
                "policy '{iri}' declares aegis:constraintClass \"{class}\" but no \
                 aegis:verificationPoint. A constraint with no declared point is \
                 not enforced anywhere in particular — say where it runs. \
                 Permitted for \"{class}\": {}.",
                permitted_list(class)
            ));
        };

        let Some(permitted) = points_for(class) else {
            return Some(format!(
                "policy '{iri}' declares an unknown aegis:constraintClass \
                 \"{class}\" (expected hard, soft or escalation)"
            ));
        };

        // Class↔effect conformance. The write gate escalates on the EFFECT
        // (`require-approval`/`escalate`), while the window/onTimeout rules
        // below key on the CLASS — so a policy with an escalating effect but
        // class hard/soft would escalate at runtime with no declared window,
        // get a zero window from the router, and become an instant permanent
        // denial. Refuse the mismatch here, where the author can still fix it.
        if let Some(effect) = self.effect.as_deref()
            && matches!(effect, "require-approval" | "escalate")
            && class != "escalation"
        {
            return Some(format!(
                "policy '{iri}' declares aegis:effect \"{effect}\", which routes \
                 to a human at runtime, but aegis:constraintClass \"{class}\". \
                 An escalating effect needs the escalation class's declared \
                 bounds (aegis:reversibilityWindowSeconds, aegis:onTimeout) — \
                 without them the escalation has no time at which the absence \
                 of a ruling becomes an answer. Declare constraintClass \
                 \"escalation\" with those fields, or use a non-escalating \
                 effect."
            ));
        }

        if !permitted.contains(&point) {
            return Some(format!(
                "policy '{iri}' declares aegis:constraintClass \"{class}\" at \
                 aegis:verificationPoint \"{point}\", which cannot enforce it. {} \
                 Permitted for \"{class}\": {}.",
                why_not(class, point),
                permitted.join(", ")
            ));
        }

        if class == "escalation" {
            if self.reversibility_window.is_none() {
                return Some(format!(
                    "escalation policy '{iri}' declares no \
                     aegis:reversibilityWindowSeconds. An escalation without a \
                     bound is not human oversight; it is deferred autonomy — \
                     there is no time by which the absence of a ruling becomes an \
                     answer. Declare the window within which the action can still \
                     be undone."
                ));
            }
            if self.on_timeout.is_none() {
                return Some(format!(
                    "escalation policy '{iri}' declares no aegis:onTimeout. \
                     Declare `onTimeout \"deny\"`: when no operator services the \
                     escalation inside the reversibility window the action must \
                     be treated as denied, or the constraint becomes a no-op \
                     exactly under the load that made it matter."
                ));
            }
        }

        None
    }
}

/// The permitted points for a class, rendered for a message. Falls back to the
/// class name itself when unknown, so the message degrades rather than panics.
fn permitted_list(class: &str) -> String {
    points_for(class).map_or_else(|| "(unknown class)".to_string(), |p| p.join(", "))
}

/// The one-sentence reason a class cannot live at a point. Generic advice is
/// what makes a rejection un-actionable, so each combination that actually
/// occurs gets its own explanation.
fn why_not(class: &str, point: &str) -> &'static str {
    match (class, point) {
        ("hard", "PAA") => {
            "The Post-Action Auditor runs after the action has completed, so it \
             cannot prevent it — a hard constraint evaluated there documents the \
             violation it was meant to stop."
        }
        ("soft", "PAG") => {
            "The Pre-Action Gate sees only the proposed action, not what it cost; \
             a soft constraint evaluated there has no completed-action data to \
             weigh, and blocking on a prediction is what makes `warn` rules \
             read as `deny` ones."
        }
        ("soft", "tool_layer" | "policy_layer") => {
            "Tool- and policy-layer enforcement refuses outright; a soft \
             constraint has no refusal to make."
        }
        ("escalation", "ATM") => {
            "The Action-Time Monitor runs mid-flight, with no seam at which to \
             suspend and await a human ruling."
        }
        ("escalation", "tool_layer" | "policy_layer") => {
            "Tool- and policy-layer enforcement has no channel to route a \
             decision to an operator, so an escalation there can only fail closed."
        }
        _ => "",
    }
}

/// The grounding-relevant fields of one Predicate, as read back from the
/// pending post-state. The rule is a pure function over this, mirroring
/// [`Placement`], so the table in `placement_tests.rs` can exercise it without
/// a store.
#[derive(Debug, Default, Clone)]
struct Grounding {
    /// Every distinct `aegis:matchType` on the predicate. A Vec for the same
    /// reason as [`Placement::predicate_tiers`]: a predicate mid-amendment can
    /// carry two, and the grounding requirement applies if ANY is grounded.
    match_types: Vec<String>,
    /// Whether the predicate declares an `aegis:groundingQuery`.
    has_grounding_query: bool,
}

impl Grounding {
    /// Check this predicate against the grounding rule, returning the reason
    /// it is malformed. `None` means conformant.
    fn violation(&self, iri: &str) -> Option<String> {
        let grounded = self
            .match_types
            .iter()
            .find(|m| matches!(m.as_str(), "must-ground" | "must-not-ground"))?;
        if self.has_grounding_query {
            return None;
        }
        // The failure this refusal pre-empts is not an evaluation error — it
        // is an evaluation SUCCESS with the wrong answer. A grounded match
        // type with no query gives the projector nothing to project; the
        // evaluator then sees an empty membership set, no candidate grounds,
        // and a must-not-ground deny rule allows everything, silently. The
        // design's contract is "missing projection => unevaluated, loud"; a
        // predicate that cannot even NAME its projection has to be refused
        // before it exists to mis-evaluate.
        Some(format!(
            "predicate '{iri}' declares aegis:matchType \"{grounded}\" but no \
             aegis:groundingQuery. A grounded match type tests candidates for \
             membership in an id set, and without a query there is no set to \
             be a member of — the rule would evaluate against emptiness, where \
             nothing grounds and a deny-on-grounding policy allows everything. \
             Declare aegis:groundingQuery (a SPARQL SELECT naming the \
             authoritative id set), or use a lexical matchType."
        ))
    }
}

/// Validate every policy AND predicate this write defines or amends.
/// `Err(PolicyDenied)` on the first malformed one; the caller rolls the
/// savepoint back.
///
/// Reads the *pending* post-state — the datums are already staged in the open
/// savepoint — so a write that supplies the missing field in the same
/// transaction passes, and one that removes it fails.
pub fn validate_write(store: &Store, datums: &[Datum], graph: i64) -> Result<()> {
    let policies = touched_of_type(store, datums, graph, "Policy")?;
    if !policies.is_empty() {
        let placements = read_placements(store, &policies)?;
        for iri in &policies {
            let placement = placements.get(iri).cloned().unwrap_or_default();
            if let Some(reason) = placement.violation(iri) {
                return Err(Error::PolicyDenied(reason));
            }
        }
    }
    // Predicates are validated as entities in their own right, not only
    // through the policies composing them: a grounded predicate with no query
    // is malformed before any policy names it, and catching it at ITS
    // definition is what keeps the refusal adjacent to the author's mistake.
    let predicates = touched_of_type(store, datums, graph, "Predicate")?;
    if !predicates.is_empty() {
        let groundings = read_groundings(store, &predicates)?;
        for iri in &predicates {
            let grounding = groundings.get(iri).cloned().unwrap_or_default();
            if let Some(reason) = grounding.violation(iri) {
                return Err(Error::PolicyDenied(reason));
            }
        }
    }
    Ok(())
}

/// The IRIs of entities this write touched that are `aegis:<class>` in the
/// pending post-state.
///
/// Deliberately keyed on the entity's type *after* this write rather than on
/// the attributes written: amending an unrelated field of a malformed
/// definition should still surface the malformation, and adding the rdf:type
/// to an existing node is itself a definition.
fn touched_of_type(
    store: &Store,
    datums: &[Datum],
    graph: i64,
    class: &str,
) -> Result<Vec<String>> {
    let (Some(rdf_type_id), Some(class_type_id)) = (
        store.lookup(RDF_TYPE)?,
        store.lookup(&format!("{DEFAULT_BASE_NS}{class}"))?,
    ) else {
        // Either term un-interned => no instance can exist yet.
        return Ok(Vec::new());
    };

    let mut entities: Vec<i64> = datums.iter().map(|d| d.entity).collect();
    entities.sort_unstable();
    entities.dedup();

    let mut out = Vec::new();
    for e in entities {
        if is_typed(store, e, rdf_type_id, class_type_id, graph)? {
            out.push(store.resolve(e)?);
        }
    }
    Ok(out)
}

/// Whether `entity` actively carries the given rdf:type in its own graph or
/// ROOT.
fn is_typed(
    store: &Store,
    entity: i64,
    rdf_type_id: i64,
    class_type_id: i64,
    graph: i64,
) -> Result<bool> {
    let mut stmt = store.prepare(
        "SELECT 1 FROM facts \
         WHERE e = ?1 AND a = ?2 AND v = ?3 AND op = 1 AND valid_to IS NULL \
           AND (g = ?4 OR g = 0) LIMIT 1",
    )?;
    let value = Value::Ref(class_type_id).to_bytes();
    let found = stmt
        .query_row(rusqlite::params![entity, rdf_type_id, value, graph], |_| {
            Ok(true)
        })
        .unwrap_or(false);
    Ok(found)
}

/// Read the SARC metadata of the touched policies in one SPARQL pass.
fn read_placements(store: &Store, touched: &[String]) -> Result<HashMap<String, Placement>> {
    // Every OPTIONAL-bound variable must ALSO be in the SELECT projection:
    // `Project` strips non-projected variables, so a binding left out here is
    // silently `None` at `row.get` — which is how the hostedAtLayer refusal
    // was dead code (quipu-sio).
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?p ?boundary ?class ?point ?effect ?window ?timeout ?layer \
                ?ptier ?op ?fp WHERE {{ \
            ?p a a:Policy . \
            OPTIONAL {{ ?p a:boundary ?boundary }} \
            OPTIONAL {{ ?p a:constraintClass ?class }} \
            OPTIONAL {{ ?p a:verificationPoint ?point }} \
            OPTIONAL {{ ?p a:effect ?effect }} \
            OPTIONAL {{ ?p a:reversibilityWindowSeconds ?window }} \
            OPTIONAL {{ ?p a:onTimeout ?timeout }} \
            OPTIONAL {{ ?p a:hostedAtLayer ?layer }} \
            OPTIONAL {{ ?p a:predicate ?pred . ?pred a:tier ?ptier }} \
            OPTIONAL {{ ?p a:operatingPoint ?op }} \
            OPTIONAL {{ ?p a:operatingPoint ?opfp . \
                        ?opfp a:falsePositiveTolerance ?fp }} \
         }}"
    );
    // Collect the DISTINCT values each field resolved to, rather than the last
    // row's value: a multi-valued field produces one row per value, and taking
    // whichever arrived last is how a re-classed policy would slip through.
    //
    // The policy's OWN seven fields feed the ambiguity refusal; the DERIVED
    // bindings (?ptier, ?op, ?fp — read through the predicate and operating
    // point) are kept apart, because two values there are not an amendment
    // race on the policy but a fact about what it composes, and the inexact-
    // tier rules already treat every collected value conservatively.
    let mut fields: HashMap<String, HashMap<&'static str, Vec<String>>> = HashMap::new();
    let mut derived: HashMap<String, HashMap<&'static str, Vec<String>>> = HashMap::new();
    if let QueryResult::Select { rows, .. } = sparql::query(store, &q)? {
        for row in rows {
            let Some(iri) = iri_of(store, row.get("p")) else {
                continue;
            };
            if !touched.contains(&iri) {
                continue;
            }
            let entry = fields.entry(iri.clone()).or_default();
            for (field, binding) in [
                ("boundary", "boundary"),
                ("constraintClass", "class"),
                ("verificationPoint", "point"),
                ("effect", "effect"),
                ("reversibilityWindowSeconds", "window"),
                ("onTimeout", "timeout"),
                ("hostedAtLayer", "layer"),
            ] {
                if let Some(v) = lexical(row.get(binding)) {
                    let seen = entry.entry(field).or_default();
                    if !seen.contains(&v) {
                        seen.push(v);
                    }
                }
            }
            let entry = derived.entry(iri).or_default();
            for (field, value) in [
                ("predicateTier", lexical(row.get("ptier"))),
                ("operatingPoint", iri_of(store, row.get("op"))),
                ("falsePositiveTolerance", numeric_lexical(row.get("fp"))),
            ] {
                if let Some(v) = value {
                    let seen = entry.entry(field).or_default();
                    if !seen.contains(&v) {
                        seen.push(v);
                    }
                }
            }
        }
    }

    let mut out: HashMap<String, Placement> = HashMap::new();
    for (iri, entry) in fields {
        let single = |field: &str| -> Option<String> {
            entry
                .get(field)
                .filter(|vs| vs.len() == 1)
                .map(|vs| vs[0].clone())
        };
        let ambiguous: Vec<(&'static str, Vec<String>)> = entry
            .iter()
            .filter(|(_, vs)| vs.len() > 1)
            .map(|(f, vs)| (*f, vs.clone()))
            .collect();
        let composed = derived.remove(&iri).unwrap_or_default();
        out.insert(
            iri,
            Placement {
                boundary: single("boundary"),
                class: single("constraintClass"),
                point: single("verificationPoint"),
                effect: single("effect"),
                reversibility_window: single("reversibilityWindowSeconds"),
                on_timeout: single("onTimeout"),
                hosted_at_layer: single("hostedAtLayer"),
                predicate_tiers: composed.get("predicateTier").cloned().unwrap_or_default(),
                has_operating_point: composed.contains_key("operatingPoint"),
                fp_tolerances: composed
                    .get("falsePositiveTolerance")
                    .map(|vs| vs.iter().filter_map(|v| v.parse().ok()).collect())
                    .unwrap_or_default(),
                ambiguous,
            },
        );
    }
    Ok(out)
}

/// Read the grounding metadata of the touched predicates in one SPARQL pass,
/// over the same pending post-state as [`read_placements`].
fn read_groundings(store: &Store, touched: &[String]) -> Result<HashMap<String, Grounding>> {
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?pr ?match ?query WHERE {{ \
            ?pr a a:Predicate . \
            OPTIONAL {{ ?pr a:matchType ?match }} \
            OPTIONAL {{ ?pr a:groundingQuery ?query }} \
         }}"
    );
    let mut out: HashMap<String, Grounding> = HashMap::new();
    if let QueryResult::Select { rows, .. } = sparql::query(store, &q)? {
        for row in rows {
            let Some(iri) = iri_of(store, row.get("pr")) else {
                continue;
            };
            if !touched.contains(&iri) {
                continue;
            }
            let entry = out.entry(iri).or_default();
            if let Some(m) = lexical(row.get("match"))
                && !entry.match_types.contains(&m)
            {
                entry.match_types.push(m);
            }
            if lexical(row.get("query")).is_some() {
                entry.has_grounding_query = true;
            }
        }
    }
    Ok(out)
}

/// The lexical form of a bound value, for the string-valued metadata fields.
/// An integer window is accepted as its decimal form — presence is what the
/// rule tests, and the shape has already typed it.
fn lexical(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(Value::Int(i)) => Some(i.to_string()),
        _ => None,
    }
}

/// The lexical form of a bound NUMERIC value. Wider than [`lexical`] because
/// the tolerances arrive differently by route: a Turtle ingest keeps
/// `xsd:decimal` as a typed literal ([`Value::Typed`], exactness preserved),
/// while a direct transact writes [`Value::Float`]. Both are the same declared
/// tolerance, and the rule must read whichever the author's path produced.
fn numeric_lexical(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(Value::Int(i)) => Some(i.to_string()),
        Some(Value::Float(f)) => Some(f.to_string()),
        Some(Value::Typed { lexical, .. }) => Some(lexical.clone()),
        _ => None,
    }
}

fn iri_of(store: &Store, v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Ref(id)) => store.resolve(*id).ok(),
        Some(Value::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
