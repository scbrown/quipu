//! Σ — the constraint specification, read back from the graph.
//!
//! The audit checker compares a trace against *what the store says the policies
//! are*, not against a copy of them. That is the point: a checker that read its
//! own snapshot of Σ would agree with itself about a policy that had since been
//! re-classed, which is the drift the check exists to catch.

use std::collections::BTreeMap;

use crate::error::Result;
use crate::namespace::DEFAULT_BASE_NS;
use crate::sparql::{self, QueryResult};
use crate::store::Store;
use crate::types::Value;

/// One constraint in Σ, in the four fields the audit passes need.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Constraint {
    /// The policy's IRI.
    pub iri: String,
    /// The id a trace record cites — `rdfs:label`, else the IRI's local name.
    pub id: String,
    /// `hard` | `soft` | `escalation`, when declared.
    pub class: Option<String>,
    /// The declared enforcement point.
    pub point: Option<String>,
    /// The declared response.
    pub effect: Option<String>,
    /// The layer the policy CLAIMS it is enforced at (SARC I6). A claim — the
    /// trace records the layer that actually evaluated it, and the placement
    /// pass compares the two.
    pub hosted_at_layer: Option<String>,
    /// Whether this constraint binds the whole subtree under a dispatch.
    pub inherited_by_delegates: Option<bool>,
}

/// The constraint an evaluation cites, keyed by the id the trace carries.
pub type Spec = BTreeMap<String, Constraint>;

/// Read Σ from `store`.
///
/// Action-boundary policies only: a `boundary "transition"` policy governs a
/// state change rather than a dispatched action, so it has no enforcement point
/// a trace record could have traversed, and holding one against a trace would
/// report an absence that is correct.
pub fn load(store: &Store) -> Result<Spec> {
    load_as_of(store, None)
}

/// Read Σ as it stood at `as_of` (quipu #72).
///
/// `None` reads the LIVE Σ, which stays the default and keeps the rationale at
/// the top of this file: a checker holding its own snapshot would agree with
/// itself about a policy that had since been re-classed, which is the drift the
/// check exists to catch.
///
/// What live-Σ alone cannot do is tell "the runtime got it wrong" apart from
/// "the spec moved" — both surface as a trace violation. Reading Σ as of the
/// trace's own window is what separates them (see
/// [`crate::governance::replay::replay_as_of`]).
///
/// The policy facts are already bitemporal and the SPARQL layer already
/// supports `AsOf` end to end; this only threads it through, which is why the
/// gap was invisible.
///
/// # Errors
/// Store and SPARQL errors.
pub fn load_as_of(store: &Store, as_of: Option<&crate::store::AsOf>) -> Result<Spec> {
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
         SELECT ?p ?label ?class ?point ?effect ?layer ?inherited WHERE {{ \
            ?p a a:Policy ; a:boundary \"action\" . \
            OPTIONAL {{ ?p rdfs:label ?label }} \
            OPTIONAL {{ ?p a:constraintClass ?class }} \
            OPTIONAL {{ ?p a:verificationPoint ?point }} \
            OPTIONAL {{ ?p a:effect ?effect }} \
            OPTIONAL {{ ?p a:hostedAtLayer ?layer }} \
            OPTIONAL {{ ?p a:inheritedByDelegates ?inherited }} \
         }}"
    );
    let ctx = crate::sparql::TemporalContext {
        valid_at: as_of.and_then(|a| a.valid_at.clone()),
        as_of_tx: as_of.and_then(|a| a.tx),
        ..Default::default()
    };
    let QueryResult::Select { rows, .. } = sparql::query_temporal(store, &q, &ctx)? else {
        return Ok(Spec::new());
    };

    let mut spec = Spec::new();
    for row in &rows {
        let Some(iri) = text(store, row.get("p")) else {
            continue;
        };
        let id = text(store, row.get("label")).unwrap_or_else(|| local_name(&iri));
        // A policy row can repeat across the OPTIONAL cross-product. Merging
        // rather than replacing means a row that happened to bind fewer
        // OPTIONALs cannot erase a field an earlier row supplied — the same
        // one-policy-is-one-constraint collapse hank's projection decode does.
        let entry = spec.entry(id.clone()).or_insert_with(|| Constraint {
            iri: iri.clone(),
            id,
            ..Constraint::default()
        });
        merge(&mut entry.class, text(store, row.get("class")));
        merge(&mut entry.point, text(store, row.get("point")));
        merge(&mut entry.effect, text(store, row.get("effect")));
        merge(&mut entry.hosted_at_layer, text(store, row.get("layer")));
        if entry.inherited_by_delegates.is_none() {
            entry.inherited_by_delegates = boolean(store, row.get("inherited"));
        }
    }
    Ok(spec)
}

/// Fill `slot` if it is empty. Never overwrites: see [`load`].
fn merge(slot: &mut Option<String>, value: Option<String>) {
    if slot.is_none() {
        *slot = value;
    }
}

/// The IRI's local name — what follows the last `#`, `/` or `:`.
fn local_name(iri: &str) -> String {
    iri.rsplit(['#', '/', ':'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(iri)
        .to_string()
}

/// A boolean, however the store holds it. Anything unrecognised is `None` —
/// "not declared" and "declared false" are different claims.
fn boolean(store: &Store, value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Int(i)) => Some(*i != 0),
        Some(Value::Str(s)) => parse_bool(s),
        Some(Value::Ref(id)) => store.resolve(*id).ok().and_then(|s| parse_bool(&s)),
        _ => None,
    }
}

fn parse_bool(text: &str) -> Option<bool> {
    match text {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn text(store: &Store, value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(s)) => Some(s.clone()),
        Some(Value::Ref(id)) => store.resolve(*id).ok(),
        _ => None,
    }
}

#[cfg(test)]
#[path = "audit_spec_tests.rs"]
mod tests;
