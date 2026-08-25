//! Virtual graph provider trait — federation interface for external data sources.
//!
//! The `GraphProvider` trait abstracts over different knowledge graph backends,
//! enabling Quipu to federate queries across its local `SQLite` store and external
//! sources like Graphiti (`FalkorDB`).

mod label;
#[cfg(feature = "remote")]
mod remote;

pub use label::{
    DeclaredLabel, check_federated_floor, check_member_floor, federated_dataset_labels,
};
#[cfg(feature = "remote")]
pub use remote::{RemoteProvider, federated_from_config};

use serde_json::Value as JsonValue;

use crate::error::Result;
use crate::sparql::QueryResult;
use crate::store::Store;

/// Health status of a graph provider.
#[derive(Debug, Clone)]
pub struct ProviderStatus {
    pub name: String,
    pub healthy: bool,
    pub fact_count: Option<u64>,
    pub message: Option<String>,
    /// The label this member's rows carry — **declared by the local operator**,
    /// never read from the member itself (quipu-fd1, multi-db-composition.md
    /// §5). `None` = undeclared; a value is never fabricated from silence.
    pub label: Option<DeclaredLabel>,
}

/// A virtual graph provider that can answer SPARQL queries and list entities.
pub trait GraphProvider {
    /// Provider name for identification in federated results.
    fn name(&self) -> &str;

    /// Execute a SPARQL SELECT query against this provider.
    fn query(&self, sparql: &str) -> Result<QueryResult>;

    /// List entities, optionally filtered by rdf:type.
    fn entities(&self, type_filter: Option<&str>, limit: usize) -> Result<JsonValue>;

    /// Health check.
    fn health(&self) -> ProviderStatus;

    /// The label this member's rows carry, declared by the **local** operator
    /// (quipu-fd1). Default: undeclared. The local store deliberately returns
    /// `None` — its labels are per-graph and enforced by the dataset fold, not
    /// summarised into one provider-level value that could overstate.
    fn declared_label(&self) -> Option<&DeclaredLabel> {
        None
    }
}

/// Local provider backed by Quipu's `SQLite` store.
pub struct LocalProvider<'a> {
    store: &'a Store,
    label: String,
}

impl<'a> LocalProvider<'a> {
    pub fn new(store: &'a Store, label: &str) -> Self {
        Self {
            store,
            label: label.to_string(),
        }
    }
}

impl GraphProvider for LocalProvider<'_> {
    fn name(&self) -> &str {
        &self.label
    }

    fn query(&self, sparql: &str) -> Result<QueryResult> {
        crate::sparql::query(self.store, sparql)
    }

    fn entities(&self, type_filter: Option<&str>, limit: usize) -> Result<JsonValue> {
        let input = serde_json::json!({
            "type": type_filter,
            "limit": limit,
        });
        crate::mcp::tools::tool_cord(self.store, &input)
    }

    fn health(&self) -> ProviderStatus {
        let fact_count = self.store.current_facts().ok().map(|f| f.len() as u64);
        ProviderStatus {
            name: self.label.clone(),
            healthy: true,
            fact_count,
            message: None,
            label: None,
        }
    }
}

/// Federated provider that combines results from multiple providers.
#[derive(Default)]
pub struct FederatedProvider<'a> {
    providers: Vec<Box<dyn GraphProvider + 'a>>,
}

/// How one federation member fared in a [`FederatedProvider::query_all`] call.
#[derive(Debug, Clone)]
pub struct ProviderOutcome {
    /// The provider's label, as reported in `_provider` tags.
    pub name: String,
    /// Whether this member's rows are in the merged result.
    pub ok: bool,
    /// Rows this member contributed (0 when `ok` is false).
    pub rows: usize,
    /// Why the member did not contribute, when it did not.
    pub error: Option<String>,
    /// The label this member's rows carry, declared by the local operator —
    /// `None` = undeclared (quipu-fd1).
    pub label: Option<DeclaredLabel>,
}

/// A federated query's merged rows plus the per-provider account of who
/// answered — `complete` is the one-field answer to "can I trust this result
/// set as exhaustive?".
#[derive(Debug)]
pub struct FederatedQuery {
    /// The merged SELECT result, `_provider`-tagged.
    pub result: QueryResult,
    /// One entry per federation member, in provider order (local first).
    pub providers: Vec<ProviderOutcome>,
    /// True iff every member contributed its rows.
    pub complete: bool,
}

