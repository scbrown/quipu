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
    /// Source identifier for unified search (e.g. "knowledge" vs Bobbin's "code").
    pub source: String,
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

    /// Query with optional embedding-based semantic search and predicate pushdown.
    ///
    /// When `embedding` is provided, vector similarity results are merged into
    /// the text-search and link-expansion results as `Semantic` relevance hits.
    /// The optional `filter` is forwarded to [`KnowledgeVectorStore::vector_search_filtered`]
    /// for predicate pushdown on backends that support it (e.g. `LanceDB`).
    pub fn query_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        filter: Option<&str>,
    ) -> Result<KnowledgeContext> {
        let mut ctx = self.query(query)?;

        let semantic_hits = self.store.vector_store().vector_search_filtered(
            embedding,
            self.config.max_entities,
            filter,
            None,
        )?;

        let mut seen: Vec<String> = ctx.entities.iter().map(|e| e.iri.clone()).collect();
        let mut semantic_count = 0;

        for hit in semantic_hits {
            if ctx.entities.len() >= self.config.max_entities {
                break;
            }
            if let Ok(iri) = self.store.resolve(hit.entity_id)
                && !seen.contains(&iri)
            {
                seen.push(iri.clone());
                if let Ok(entity) =
                    self.build_entity(&iri, KnowledgeRelevance::Semantic, hit.score as f32)
                {
                    semantic_count += 1;
                    ctx.entities.push(entity);
                }
            }
        }

        ctx.summary.total_entities += semantic_count;
        ctx.summary.total_facts = ctx.entities.iter().map(|e| e.facts.len()).sum();

        // Re-sort after merging semantic hits.
        ctx.entities.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        ctx.entities.truncate(self.config.max_entities);

        Ok(ctx)
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
        entities.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
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
        let safe_iri = iri
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('>', "\\>");

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
                    if let Ok(entity) =
                        self.build_entity(&neighbor_iri, KnowledgeRelevance::Linked, 0.5)
                    {
                        neighbors.push(entity);
                    }
                }
            }
        }

        if let Ok(result) = sparql::query(self.store, &sparql_in) {
            for row in result.rows() {
                if let Some(Value::Ref(id)) = row.get("s") {
                    let neighbor_iri = self.store.resolve(*id)?;
                    if let Ok(entity) =
                        self.build_entity(&neighbor_iri, KnowledgeRelevance::Linked, 0.5)
                    {
                        neighbors.push(entity);
                    }
                }
            }
        }

        Ok(neighbors)
    }

    /// Build a `KnowledgeEntity` by loading all current facts about an IRI.
    fn build_entity(
        &self,
        iri: &str,
        relevance: KnowledgeRelevance,
        score: f32,
    ) -> Result<KnowledgeEntity> {
        let safe_iri = iri
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
            .replace('>', "\\>");

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
            source: "knowledge".to_string(),
            facts,
        })
    }
}

pub mod tools;

pub use tools::{tool_context, tool_unified_search};

#[cfg(test)]
mod tests;
