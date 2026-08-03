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
fn points_for(class: &str) -> Option<&'static [&'static str]> {
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
    reversibility_window: Option<String>,
    on_timeout: Option<String>,
    hosted_at_layer: Option<String>,
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

/// Validate every policy this write defines or amends. `Err(PolicyDenied)` on
/// the first malformed one; the caller rolls the savepoint back.
///
/// Reads the *pending* post-state — the datums are already staged in the open
/// savepoint — so a write that supplies the missing field in the same
/// transaction passes, and one that removes it fails.
pub fn validate_write(store: &Store, datums: &[Datum], graph: i64) -> Result<()> {
    let touched = touched_policies(store, datums, graph)?;
    if touched.is_empty() {
        return Ok(());
    }
    let placements = read_placements(store, &touched)?;
    for iri in &touched {
        let placement = placements.get(iri).cloned().unwrap_or_default();
        if let Some(reason) = placement.violation(iri) {
            return Err(Error::PolicyDenied(reason));
        }
    }
    Ok(())
}

/// The IRIs of entities this write touched that are `aegis:Policy` in the
/// pending post-state.
///
/// Deliberately keyed on the entity being a Policy *after* this write rather
/// than on the attributes written: amending an unrelated field of a malformed
/// policy should still surface the malformation, and adding `rdf:type
/// aegis:Policy` to an existing node is itself a definition.
fn touched_policies(store: &Store, datums: &[Datum], graph: i64) -> Result<Vec<String>> {
    let (Some(rdf_type_id), Some(policy_type_id)) = (
        store.lookup(RDF_TYPE)?,
        store.lookup(&format!("{DEFAULT_BASE_NS}Policy"))?,
    ) else {
        // Neither term interned => no policy can exist yet.
        return Ok(Vec::new());
    };

    let mut entities: Vec<i64> = datums.iter().map(|d| d.entity).collect();
    entities.sort_unstable();
    entities.dedup();

    let mut out = Vec::new();
    for e in entities {
        if is_policy(store, e, rdf_type_id, policy_type_id, graph)? {
            out.push(store.resolve(e)?);
        }
    }
    Ok(out)
}

/// Whether `entity` is an active `aegis:Policy` in its own graph or ROOT.
fn is_policy(
    store: &Store,
    entity: i64,
    rdf_type_id: i64,
    policy_type_id: i64,
    graph: i64,
) -> Result<bool> {
    let mut stmt = store.prepare(
        "SELECT 1 FROM facts \
         WHERE e = ?1 AND a = ?2 AND v = ?3 AND op = 1 AND valid_to IS NULL \
           AND (g = ?4 OR g = 0) LIMIT 1",
    )?;
    let value = Value::Ref(policy_type_id).to_bytes();
    let found = stmt
        .query_row(rusqlite::params![entity, rdf_type_id, value, graph], |_| {
            Ok(true)
        })
        .unwrap_or(false);
    Ok(found)
}

/// Read the SARC metadata of the touched policies in one SPARQL pass.
fn read_placements(store: &Store, touched: &[String]) -> Result<HashMap<String, Placement>> {
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?p ?boundary ?class ?point ?window ?timeout WHERE {{ \
            ?p a a:Policy . \
            OPTIONAL {{ ?p a:boundary ?boundary }} \
            OPTIONAL {{ ?p a:constraintClass ?class }} \
            OPTIONAL {{ ?p a:verificationPoint ?point }} \
            OPTIONAL {{ ?p a:reversibilityWindowSeconds ?window }} \
            OPTIONAL {{ ?p a:onTimeout ?timeout }} \
            OPTIONAL {{ ?p a:hostedAtLayer ?layer }} \
         }}"
    );
    // Collect the DISTINCT values each field resolved to, rather than the last
    // row's value: a multi-valued field produces one row per value, and taking
    // whichever arrived last is how a re-classed policy would slip through.
    let mut fields: HashMap<String, HashMap<&'static str, Vec<String>>> = HashMap::new();
    if let QueryResult::Select { rows, .. } = sparql::query(store, &q)? {
        for row in rows {
            let Some(iri) = iri_of(store, row.get("p")) else {
                continue;
            };
            if !touched.contains(&iri) {
                continue;
            }
            let entry = fields.entry(iri).or_default();
            for (field, binding) in [
                ("boundary", "boundary"),
                ("constraintClass", "class"),
                ("verificationPoint", "point"),
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
        out.insert(
            iri,
            Placement {
                boundary: single("boundary"),
                class: single("constraintClass"),
                point: single("verificationPoint"),
                reversibility_window: single("reversibilityWindowSeconds"),
                on_timeout: single("onTimeout"),
                hosted_at_layer: single("hostedAtLayer"),
                ambiguous,
            },
        );
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
