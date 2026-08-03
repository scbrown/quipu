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
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#> \
         SELECT ?p ?label ?class ?point ?effect ?layer WHERE {{ \
            ?p a a:Policy ; a:boundary \"action\" . \
            OPTIONAL {{ ?p rdfs:label ?label }} \
            OPTIONAL {{ ?p a:constraintClass ?class }} \
            OPTIONAL {{ ?p a:verificationPoint ?point }} \
            OPTIONAL {{ ?p a:effect ?effect }} \
            OPTIONAL {{ ?p a:hostedAtLayer ?layer }} \
         }}"
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &q)? else {
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
