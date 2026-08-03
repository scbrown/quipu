//! Authority intersection over named graphs — SARC I5.
//!
//! ## What was here before, and what was actually missing
//!
//! Quipu's named graphs are a real isolation substrate, not a provenance label:
//! `graphs` is a registry with an enforced `committed|overlay` class and a
//! bind-once `parent_branch`, and writes, retractions and idempotency are all
//! graph-scoped in SQL (`store/overlays.rs`, `store/ops.rs`). What was missing
//! is not partitioning — it is **authorization**. `http_auth::authorize` is one
//! global bearer token, all-or-nothing, and nothing checked whether a principal
//! may write to graph *N*.
//!
//! That distinction matters because it is the difference between "multi-tenancy
//! is unbuilt" and "multi-tenancy has no access-control layer". The second is
//! true, much smaller, and is what this module fixes.
//!
//! ## The rule: authority narrows, never widens
//!
//! SARC §9.3: an action at depth *k* executes under the **intersection** of the
//! authorities along its call chain,
//!
//! ```text
//! auth_k = ⋂ authority(p_i) for i in 0..=k
//! ```
//!
//! so adding a delegate can only narrow what is permitted. That is the defence
//! against **authority escalation via tool capability** (§9.5): a sub-agent
//! whose own credentials are broader than its caller's cannot use them, because
//! the effective authority is the intersection and not the executor's own.
//!
//! **An empty intersection fails safe.** A chain that has narrowed to nothing
//! cannot act, and that is a refusal rather than a fallback to the principal's
//! authority — the fallback would be precisely the escalation the rule exists to
//! stop.
//!
//! ## The wildcard, and why it is not a hole
//!
//! A principal may hold `*`, meaning every graph. It is how a single-tenant
//! deployment keeps working unchanged, and it composes correctly: `*`
//! intersected with a narrow authority is the narrow one, so a wildcard-holding
//! orchestrator delegating to a scoped worker yields the worker's scope, not the
//! orchestrator's. The wildcard widens nothing under intersection; it only
//! declines to narrow.

use std::collections::BTreeSet;

use crate::error::Result;
use crate::namespace::DEFAULT_BASE_NS;
use crate::sparql::{self, QueryResult};
use crate::store::Store;
use crate::types::Value;

/// The graph-set token meaning "every graph".
pub const ANY_GRAPH: &str = "*";

/// What one principal — or a whole call chain — is permitted to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authority {
    /// Graph IRIs this authority covers, or a single [`ANY_GRAPH`].
    graphs: BTreeSet<String>,
}

impl Authority {
    /// An authority over exactly these graphs.
    #[must_use]
    pub fn over<I, S>(graphs: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            graphs: graphs.into_iter().map(Into::into).collect(),
        }
    }

    /// The unconstrained authority — every graph.
    #[must_use]
    pub fn any() -> Self {
        Self::over([ANY_GRAPH])
    }

    /// The empty authority. Permits nothing; this is the fail-safe state.
    #[must_use]
    pub fn none() -> Self {
        Self {
            graphs: BTreeSet::new(),
        }
    }

    /// Whether this authority is the wildcard.
    #[must_use]
    pub fn is_any(&self) -> bool {
        self.graphs.len() == 1 && self.graphs.contains(ANY_GRAPH)
    }

    /// Whether it permits nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.graphs.is_empty()
    }

    /// Whether this authority covers `graph_iri`.
    #[must_use]
    pub fn permits(&self, graph_iri: &str) -> bool {
        self.is_any() || self.graphs.contains(graph_iri)
    }

    /// The graphs, for diagnostics. Sorted, so a refusal message is stable.
    #[must_use]
    pub fn graphs(&self) -> Vec<&str> {
        self.graphs.iter().map(String::as_str).collect()
    }

    /// Intersect with `other`. Monotonically non-increasing: the result permits
    /// no more than either input.
    ///
    /// The wildcard is the identity, which is what lets a single-tenant
    /// deployment (everyone holds `*`) behave exactly as it did before this
    /// module existed.
    #[must_use]
    pub fn intersect(&self, other: &Self) -> Self {
        if self.is_any() {
            return other.clone();
        }
        if other.is_any() {
            return self.clone();
        }
        Self {
            graphs: self.graphs.intersection(&other.graphs).cloned().collect(),
        }
    }
}

/// The intersected authority of a principal-and-agent chain.
///
/// An EMPTY chain yields [`Authority::none`], not [`Authority::any`]. That is
/// the whole difference between a fail-safe and a fail-open: "nobody said who is
/// acting" must not mean "anybody may act", and the caller decides whether an
/// unattributed write is permitted rather than inheriting a silent yes.
#[must_use]
pub fn intersect_chain(authorities: &[Authority]) -> Authority {
    let Some((first, rest)) = authorities.split_first() else {
        return Authority::none();
    };
    rest.iter()
        .fold(first.clone(), |acc, next| acc.intersect(next))
}

/// Read a principal's declared authority from the graph.
///
/// A principal with no `aegis:authorityOver` facts holds [`Authority::none`] —
/// **not** the wildcard. An undeclared authority is an absent grant, and reading
/// absence as permission is how an access-control layer becomes decorative.
pub fn authority_of(store: &Store, principal: &str) -> Result<Authority> {
    let q = format!(
        "PREFIX a: <{DEFAULT_BASE_NS}> \
         SELECT ?g WHERE {{ ?p a a:Principal ; a:principalId \"{id}\" ; a:authorityOver ?g }}",
        id = principal.replace('\\', "\\\\").replace('"', "\\\""),
    );
    let QueryResult::Select { rows, .. } = sparql::query(store, &q)? else {
        return Ok(Authority::none());
    };
    let graphs: Vec<String> = rows
        .iter()
        .filter_map(|r| match r.get("g") {
            Some(Value::Str(s)) => Some(s.clone()),
            Some(Value::Ref(id)) => store.resolve(*id).ok(),
            _ => None,
        })
        .collect();
    if graphs.is_empty() {
        return Ok(Authority::none());
    }
    Ok(Authority::over(graphs))
}

/// The effective authority of a whole chain, read from the graph.
pub fn chain_authority(store: &Store, chain: &[String]) -> Result<Authority> {
    let mut authorities = Vec::with_capacity(chain.len());
    for principal in chain {
        authorities.push(authority_of(store, principal)?);
    }
    Ok(intersect_chain(&authorities))
}

/// The refusal message for a write the chain's authority does not cover.
///
/// Names the chain, the graph, and what the chain actually holds — a refusal
/// that says only "denied" leaves an operator guessing which link narrowed it.
#[must_use]
pub fn refusal(chain: &[String], graph_iri: &str, authority: &Authority) -> String {
    let held = if authority.is_empty() {
        "nothing (the intersection along the chain is empty)".to_string()
    } else {
        authority.graphs().join(", ")
    };
    format!(
        "principal chain [{}] may not write to graph '{graph_iri}'. Authority \
         along a chain is the INTERSECTION of every link's, so a delegate can \
         only narrow it — this chain holds: {held}. Widen the narrowest link's \
         aegis:authorityOver, or dispatch through a chain that retains the \
         graph.",
        chain.join(" → ")
    )
}

#[cfg(test)]
#[path = "authority_tests.rs"]
mod tests;
