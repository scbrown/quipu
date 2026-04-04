//! Virtual graph provider trait — federation interface for external data sources.
//!
//! The `GraphProvider` trait abstracts over different knowledge graph backends,
//! enabling Quipu to federate queries across its local SQLite store and external
//! sources like Graphiti (FalkorDB).

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
}

/// Local provider backed by Quipu's SQLite store.
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
        crate::mcp::tool_cord(self.store, &input)
    }

    fn health(&self) -> ProviderStatus {
        let fact_count = self
            .store
            .current_facts()
            .ok()
            .map(|f| f.len() as u64);
        ProviderStatus {
            name: self.label.clone(),
            healthy: true,
            fact_count,
            message: None,
        }
    }
}

/// Federated provider that combines results from multiple providers.
pub struct FederatedProvider<'a> {
    providers: Vec<Box<dyn GraphProvider + 'a>>,
}

impl<'a> FederatedProvider<'a> {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn add(&mut self, provider: Box<dyn GraphProvider + 'a>) {
        self.providers.push(provider);
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    /// Query all providers and merge results.
    /// Rows are tagged with a `_provider` field to identify the source.
    pub fn query_all(&self, sparql: &str) -> Result<QueryResult> {
        let mut merged_rows = Vec::new();
        let mut variables = Vec::new();
        let mut provider_var_added = false;

        for provider in &self.providers {
            match provider.query(sparql) {
                Ok(result) => {
                    if variables.is_empty() {
                        variables = result.variables.clone();
                        if !variables.contains(&"_provider".to_string()) {
                            variables.push("_provider".to_string());
                            provider_var_added = true;
                        }
                    }
                    for mut row in result.rows {
                        if provider_var_added {
                            row.insert(
                                "_provider".to_string(),
                                crate::types::Value::Str(provider.name().to_string()),
                            );
                        }
                        merged_rows.push(row);
                    }
                }
                Err(_) => continue, // Skip failed providers
            }
        }

        Ok(QueryResult {
            variables,
            rows: merged_rows,
        })
    }

    /// Health check all providers.
    pub fn health_all(&self) -> Vec<ProviderStatus> {
        self.providers.iter().map(|p| p.health()).collect()
    }

    /// List entities from all providers.
    pub fn entities_all(
        &self,
        type_filter: Option<&str>,
        limit: usize,
    ) -> Result<JsonValue> {
        let mut all_entities = Vec::new();

        for provider in &self.providers {
            if let Ok(result) = provider.entities(type_filter, limit) {
                if let Some(entities) = result["entities"].as_array() {
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
        assert_eq!(result.rows.len(), 1);
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
        let store_a = make_store(
            "@prefix ex: <http://example.org/> .\nex:alice ex:name \"Alice\" .",
        );
        let store_b = make_store(
            "@prefix ex: <http://example.org/> .\nex:bob ex:name \"Bob\" .",
        );

        let mut fed = FederatedProvider::new();
        fed.add(Box::new(LocalProvider::new(&store_a, "store-a")));
        fed.add(Box::new(LocalProvider::new(&store_b, "store-b")));

        let result = fed
            .query_all("SELECT ?s ?name WHERE { ?s <http://example.org/name> ?name }")
            .unwrap();
        assert_eq!(result.rows.len(), 2);
        assert!(result.variables.contains(&"_provider".to_string()));
    }

    #[test]
    fn test_federated_health() {
        let store = make_store(
            "@prefix ex: <http://example.org/> .\nex:a ex:b \"c\" .",
        );
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
