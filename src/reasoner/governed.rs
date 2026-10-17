//! Historical selection and evaluation over one admitted immutable raw source.
//!
//! Catalogue metadata is selected by transaction visibility. Edition legal
//! applicability is a separate interval selection; premise facts use both
//! clocks. No mutable current shapes or current inferred companion is loaded.

use std::collections::BTreeSet;

use crate::sparql::{GraphScope, QueryResult, TemporalContext, query_temporal};

use super::{
    editions::{SelectedEdition, SelectionError, select_from_catalogue_snapshot},
    historical::{HistoricalError, HistoricalFact, evaluate_temporal_view},
    history_attestation::HistoryAttestation,
    raw_history::VerifiedRawHistorySource,
};

/// An isolated model answer with its exact admitted source and edition identity.
///
/// This is not a legal judgment or a captured derivation proof. The latter
/// requires explicit support evidence, distinct from later reconstruction.
#[derive(Debug, serde::Serialize)]
pub struct GovernedHistoricalAnswer {
    /// Exact selected executable edition, digest and applicability interval.
    pub selected_edition: SelectedEdition,
    /// Real retained source-local transaction cut.
    pub as_of_tx: i64,
    /// Exact signed source and externally reviewed coverage terms.
    pub source: HistoryAttestation,
    /// Original source admission time, UTC Unix seconds.
    pub admitted_at: i64,
    /// Sorted canonical IRI tuples from isolated evaluation.
    pub facts: Vec<HistoricalFact>,
}

/// Typed refusals, never latest-file fallback or a successful unknown zero.
#[derive(Debug, thiserror::Error)]
pub enum GovernedHistoricalError {
    /// A source/profile output ceiling was reached, or too many rules.
    #[error("governed historical evaluation capacity exceeded")]
    Capacity,
    /// Rule IDs must be unique within one executable edition.
    #[error("duplicate rule identifiers in governed edition")]
    DuplicateRuleId,
    /// Catalogue selection refused.
    #[error(transparent)]
    Selection(#[from] SelectionError),
    /// Premise evaluation refused.
    #[error(transparent)]
    Historical(#[from] HistoricalError),
    /// Query, dictionary or read policy failure.
    #[error(transparent)]
    Store(#[from] crate::Error),
    /// Invalid executable source.
    #[error(transparent)]
    Reasoner(#[from] super::ReasonerError),
}

/// Evaluate one catalogue package in the exact request admitted for this source.
pub fn evaluate_governed_history(
    source: &VerifiedRawHistorySource,
    ruleset: &str,
) -> Result<GovernedHistoricalAnswer, GovernedHistoricalError> {
    use GovernedHistoricalError as E;
    let store = source.store();
    let coverage = &source.terms().coverage;
    let graph = |iri: &str| -> Result<_, E> {
        if iri == crate::schema::ROOT_GRAPH_IRI {
            return Ok(crate::schema::ROOT_GRAPH);
        }
        store.registered_graph_id(iri)?.ok_or_else(|| {
            crate::Error::Store("admitted graph registration disappeared".into()).into()
        })
    };
    let request = source.request();
    let at = crate::time::format_iso_signed(request.valid_at);
    let cap = store.search_config().max_sparql_rows;
    let sentinel = if cap == 0 { 4097 } else { cap.min(4097) };
    let context = TemporalContext {
        as_of_tx: Some(request.as_of_tx),
        graph: GraphScope::Named(vec![graph(&coverage.catalogue_graph)?]),
        ..TemporalContext::default()
    };
    let query = format!("SELECT ?s ?p ?o WHERE {{ ?s ?p ?o }} LIMIT {sentinel}");
    let QueryResult::Select { rows, .. } = query_temporal(store, &query, &context)? else {
        unreachable!("SELECT returns binding rows")
    };
    if rows.len() >= sentinel {
        return Err(E::Capacity);
    }
    let mut catalogue = String::new();
    for row in rows {
        let (Some(crate::Value::Ref(s)), Some(crate::Value::Ref(p)), Some(o)) =
            (row.get("s"), row.get("p"), row.get("o"))
        else {
            return Err(SelectionError::MalformedCatalogue(
                "non-IRI catalogue subject or predicate".into(),
            )
            .into());
        };
        let s = oxrdf::NamedNode::new(store.resolve(*s)?)
            .map_err(|e| SelectionError::MalformedCatalogue(e.to_string()))?;
        let p = oxrdf::NamedNode::new(store.resolve(*p)?)
            .map_err(|e| SelectionError::MalformedCatalogue(e.to_string()))?;
        let o = crate::rdf::value_to_term(store, o)?;
        catalogue.push_str(&format!("{s} {p} {o} .\n"));
        if catalogue.len() > 1024 * 1024 {
            return Err(E::Capacity);
        }
    }
    let mut selected = select_from_catalogue_snapshot(&catalogue, ruleset, &at)?;
    selected.primary_sources.sort();
    let rules = super::parse_rules(&selected.turtle, None)?;
    if rules.rules.len() > 64 {
        return Err(E::Capacity);
    }
    let unique: BTreeSet<_> = rules.rules.iter().map(|rule| &rule.id).collect();
    if unique.len() != rules.rules.len() {
        return Err(E::DuplicateRuleId);
    }
    let context = TemporalContext {
        valid_at: Some(at),
        graph: GraphScope::Named(vec![graph(&coverage.premise_graph)?]),
        ..context
    };
    let facts = evaluate_temporal_view(store, &rules, &context)?;
    Ok(GovernedHistoricalAnswer {
        selected_edition: selected,
        as_of_tx: request.as_of_tx,
        source: source.terms().clone(),
        admitted_at: source.admitted_at(),
        facts,
    })
}
