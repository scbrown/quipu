//! Unified context pipeline — blends knowledge graph facts with code context.
//!
//! This module provides the integration surface between Quipu's knowledge graph
//! and Bobbin's code context engine. When an agent asks for context about a topic,
//! the pipeline queries Quipu for relevant entities, facts, and relationships,
//! producing `KnowledgeContext` results that Bobbin can merge with code results.
//!
//! The output shape mirrors Bobbin's `ContextFile`/`ContextChunk` hierarchy so
//! results can be interleaved in a single ranked response.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::sparql;
use crate::store::Store;
use crate::types::Value;

/// A knowledge context result — the Quipu counterpart to Bobbin's `ContextBundle`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeContext {
    pub query: String,
    pub entities: Vec<KnowledgeEntity>,
    pub summary: KnowledgeSummary,
}

/// A discovered entity with its facts — analogous to Bobbin's `ContextFile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeEntity {
    pub iri: String,
    pub label: Option<String>,
    pub types: Vec<String>,
    pub relevance: KnowledgeRelevance,
    pub score: f32,
    pub facts: Vec<KnowledgeFact>,
}

/// A single fact about an entity — analogous to Bobbin's `ContextChunk`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeFact {
    pub predicate: String,
    pub value: String,
    pub value_type: FactValueType,
}

/// How a knowledge entity was discovered.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum KnowledgeRelevance {
    /// Found via direct SPARQL text match on the query terms.
    Direct,
    /// Found via graph traversal from a direct hit (neighbor).
    Linked,
    /// Found via vector similarity search.
    Semantic,
}

/// The type of a fact's value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FactValueType {
    Entity,
    Literal,
}

/// Summary statistics for a knowledge context response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSummary {
    pub total_entities: usize,
    pub total_facts: usize,
    pub direct_hits: usize,
    pub linked_additions: usize,
}

/// Configuration for the context pipeline.
#[derive(Debug, Clone)]
pub struct ContextPipelineConfig {
    /// Maximum number of entities to return.
    pub max_entities: usize,
    /// Maximum number of facts per entity.
    pub max_facts_per_entity: usize,
    /// Whether to expand results by following links from direct hits.
    pub expand_links: bool,
    /// Maximum link expansion depth (1 = immediate neighbors only).
    pub link_depth: u32,
}

impl Default for ContextPipelineConfig {
    fn default() -> Self {
        Self {
            max_entities: 20,
            max_facts_per_entity: 20,
            expand_links: true,
            link_depth: 1,
        }
    }
}

/// The unified context pipeline — queries Quipu for knowledge relevant to a topic.
pub struct ContextPipeline<'a> {
    store: &'a Store,
    config: ContextPipelineConfig,
}

impl<'a> ContextPipeline<'a> {
    pub fn new(store: &'a Store, config: ContextPipelineConfig) -> Self {
        Self { store, config }
    }

    /// Query the knowledge graph for entities relevant to the given query string.
    ///
    /// Strategy:
    /// 1. Text search — SPARQL FILTER(CONTAINS) on entity IRIs and literal values
    /// 2. Link expansion — follow outgoing/incoming relations from direct hits
    /// 3. Rank and truncate to budget
    pub fn query(&self, query: &str) -> Result<KnowledgeContext> {
        let mut entities = Vec::new();
        let mut seen_iris: Vec<String> = Vec::new();

        // Step 1: Find entities whose IRIs or literal values contain query terms.
        let direct_hits = self.text_search(query)?;
        let direct_count = direct_hits.len();

        for entity in direct_hits {
            if !seen_iris.contains(&entity.iri) {
                seen_iris.push(entity.iri.clone());
                entities.push(entity);
            }
        }

        // Step 2: Expand links from direct hits.
        let mut linked_count = 0;
        if self.config.expand_links && !entities.is_empty() {
            let seed_iris: Vec<String> = entities.iter().map(|e| e.iri.clone()).collect();
            for iri in &seed_iris {
                if entities.len() >= self.config.max_entities {
                    break;
                }
                let neighbors = self.linked_entities(iri)?;
                for neighbor in neighbors {
                    if entities.len() >= self.config.max_entities {
                        break;
                    }
                    if !seen_iris.contains(&neighbor.iri) {
                        seen_iris.push(neighbor.iri.clone());
                        linked_count += 1;
                        entities.push(neighbor);
                    }
                }
            }
        }

        // Step 3: Sort by score descending, truncate.
        entities.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        entities.truncate(self.config.max_entities);

        let total_facts: usize = entities.iter().map(|e| e.facts.len()).sum();

        Ok(KnowledgeContext {
            query: query.to_string(),
            entities,
            summary: KnowledgeSummary {
                total_entities: direct_count + linked_count,
                total_facts,
                direct_hits: direct_count,
                linked_additions: linked_count,
            },
        })
    }

