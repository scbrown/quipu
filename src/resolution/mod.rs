//! Entity resolution — detect near-duplicate entities before writing.
//!
//! On entity writes (episode ingest or direct fact insert), the resolver:
//! 1. Computes an embedding of the entity's canonical name + properties.
//! 2. Queries the existing vector index for top-K nearest entities above a
//!    configurable similarity threshold.
//! 3. Runs canonical name matching (Jaro-Winkler) alongside the vector query
//!    to catch typos the embedding may miss.
//! 4. Drops any pairing the graph already excuses via `quipu:distinctFrom`.
//! 5. Returns merged, deduplicated candidates with scores and explanations.
//!
//! # What the two halves can see
//!
//! The name half reads [`Store::facts_source`] — the `UNION ALL` over the local
//! store and every attached layer — so on a composed store it sees the shared
//! reference layer a tenant is most likely to duplicate. It did not always: it
//! read the bare `facts` table, which is `main.facts`, so resolution against a
//! knowledge pack found nothing and the tenant minted a duplicate of an entity
//! the pack already defined. That is the fragmentation this module exists to
//! prevent, failing in the one deployment shape the composition design targets.
//!
//! The vector half cannot follow it there. `vectors` is a per-database table and
//! an attached pack may carry embeddings from a different model or dimension, so
//! unioning the indexes would turn a working search into a dimension-mismatch
//! error (`src/vector.rs` fails loud on that, deliberately). Rather than paper
//! over it, every result carries a [`VectorScope`] saying how much of the
//! composition the embedding half actually covered — because the failure that
//! cost us the first time was not the missing coverage, it was that missing
//! coverage and "no duplicates exist" returned the same empty list.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::namespace;
use crate::store::Store;

mod matching;
#[cfg(test)]
mod tests;

pub use matching::{
    Contention, EpisodeResolution, NodeQuery, resolve_episode_nodes, resolve_nodes,
};

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

/// How much of a composed store the embedding half of resolution searched.
///
/// See the module docs: this exists so that a caller can tell "the vector index
/// found nothing" from "the vector index was never asked about the layer your
/// duplicate is in".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VectorScope {
    /// No attachments — the vector index covers everything the name half reads.
    WholeStore,
    /// The store has attached layers whose vectors are NOT in the searched
    /// index. The name half covered them; the embedding half did not, so an
    /// embedding-only near-duplicate living in a layer will not be reported.
    LocalOnly {
        /// How many attached layers the embedding half could not see.
        attached_layers: usize,
    },
}

impl VectorScope {
    /// The scope a search against this store actually has.
    pub fn of(store: &Store) -> Self {
        match store.attachments.len() {
            0 => Self::WholeStore,
            n => Self::LocalOnly { attached_layers: n },
        }
    }

    /// Whether some part of the composition went unsearched by the vector half.
    pub fn is_partial(self) -> bool {
        matches!(self, Self::LocalOnly { .. })
    }
}

/// Result of entity resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolutionResult {
    /// Whether any candidates exceeded the threshold.
    pub has_matches: bool,
    /// Candidate entities sorted by descending score.
    pub candidates: Vec<EntityCandidate>,
    /// How much of a composed store the embedding half covered.
    pub vector_scope: VectorScope,
}

/// Resolve a candidate entity name against the existing knowledge graph.
///
/// Combines vector similarity (embedding-based) and canonical name matching
/// (Jaro-Winkler) to find entities that may be duplicates of the proposed
/// name + properties. Results are merged, deduplicated by IRI, and sorted
/// by descending score.
///
/// This is the single-entity path. Resolving several entities at once — an
/// episode's nodes — goes through [`resolve_nodes`], which scans the label set
/// once for the whole batch instead of once per entity and can see contention
/// BETWEEN the entities being written.
///
/// # Errors
/// Propagates store, embedding and decode failures.
pub fn resolve_entity(
    store: &Store,
    name: &str,
    properties: &[(String, String)],
    threshold: f64,
    top_k: usize,
) -> Result<ResolutionResult> {
    let index = LabelIndex::build(store)?;
    resolve_one(store, &index, name, properties, threshold, top_k, &[])
}

