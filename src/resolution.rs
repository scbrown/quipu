//! Entity resolution — detect near-duplicate entities before writing.
//!
//! On entity writes (episode ingest or direct fact insert), the resolver:
//! 1. Computes an embedding of the entity's canonical name + properties.
//! 2. Queries the existing vector index for top-K nearest entities above a
//!    configurable similarity threshold.
//! 3. Runs canonical name matching (Jaro-Winkler) alongside the vector query
//!    to catch typos the embedding may miss.
//! 4. Returns merged, deduplicated candidates with scores and match explanations.

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::namespace;
use crate::store::Store;

/// A candidate entity match from resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityCandidate {
    /// The IRI of the existing entity.
    pub iri: String,
    /// Similarity score (0.0 to 1.0).
    pub score: f64,
    /// How the match was found: `"embedding:0.91"` or `"canonical_name:jaro_winkler:0.95"`.
    pub matched_on: String,
}

/// Result of entity resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionResult {
    /// Whether any candidates exceeded the threshold.
    pub has_matches: bool,
    /// Candidate entities sorted by descending score.
    pub candidates: Vec<EntityCandidate>,
}

/// Resolve a candidate entity name against the existing knowledge graph.
///
/// Combines vector similarity (embedding-based) and canonical name matching
/// (Jaro-Winkler) to find entities that may be duplicates of the proposed
/// name + properties. Results are merged, deduplicated by IRI, and sorted
/// by descending score.
pub fn resolve_entity(
    store: &Store,
    name: &str,
    properties: &[(String, String)],
    threshold: f64,
    top_k: usize,
) -> Result<ResolutionResult> {
    let mut candidates = Vec::new();

    // Phase 1: Vector similarity search.
    // Build text from name + properties, embed it, search the vector store.
    let text = build_resolution_text(name, properties);

    if let Some(embedding) = store.embed_query(&text)? {
        let vs = store.vector_store();
        // Oversample to allow room after threshold filtering.
        let matches = vs.vector_search(&embedding, top_k * 3, None)?;

        for m in &matches {
            if m.score >= threshold {
                let iri = store.resolve(m.entity_id)?;
                candidates.push(EntityCandidate {
                    iri,
                    score: m.score,
                    matched_on: format!("embedding:{:.2}", m.score),
                });
            }
        }
    }

    // Phase 2: Canonical name matching (Jaro-Winkler).
    // Query all entities with rdfs:label and compare names.
    let label_candidates = resolve_by_name(store, name, threshold)?;
    candidates.extend(label_candidates);

    // Merge: deduplicate by IRI, keeping the highest score.
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    dedup_by_iri(&mut candidates);
    candidates.truncate(top_k);

    Ok(ResolutionResult {
        has_matches: !candidates.is_empty(),
        candidates,
    })
}

/// Build embeddable text for resolution from a name and optional properties.
fn build_resolution_text(name: &str, properties: &[(String, String)]) -> String {
    let mut parts = vec![name.to_string()];
    for (k, v) in properties {
        parts.push(format!("{k}: {v}"));
    }
    parts.join(". ")
}

/// Resolve by canonical name using Jaro-Winkler similarity.
///
/// Queries all current `rdfs:label` values and compares with the proposed name.
fn resolve_by_name(store: &Store, name: &str, threshold: f64) -> Result<Vec<EntityCandidate>> {
    let rdfs_label_iri = format!("{}label", namespace::RDFS);
    let Some(label_id) = store.lookup(&rdfs_label_iri)? else {
        return Ok(vec![]); // No labels interned yet.
    };

    // Query current facts with the rdfs:label attribute.
    // Schema: facts(e, a, v BLOB, tx, valid_from, valid_to, op)
    // Values are tagged BLOBs: tag byte 1 = string, followed by UTF-8.
    let mut stmt = store.conn.prepare(
        "SELECT e, v FROM facts \
         WHERE a = ?1 AND valid_to IS NULL AND op = 1",
    )?;

    let mut candidates = Vec::new();
    let name_lower = name.to_lowercase();

    let rows = stmt.query_map(rusqlite::params![label_id], |row| {
        let entity_id: i64 = row.get(0)?;
        let v_bytes: Vec<u8> = row.get(1)?;
        Ok((entity_id, v_bytes))
    })?;

    for row in rows {
        let (entity_id, v_bytes) = row?;

        // Decode the value — only process string values.
        let Ok(crate::types::Value::Str(label)) = crate::types::Value::from_bytes(&v_bytes) else {
            continue;
        };

        let label_lower = label.to_lowercase();

        // Exact match (case-insensitive).
        if name_lower == label_lower {
            let iri = store.resolve(entity_id)?;
            candidates.push(EntityCandidate {
                iri,
                score: 1.0,
                matched_on: "canonical_name:exact".to_string(),
            });
            continue;
        }

        // Jaro-Winkler similarity.
        let jw = strsim::jaro_winkler(&name_lower, &label_lower);
        if jw >= threshold {
            let iri = store.resolve(entity_id)?;
            candidates.push(EntityCandidate {
                iri,
                score: jw,
                matched_on: format!("canonical_name:jaro_winkler:{jw:.2}"),
            });
        }
    }

    Ok(candidates)
}