    /// Find entities whose IRIs or literal values match query terms.
    fn text_search(&self, query: &str) -> Result<Vec<KnowledgeEntity>> {
        // Escape single quotes for SPARQL injection safety.
        let safe_query = query.replace('\\', "\\\\").replace('\'', "\\'");

        // Find entities that have any literal value containing the query string.
        let sparql = format!(
            "SELECT DISTINCT ?s WHERE {{ \
                ?s ?p ?o . \
                FILTER(CONTAINS(LCASE(STR(?s)), LCASE('{safe_query}')) || \
                       CONTAINS(LCASE(STR(?o)), LCASE('{safe_query}'))) \
            }} LIMIT {}",
            self.config.max_entities
        );

        let result = sparql::query(self.store, &sparql)?;

        let mut entities = Vec::new();
        for row in result.rows() {
            if let Some(Value::Ref(id)) = row.get("s") {
                let iri = self.store.resolve(*id)?;
                if let Ok(entity) = self.build_entity(&iri, KnowledgeRelevance::Direct, 1.0) {
                    entities.push(entity);
                }
            }
        }

        Ok(entities)
    }

    /// Find entities linked to the given IRI (outgoing object references + incoming subjects).
    fn linked_entities(&self, iri: &str) -> Result<Vec<KnowledgeEntity>> {
        let safe_iri = iri.replace('\\', "\\\\").replace('\'', "\\'").replace('>', "\\>");

        // Outgoing: ?iri ?p ?neighbor where ?neighbor is a resource.
        let sparql_out = format!(
            "SELECT DISTINCT ?o WHERE {{ \
                <{safe_iri}> ?p ?o . \
                FILTER(isIRI(?o)) \
            }} LIMIT 10"
        );

        // Incoming: ?neighbor ?p ?iri.
        let sparql_in = format!(
            "SELECT DISTINCT ?s WHERE {{ \
                ?s ?p <{safe_iri}> . \
            }} LIMIT 10"
        );

        let mut neighbors = Vec::new();

        if let Ok(result) = sparql::query(self.store, &sparql_out) {
            for row in result.rows() {
                if let Some(Value::Ref(id)) = row.get("o") {
                    let neighbor_iri = self.store.resolve(*id)?;
                    if let Ok(entity) = self.build_entity(&neighbor_iri, KnowledgeRelevance::Linked, 0.5) {
                        neighbors.push(entity);
                    }
                }
            }
        }

        if let Ok(result) = sparql::query(self.store, &sparql_in) {
            for row in result.rows() {
                if let Some(Value::Ref(id)) = row.get("s") {
                    let neighbor_iri = self.store.resolve(*id)?;
                    if let Ok(entity) = self.build_entity(&neighbor_iri, KnowledgeRelevance::Linked, 0.5) {
                        neighbors.push(entity);
                    }
                }
            }
        }

        Ok(neighbors)
    }

    /// Build a KnowledgeEntity by loading all current facts about an IRI.
    fn build_entity(&self, iri: &str, relevance: KnowledgeRelevance, score: f32) -> Result<KnowledgeEntity> {
        let safe_iri = iri.replace('\\', "\\\\").replace('\'', "\\'").replace('>', "\\>");

        let sparql = format!(
            "SELECT ?p ?o WHERE {{ <{safe_iri}> ?p ?o }} LIMIT {}",
            self.config.max_facts_per_entity
        );

        let result = sparql::query(self.store, &sparql)?;

        let mut label = None;
        let mut types = Vec::new();
        let mut facts = Vec::new();

        for row in result.rows() {
            let pred_str = match row.get("p") {
                Some(Value::Ref(id)) => self.store.resolve(*id)?,
                _ => continue,
            };

            let (val_str, val_type) = match row.get("o") {
                Some(Value::Ref(id)) => (self.store.resolve(*id)?, FactValueType::Entity),
                Some(Value::Str(s)) => (s.clone(), FactValueType::Literal),
                Some(Value::Int(n)) => (n.to_string(), FactValueType::Literal),
                Some(Value::Float(f)) => (f.to_string(), FactValueType::Literal),
                Some(Value::Bool(b)) => (b.to_string(), FactValueType::Literal),
                _ => continue,
            };

            // Extract rdfs:label and rdf:type for convenience fields.
            if pred_str.ends_with("#label") || pred_str.ends_with("/label") {
                label = Some(val_str.clone());
            }
            if pred_str.ends_with("#type") || pred_str.ends_with("/type") {
                types.push(val_str.clone());
            }

            facts.push(KnowledgeFact {
                predicate: pred_str,
                value: val_str,
                value_type: val_type,
            });
        }

        Ok(KnowledgeEntity {
            iri: iri.to_string(),
            label,
            types,
            relevance,
            score,
            facts,
        })
    }
}

