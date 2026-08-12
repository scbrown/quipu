//! PPR re-ranking of the context pipeline's candidate set (quipu-mq7,
//! pagerank design Phase 4).

use std::collections::HashMap;

use super::{KnowledgeEntity, KnowledgeRelevance};
use crate::error::Result;
use crate::store::Store;

/// Re-order `entities` by Personalized `PageRank` seeded at the DIRECT hits, so
/// link-expanded neighbours are ranked by structural importance relative to
/// what the query actually matched, not by discovery order or text score
/// alone.
///
/// Opt-in via `ContextPipelineConfig::ppr_rerank`, and deliberately a
/// re-ORDER, not a re-SCORE: each entity keeps its own relevance score (text /
/// BM25 / cosine — a different scale entirely), the PPR ordering only decides
/// who survives the `max_entities` truncation. Ties (entities the projection
/// does not reach — both score 0) fall back to the original relevance score.
/// With no direct hits there is nothing to seed, so the order is untouched.
pub(super) fn apply_ppr(store: &Store, entities: &mut [KnowledgeEntity]) -> Result<()> {
    let seeds: Vec<i64> = entities
        .iter()
        .filter(|e| matches!(e.relevance, KnowledgeRelevance::Direct))
        .filter_map(|e| store.lookup(&e.iri).ok().flatten())
        .collect();
    if seeds.is_empty() {
        return Ok(());
    }

    let pg = crate::graph::project_cached(store, None, None, None)?;
    let cfg = crate::graph::PageRankConfig {
        seeds,
        ..Default::default()
    };
    let scores: HashMap<String, f32> = crate::graph::page_rank(&pg, &cfg)?
        .into_iter()
        .filter_map(|(id, score)| store.resolve(id).ok().map(|iri| (iri, score)))
        .collect();

    entities.sort_by(|a, b| {
        let (sa, sb) = (
            scores.get(&a.iri).copied().unwrap_or(0.0),
            scores.get(&b.iri).copied().unwrap_or(0.0),
        );
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    Ok(())
}