/// One entity against an already-built label index.
///
/// `excluded` holds IRIs this entity is declared or recorded distinct from;
/// they are dropped from the candidate list before it is scored or refused.
fn resolve_one(
    store: &Store,
    index: &LabelIndex,
    name: &str,
    properties: &[(String, String)],
    threshold: f64,
    top_k: usize,
    excluded: &[String],
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
                let iri = store.resolve(store.canonical_id(m.entity_id)?)?;
                candidates.push(EntityCandidate {
                    iri,
                    score: m.score,
                    matched_on: format!("embedding:{:.2}", m.score),
                });
            }
        }
    }

    // Phase 2: Canonical name matching (Jaro-Winkler), over the whole
    // composition rather than the local store alone.
    candidates.extend(index.candidates(store, name, threshold)?);

    // Phase 3: drop pairings the writer or the graph has already excused.
    if !excluded.is_empty() {
        let excused: HashSet<&str> = excluded.iter().map(String::as_str).collect();
        candidates.retain(|c| !excused.contains(c.iri.as_str()));
    }

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
        vector_scope: VectorScope::of(store),
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

/// Every current `rdfs:label` in the composition, read ONCE.
///
/// The scan used to sit inside the per-entity path, so ingesting an episode of
/// N nodes ran N full scans of the label set, each decoding every value, and
/// running a Jaro-Winkler comparison and an id→IRI resolve per row: an O(N × L)
/// walk where O(N + L) was available. The rows do not change during a resolution
/// pass, so one scan serves the whole batch.
///
/// The IRI is resolved lazily — only for rows that actually clear the threshold
/// — because `store.resolve` is a query per id and most labels match nothing.
pub(crate) struct LabelIndex {
    /// `(canonical entity id, label)` for every current `rdfs:label` fact.
    entries: Vec<(i64, String)>,
}

impl LabelIndex {
    /// Scan the composed fact source for current `rdfs:label` values.
    ///
    /// # Errors
    /// Propagates store and decode failures.
    pub(crate) fn build(store: &Store) -> Result<Self> {
        // `lookup` is main-scoped, and a layer interns `rdfs:label` in ITS OWN
        // term space — so on a composed store the local id matches none of the
        // layer's label rows, and on a store whose main file has no labels at
        // all there is no local id to look up. Either way the union would be
        // read with a predicate that selects nothing. `lookup_all` is #76's
        // answer: every id the IRI denotes across the composition, matched with
        // `a IN (…)`, which is byte-identical to `a = ?` when there are none.
        let rdfs_label_iri = format!("{}label", namespace::RDFS);
        let label_ids = store.lookup_all(&rdfs_label_iri)?;
        if label_ids.is_empty() {
            return Ok(Self {
                entries: Vec::new(),
            }); // No labels interned yet.
        }

        // Schema: facts(e, a, v BLOB, tx, valid_from, valid_to, op).
        // `facts_source()` is the literal `facts` table with no attachments and
        // the `UNION ALL` over local + layers with them, so this one statement
        // reads the composition without knowing whether there is one.
        let sql = format!(
            "SELECT e, v FROM {} WHERE a IN ({}) AND valid_to IS NULL AND op = 1",
            store.facts_source(),
            id_list(&label_ids),
        );
        let mut stmt = store.conn.prepare(&sql)?;

        let rows = stmt.query_map([], |row| {
            let entity_id: i64 = row.get(0)?;
            let v_bytes: Vec<u8> = row.get(1)?;
            Ok((entity_id, v_bytes))
        })?;

        let mut entries = Vec::new();
        for row in rows {
            let (entity_id, v_bytes) = row?;
            // Only string values are labels.
            let Ok(crate::types::Value::Str(label)) = crate::types::Value::from_bytes(&v_bytes)
            else {
                continue;
            };
            // An attached layer and the local store can intern the same IRI at
            // different ids; canonicalising here means one entity yields one
            // candidate rather than one per layer that mentions it.
            entries.push((store.canonical_id(entity_id)?, label));
        }
        Ok(Self { entries })
    }