/// MCP tool handler: `quipu_context` — query for knowledge context.
///
/// Input: `{ "query": "...", "max_entities": N, "expand_links": bool }`
/// Output: `KnowledgeContext` as JSON
pub fn tool_context(store: &Store, input: &serde_json::Value) -> Result<serde_json::Value> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| crate::error::Error::InvalidValue("missing 'query' parameter".into()))?;

    let mut config = ContextPipelineConfig::default();

    if let Some(max) = input.get("max_entities").and_then(|v| v.as_u64()) {
        config.max_entities = max as usize;
    }
    if let Some(expand) = input.get("expand_links").and_then(|v| v.as_bool()) {
        config.expand_links = expand;
    }

    let pipeline = ContextPipeline::new(store, config);
    let result = pipeline.query(query)?;

    Ok(serde_json::to_value(result).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rdf::ingest_rdf;
    use oxrdfio::RdfFormat;

    fn setup_test_store() -> Store {
        let store = Store::open_in_memory().unwrap();

        let turtle = r#"
@prefix ex: <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

ex:traefik rdf:type ex:WebApplication ;
    rdfs:label "Traefik" ;
    ex:runsOn ex:kota ;
    ex:port "443" .

ex:kota rdf:type ex:ProxmoxNode ;
    rdfs:label "Kota" ;
    ex:hostname "kota.lan" ;
    ex:manages ex:traefik .

ex:forgejo rdf:type ex:WebApplication ;
    rdfs:label "Forgejo" ;
    ex:runsOn ex:koror ;
    ex:port "3000" .

ex:koror rdf:type ex:ProxmoxNode ;
    rdfs:label "Koror" ;
    ex:hostname "koror.lan" .
        "#;

        let mut store = store;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            RdfFormat::Turtle,
            None,
            "2026-01-01T00:00:00Z",
            None,
            Some("test.ttl"),
        )
        .unwrap();
        store
    }

    #[test]
    fn direct_text_search() {
        let store = setup_test_store();
        let pipeline = ContextPipeline::new(&store, ContextPipelineConfig {
            expand_links: false,
            ..Default::default()
        });

        let ctx = pipeline.query("traefik").unwrap();
        assert!(!ctx.entities.is_empty());
        assert!(ctx.entities.iter().any(|e| e.iri.contains("traefik")));
        assert_eq!(ctx.entities[0].relevance, KnowledgeRelevance::Direct);
    }

    #[test]
    fn link_expansion() {
        let store = setup_test_store();
        let pipeline = ContextPipeline::new(&store, ContextPipelineConfig {
            expand_links: true,
            ..Default::default()
        });

        let ctx = pipeline.query("traefik").unwrap();
        // Should find traefik (direct) and kota (linked via runsOn/manages).
        assert!(ctx.entities.len() >= 2);
        assert!(ctx.entities.iter().any(|e| e.iri.contains("traefik")));
        assert!(ctx.entities.iter().any(|e| e.iri.contains("kota")));
        assert!(ctx.summary.linked_additions > 0);
    }

    #[test]
    fn entity_has_label_and_types() {
        let store = setup_test_store();
        let pipeline = ContextPipeline::new(&store, ContextPipelineConfig {
            expand_links: false,
            ..Default::default()
        });

        let ctx = pipeline.query("traefik").unwrap();
        let traefik = ctx.entities.iter().find(|e| e.iri.contains("traefik")).unwrap();
        assert_eq!(traefik.label, Some("Traefik".to_string()));
        assert!(traefik.types.iter().any(|t| t.contains("WebApplication")));
    }

    #[test]
    fn max_entities_respected() {
        let store = setup_test_store();
        let pipeline = ContextPipeline::new(&store, ContextPipelineConfig {
            max_entities: 1,
            expand_links: false,
            ..Default::default()
        });

        let ctx = pipeline.query("kota").unwrap();
        assert!(ctx.entities.len() <= 1);
    }

    #[test]
    fn tool_context_handler() {
        let store = setup_test_store();
        let input = serde_json::json!({
            "query": "traefik",
            "expand_links": true,
        });

        let result = tool_context(&store, &input).unwrap();
        assert!(!result["entities"].as_array().unwrap().is_empty());
        assert!(result["summary"]["direct_hits"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn no_match_returns_empty() {
        let store = Store::open_in_memory().unwrap();
        let pipeline = ContextPipeline::new(&store, ContextPipelineConfig::default());

        let ctx = pipeline.query("anything").unwrap();
        assert_eq!(ctx.entities.len(), 0);
        assert_eq!(ctx.summary.direct_hits, 0);
    }

    #[test]
    fn facts_have_correct_types() {
        let store = setup_test_store();
        let pipeline = ContextPipeline::new(&store, ContextPipelineConfig {
            expand_links: false,
            ..Default::default()
        });

        let ctx = pipeline.query("traefik").unwrap();
        let traefik = ctx.entities.iter().find(|e| e.iri.contains("traefik")).unwrap();

        // runsOn should be an Entity reference.
        let runs_on = traefik.facts.iter().find(|f| f.predicate.contains("runsOn"));
        assert!(runs_on.is_some());
        assert_eq!(runs_on.unwrap().value_type, FactValueType::Entity);

        // port should be a Literal.
        let port = traefik.facts.iter().find(|f| f.predicate.contains("port"));
        assert!(port.is_some());
        assert_eq!(port.unwrap().value_type, FactValueType::Literal);
    }
}