/// Deduplicate candidates by IRI, keeping the highest-scoring entry for each.
fn dedup_by_iri(candidates: &mut Vec<EntityCandidate>) {
    // Candidates are already sorted by descending score, so the first
    // occurrence of each IRI is the highest-scoring one.
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|c| seen.insert(c.iri.clone()));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::embedding::EmbeddingProvider;
    use crate::rdf::ingest_rdf;

    /// Deterministic test embedding provider — produces embeddings based
    /// on text length so that similar-length texts have similar vectors.
    struct DummyProvider;

    impl EmbeddingProvider for DummyProvider {
        fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
            let seed = text.len() as f32;
            Ok((0..8).map(|i| (seed + i as f32 * 0.1).sin()).collect())
        }

        fn dimension(&self) -> usize {
            8
        }
    }

    #[test]
    fn resolve_finds_exact_name_match() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_embedding_provider(Arc::new(DummyProvider));
        store.embedding_config_mut().auto_embed = true;

        let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:Alice a ex:Person ;
    rdfs:label "Alice" ;
    rdfs:comment "A software engineer" .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        // Exact name match.
        let result = resolve_entity(&store, "Alice", &[], 0.85, 3).unwrap();
        assert!(result.has_matches);
        assert!(!result.candidates.is_empty());
        assert!(result.candidates[0].iri.contains("Alice"));
        assert_eq!(result.candidates[0].score, 1.0);
        assert!(
            result.candidates[0]
                .matched_on
                .starts_with("canonical_name:exact")
        );
    }

    #[test]
    fn resolve_finds_similar_name_jaro_winkler() {
        let mut store = Store::open_in_memory().unwrap();

        let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice_smith a ex:Person ;
    rdfs:label "Alice Smith" .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        // Similar name should match via Jaro-Winkler.
        let result = resolve_entity(&store, "Alice Smth", &[], 0.85, 3).unwrap();
        assert!(
            result.has_matches,
            "expected 'Alice Smth' to match 'Alice Smith' via Jaro-Winkler"
        );
        assert!(result.candidates[0].score > 0.85);
        assert!(
            result.candidates[0].matched_on.contains("jaro_winkler"),
            "expected Jaro-Winkler match, got: {}",
            result.candidates[0].matched_on
        );
    }

    #[test]
    fn resolve_no_match_for_dissimilar_name() {
        let mut store = Store::open_in_memory().unwrap();

        let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
    rdfs:label "Alice" .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        // Very different name should not match.
        let result = resolve_entity(&store, "Zebra Corporation", &[], 0.85, 3).unwrap();
        assert!(
            !result.has_matches,
            "expected no match for 'Zebra Corporation' vs 'Alice'"
        );
    }

    #[test]
    fn resolve_with_embedding_similarity() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_embedding_provider(Arc::new(DummyProvider));
        store.embedding_config_mut().auto_embed = true;

        let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
    rdfs:label "Alice" ;
    rdfs:comment "A software engineer" .

ex:bob a ex:Person ;
    rdfs:label "Bob" ;
    rdfs:comment "A data scientist" .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        // Resolve with a name that has similar embedding.
        let result = resolve_entity(&store, "Alic", &[], 0.50, 5).unwrap();
        // With DummyProvider, similar-length texts → similar embeddings.
        // At least the name-based match should fire.
        assert!(
            result.has_matches,
            "expected at least a name-based match for 'Alic' vs 'Alice'"
        );
    }

    #[test]
    fn resolve_disabled_returns_empty() {
        let store = Store::open_in_memory().unwrap();

        let result = resolve_entity(&store, "Alice", &[], 0.85, 3).unwrap();
        // No entities in store → no matches.
        assert!(!result.has_matches);
        assert!(result.candidates.is_empty());
    }

    #[test]
    fn resolve_threshold_099_effectively_off() {
        let mut store = Store::open_in_memory().unwrap();

        let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
    rdfs:label "Alice" .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        // Very high threshold should only match exact names.
        let result = resolve_entity(&store, "Alic", &[], 0.99, 3).unwrap();
        assert!(
            !result.has_matches,
            "expected no match at threshold 0.99 for 'Alic' vs 'Alice'"
        );
    }

    #[test]
    fn resolve_deduplicates_by_iri() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_embedding_provider(Arc::new(DummyProvider));
        store.embedding_config_mut().auto_embed = true;

        let turtle = r#"
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .

ex:alice a ex:Person ;
    rdfs:label "Alice" ;
    rdfs:comment "A software engineer" .
"#;
        ingest_rdf(
            &mut store,
            turtle.as_bytes(),
            oxrdfio::RdfFormat::Turtle,
            None,
            "2026-01-01",
            None,
            None,
        )
        .unwrap();

        // "Alice" should be found by both embedding and name matching
        // but should appear only once in results.
        let result = resolve_entity(&store, "Alice", &[], 0.50, 10).unwrap();
        let alice_count = result
            .candidates
            .iter()
            .filter(|c| c.iri.contains("alice"))
            .count();
        assert_eq!(
            alice_count, 1,
            "expected exactly one Alice candidate after dedup, got {alice_count}"
        );
    }
}