    /// Candidates for `name` by exact (case-insensitive) then Jaro-Winkler match.
    fn candidates(
        &self,
        store: &Store,
        name: &str,
        threshold: f64,
    ) -> Result<Vec<EntityCandidate>> {
        let name_lower = name.to_lowercase();
        let mut candidates = Vec::new();
        for (entity_id, label) in &self.entries {
            let label_lower = label.to_lowercase();

            // Exact match (case-insensitive).
            if name_lower == label_lower {
                candidates.push(EntityCandidate {
                    iri: store.resolve(*entity_id)?,
                    score: 1.0,
                    matched_on: "canonical_name:exact".to_string(),
                });
                continue;
            }

            // Canonical commit node names identify immutable objects. Two
            // different hashes are distinct even when their shared
            // `commit/<repo>/` prefix gives them a high Jaro-Winkler score.
            // Keep exact matching above for idempotent reuse, and keep this
            // deliberately narrower than a generic kind-prefix exemption.
            if is_slash_qualified_commit_id(&name_lower)
                && is_slash_qualified_commit_id(&label_lower)
            {
                continue;
            }

            // Jaro-Winkler similarity.
            let jw = strsim::jaro_winkler(&name_lower, &label_lower);
            if jw >= threshold {
                candidates.push(EntityCandidate {
                    iri: store.resolve(*entity_id)?,
                    score: jw,
                    matched_on: format!("canonical_name:jaro_winkler:{jw:.2}"),
                });
            }
        }
        Ok(candidates)
    }
}

/// Whether `name` is a canonical `commit/<repo>/<hex-sha>` node name.
///
/// Git SHA-1 and SHA-256 object ids are 40 and 64 hex digits respectively;
/// seven digits is the conventional minimum useful abbreviated commit id.
fn is_slash_qualified_commit_id(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("commit/") else {
        return false;
    };
    let Some((repo, sha)) = rest.rsplit_once('/') else {
        return false;
    };
    !repo.is_empty()
        && !repo.split('/').any(str::is_empty)
        && (7..=64).contains(&sha.len())
        && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The IRIs `iri` is already recorded as `quipu:distinctFrom` in the graph.
///
/// This is what makes the override durable. A writer asserts the pairing once;
/// every later resolution of the same entity reads it back and stays silent
/// about that one pairing, without `strict_mode` having to be disabled for
/// every entity in the store.
///
/// # Errors
/// Propagates store and decode failures.
pub(crate) fn recorded_distinct_from(store: &Store, iri: &str) -> Result<Vec<String>> {
    // Both sides need every id the IRI denotes across the composition, for the
    // same reason the label scan does — see [`LabelIndex::build`].
    let subjects = store.lookup_all(iri)?;
    let preds = store.lookup_all(namespace::QUIPU_DISTINCT_FROM)?;
    if subjects.is_empty() || preds.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT v FROM {} WHERE e IN ({}) AND a IN ({}) AND valid_to IS NULL AND op = 1",
        store.facts_source(),
        id_list(&subjects),
        id_list(&preds),
    );
    let mut stmt = store.conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))?;

    let mut out = Vec::new();
    for row in rows {
        match crate::types::Value::from_bytes(&row?) {
            // The object is normally an IRI reference…
            Ok(crate::types::Value::Ref(id)) => out.push(store.resolve(id)?),
            // …but a writer that asserted it as a plain literal meant the same
            // thing, and silently ignoring that would put us back to an override
            // that looks asserted and does nothing.
            Ok(crate::types::Value::Str(s)) => out.push(s),
            _ => {}
        }
    }
    Ok(out)
}

/// Deduplicate candidates by IRI, keeping the highest-scoring entry for each.
fn dedup_by_iri(candidates: &mut Vec<EntityCandidate>) {
    // Candidates are already sorted by descending score, so the first
    // occurrence of each IRI is the highest-scoring one.
    let mut seen = HashSet::new();
    candidates.retain(|c| seen.insert(c.iri.clone()));
}

/// Term ids as a SQL `IN` list.
///
/// Inlined rather than bound because the arity varies with the number of
/// attached layers; the values are `i64`s the store itself produced, never
/// caller text, so there is nothing here to inject.
fn id_list(ids: &[i64]) -> String {
    ids.iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}