impl<'a> FederatedProvider<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, provider: Box<dyn GraphProvider + 'a>) {
        self.providers.push(provider);
    }

    /// Query all providers and merge results, REPORTING which providers
    /// answered (federation design §4).
    ///
    /// A provider that errors, returns a non-SELECT shape, or disagrees on the
    /// variable list does not abort the federated query — one dead peer must
    /// not deny the whole result — but the outcome is carried, not swallowed:
    /// `providers` names every member with its row count or failure reason, and
    /// `complete` is false the moment any member did not contribute. A `200`
    /// with fewer rows and no indication that a third of the federation did not
    /// answer is the silent-incompleteness failure class this exists to end.
    ///
    /// Rows are tagged with a `_provider` field identifying the source. The tag
    /// is decided per row, not once globally: a row that already carries
    /// `_provider` (a remote that itself federates) keeps its own tag.
    ///
    /// When any member carries a declared label (quipu-fd1), its rows are also
    /// stamped `_trust` (the trust IRI; rank and chain ride the per-member
    /// outcome) and `_freshness`. Rows from an undeclared member simply lack
    /// the binding — undeclared is absent, never fabricated.
    pub fn query_all(&self, sparql: &str) -> FederatedQuery {
        // The stamp columns a row can carry beside its data. Excluded from the
        // variable-list agreement check the same way `_provider` is.
        const META_VARS: [&str; 3] = ["_provider", "_trust", "_freshness"];
        let stamp_axis = |probe: fn(&DeclaredLabel) -> bool| {
            self.providers
                .iter()
                .any(|p| p.declared_label().is_some_and(probe))
        };
        let stamp_trust = stamp_axis(|l| l.trust.is_some());
        let stamp_fresh = stamp_axis(|l| l.freshness.is_some());

        let mut merged_rows = Vec::new();
        let mut variables: Option<Vec<String>> = None;
        let mut outcomes = Vec::new();

        for provider in &self.providers {
            let name = provider.name().to_string();
            let label = provider.declared_label().cloned();
            match provider.query(sparql) {
                Ok(QueryResult::Select {
                    variables: their_vars,
                    rows,
                }) => {
                    // The canonical variable list comes from the first provider
                    // that answers; a later provider that disagrees (a remote
                    // on an older quipu, say) is a provider-level failure, not
                    // a silent merge under mislabelled columns.
                    let canon = variables.get_or_insert_with(|| {
                        let mut v = their_vars.clone();
                        let mut want = vec!["_provider"];
                        if stamp_trust {
                            want.push("_trust");
                        }
                        if stamp_fresh {
                            want.push("_freshness");
                        }
                        for meta in want {
                            if !v.iter().any(|x| x == meta) {
                                v.push(meta.to_string());
                            }
                        }
                        v
                    });
                    let mut expected = canon.clone();
                    expected.retain(|v| !META_VARS.contains(&v.as_str()));
                    let mut theirs = their_vars.clone();
                    theirs.retain(|v| !META_VARS.contains(&v.as_str()));
                    if theirs != expected {
                        outcomes.push(ProviderOutcome {
                            name,
                            ok: false,
                            rows: 0,
                            error: Some(format!(
                                "variable list {theirs:?} does not match the federated list {expected:?}"
                            )),
                            label,
                        });
                        continue;
                    }
                    let count = rows.len();
                    for row in rows {
                        let mut row = row;
                        row.entry("_provider".to_string()).or_insert_with(|| {
                            crate::types::Value::Str(provider.name().to_string())
                        });
                        // Stamp the DECLARED label per row (quipu-fd1). Same
                        // per-row rule as `_provider`: a pre-existing stamp (a
                        // remote that itself federates) survives.
                        if let Some(l) = &label {
                            if let Some(t) = &l.trust {
                                row.entry("_trust".to_string())
                                    .or_insert_with(|| crate::types::Value::Str(t.iri.clone()));
                            }
                            if let Some(f) = l.freshness {
                                row.entry("_freshness".to_string()).or_insert_with(|| {
                                    crate::types::Value::Str(f.as_str().to_string())
                                });
                            }
                        }
                        merged_rows.push(row);
                    }
                    outcomes.push(ProviderOutcome {
                        name,
                        ok: true,
                        rows: count,
                        error: None,
                        label,
                    });
                }
                Ok(_) => {
                    // ASK / CONSTRUCT answers cannot be unioned into a SELECT
                    // row set. Refusing loudly beats merging zero rows silently.
                    outcomes.push(ProviderOutcome {
                        name,
                        ok: false,
                        rows: 0,
                        error: Some(
                            "non-SELECT result cannot be merged into a federated row set".into(),
                        ),
                        label,
                    });
                }
                Err(e) => {
                    outcomes.push(ProviderOutcome {
                        name,
                        ok: false,
                        rows: 0,
                        error: Some(e.to_string()),
                        label,
                    });
                }
            }
        }

        let complete = outcomes.iter().all(|o| o.ok);
        FederatedQuery {
            result: QueryResult::Select {
                variables: variables.unwrap_or_default(),
                rows: merged_rows,
            },
            providers: outcomes,
            complete,
        }
    }

    /// Health check all providers.
    pub fn health_all(&self) -> Vec<ProviderStatus> {
        self.providers.iter().map(|p| p.health()).collect()
    }

    /// List entities from all providers.
    pub fn entities_all(&self, type_filter: Option<&str>, limit: usize) -> Result<JsonValue> {
        let mut all_entities = Vec::new();

        for provider in &self.providers {
            if let Ok(result) = provider.entities(type_filter, limit)
                && let Some(entities) = result["entities"].as_array()
            {
                for entity in entities {
                    let mut tagged = entity.clone();
                    if let Some(obj) = tagged.as_object_mut() {
                        obj.insert(
                            "_provider".to_string(),
                            JsonValue::String(provider.name().to_string()),
                        );
                    }
                    all_entities.push(tagged);
                }
            }
        }

        Ok(serde_json::json!({
            "entities": all_entities,
            "count": all_entities.len()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::ingest_rdf;

    fn make_store(turtle: &str) -> Store {
        let mut store = Store::open_in_memory().unwrap();
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-04-04T00:00:00Z",
            None,
            None,
        )
        .unwrap();
        store
    }

    #[test]
    fn test_local_provider_query() {
        let store = make_store(
            "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .",
        );
        let provider = LocalProvider::new(&store, "local");
        let result = provider
            .query("SELECT ?name WHERE { ?s <http://example.org/name> ?name }")
            .unwrap();
        assert_eq!(result.rows().len(), 1);
    }

    #[test]
    fn test_local_provider_health() {
        let store = make_store(
            "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .",
        );
        let provider = LocalProvider::new(&store, "local");
        let status = provider.health();
        assert!(status.healthy);
        assert_eq!(status.fact_count, Some(2));
    }

    #[test]
    fn test_local_provider_entities() {
        let store = make_store(
            "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .",
        );
        let provider = LocalProvider::new(&store, "local");
        let result = provider
            .entities(Some("http://example.org/Person"), 10)
            .unwrap();
        assert_eq!(result["count"], 1);
    }

    #[test]
    fn test_federated_query() {
        let store_a =
            make_store("@prefix ex: <http://example.org/> .\nex:alice ex:name \"Alice\" .");
        let store_b = make_store("@prefix ex: <http://example.org/> .\nex:bob ex:name \"Bob\" .");

        let mut fed = FederatedProvider::new();
        fed.add(Box::new(LocalProvider::new(&store_a, "store-a")));
        fed.add(Box::new(LocalProvider::new(&store_b, "store-b")));

        let fq = fed.query_all("SELECT ?s ?name WHERE { ?s <http://example.org/name> ?name }");
        assert_eq!(fq.result.rows().len(), 2);
        assert!(fq.result.variables().contains(&"_provider".to_string()));
        assert!(fq.complete, "both members answered");
        assert_eq!(fq.providers.len(), 2);
        assert!(fq.providers.iter().all(|o| o.ok && o.rows == 1));
    }

    /// §4 of the federation design: a member that cannot answer is REPORTED,
    /// never silently skipped — `complete: false` is the one-field answer to
    /// "can I trust this result set as exhaustive?".
    struct FailingProvider;

    impl GraphProvider for FailingProvider {
        fn name(&self) -> &str {
            "broken"
        }
        fn query(&self, _sparql: &str) -> Result<QueryResult> {
            Err(crate::error::Error::InvalidValue("boom".into()))
        }
        fn entities(&self, _type_filter: Option<&str>, _limit: usize) -> Result<JsonValue> {
            Err(crate::error::Error::InvalidValue("boom".into()))
        }
        fn health(&self) -> ProviderStatus {
            ProviderStatus {
                name: "broken".into(),
                healthy: false,
                fact_count: None,
                message: None,
                label: None,
            }
        }
    }

    #[test]
    fn a_failed_member_is_reported_not_swallowed() {
        let store = make_store("@prefix ex: <http://example.org/> .\nex:a ex:name \"A\" .");
        let mut fed = FederatedProvider::new();
        fed.add(Box::new(LocalProvider::new(&store, "local")));
        fed.add(Box::new(FailingProvider));

        let fq = fed.query_all("SELECT ?s ?name WHERE { ?s <http://example.org/name> ?name }");
        assert_eq!(fq.result.rows().len(), 1, "the live member still answers");
        assert!(!fq.complete, "a dead member must flip complete to false");
        let broken = fq.providers.iter().find(|o| o.name == "broken").unwrap();
        assert!(!broken.ok);
        assert!(broken.error.as_deref().is_some_and(|e| e.contains("boom")));
    }

    /// §4.1 bug 1: a member whose variable list disagrees with the federated
    /// list is a provider-level failure, not a silent merge under mislabelled
    /// columns.
    struct FixedResultProvider {
        name: &'static str,
        variables: Vec<String>,
    }

    impl GraphProvider for FixedResultProvider {
        fn name(&self) -> &str {
            self.name
        }
        fn query(&self, _sparql: &str) -> Result<QueryResult> {
            let mut row = crate::sparql::Bindings::new();
            for v in &self.variables {
                row.insert(v.clone(), crate::types::Value::Str(format!("{v}-val")));
            }
            Ok(QueryResult::Select {
                variables: self.variables.clone(),
                rows: vec![row],
            })
        }
        fn entities(&self, _type_filter: Option<&str>, _limit: usize) -> Result<JsonValue> {
            Ok(serde_json::json!({"entities": []}))
        }
        fn health(&self) -> ProviderStatus {
            ProviderStatus {
                name: self.name.into(),
                healthy: true,
                fact_count: None,
                message: None,
                label: None,
            }
        }
    }

    #[test]
    fn a_variable_list_mismatch_is_a_provider_failure() {
        let mut fed = FederatedProvider::new();
        fed.add(Box::new(FixedResultProvider {
            name: "first",
            variables: vec!["s".into(), "name".into()],
        }));
        fed.add(Box::new(FixedResultProvider {
            name: "drifted",
            variables: vec!["s".into(), "label".into()],
        }));

        let fq = fed.query_all("SELECT ?s ?name WHERE { ?s ?p ?name }");
        assert_eq!(fq.result.rows().len(), 1, "only the agreeing member merges");
        assert!(!fq.complete);
        let drifted = fq.providers.iter().find(|o| o.name == "drifted").unwrap();
        assert!(!drifted.ok);
        assert!(
            drifted
                .error
                .as_deref()
                .is_some_and(|e| e.contains("variable list")),
            "the mismatch is named: {:?}",
            drifted.error
        );
    }

    /// §4.1 bug 2: `_provider` tagging is per row, not decided once from the
    /// first member — a member whose rows already carry `_provider` (a remote
    /// that itself federates) keeps its own tags, and every OTHER member is
    /// still tagged.
    #[test]
    fn provider_tagging_is_per_row_even_when_the_first_member_already_has_it() {
        let mut fed = FederatedProvider::new();
        fed.add(Box::new(FixedResultProvider {
            name: "already-federated",
            variables: vec!["s".into(), "_provider".into()],
        }));
        fed.add(Box::new(FixedResultProvider {
            name: "plain",
            variables: vec!["s".into()],
        }));

        let fq = fed.query_all("SELECT ?s WHERE { ?s ?p ?o }");
        assert!(fq.complete, "{:?}", fq.providers);
        let rows = fq.result.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].get("_provider"),
            Some(&crate::types::Value::Str("_provider-val".into())),
            "a pre-existing tag survives"
        );
        assert_eq!(
            rows[1].get("_provider"),
            Some(&crate::types::Value::Str("plain".into())),
            "the untagged member is still tagged"
        );
    }

    #[test]
    fn test_federated_health() {
        let store = make_store("@prefix ex: <http://example.org/> .\nex:a ex:b \"c\" .");
        let mut fed = FederatedProvider::new();
        fed.add(Box::new(LocalProvider::new(&store, "test")));
        let statuses = fed.health_all();
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].healthy);
    }

    #[test]
    fn test_federated_entities() {
        let store_a = make_store(
            "@prefix ex: <http://example.org/> .\nex:alice a ex:Person ; ex:name \"Alice\" .",
        );
        let store_b = make_store(
            "@prefix ex: <http://example.org/> .\nex:bob a ex:Person ; ex:name \"Bob\" .",
        );

        let mut fed = FederatedProvider::new();
        fed.add(Box::new(LocalProvider::new(&store_a, "a")));
        fed.add(Box::new(LocalProvider::new(&store_b, "b")));

        let result = fed
            .entities_all(Some("http://example.org/Person"), 10)
            .unwrap();
        assert_eq!(result["count"], 2);
    }
}
