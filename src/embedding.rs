//! Auto-embedding support for entity writes.
//!
//! Provides the [`EmbeddingProvider`] trait for pluggable embedding backends
//! (e.g. Bobbin's ONNX pipeline) and [`build_entity_text`] for constructing
//! embeddable text from an entity's current facts.
//!
//! When `auto_embed` is enabled in config and an `EmbeddingProvider` is
//! attached to the [`Store`], entities are automatically embedded after
//! each successful transaction.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::error::Result;
use crate::namespace;
use crate::store::Store;
use crate::types::{Op, Value};

use super::store::Datum;

/// Trait for pluggable embedding backends.
///
/// Implementations must be `Send + Sync` so the provider can be shared
/// via `Arc<dyn EmbeddingProvider>` across threads and subsystems.
pub trait EmbeddingProvider: Send + Sync {
    /// Embed a single text string.
    fn embed_text(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a batch of texts. The default calls `embed_text` in a loop;
    /// backends with native batching should override for efficiency.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed_text(t)).collect()
    }

    /// Return the embedding dimension (e.g. 384 for all-MiniLM-L6-v2).
    fn dimension(&self) -> usize;
}

/// What to configure when no [`EmbeddingProvider`] is attached (quipu #53).
///
/// "No embedding provider configured" on its own sent a user through the
/// `--features onnx` rebuild, the model download and the config file one at a
/// time, because the error named none of them. Every site that discovers a
/// missing provider appends this, so the fix is in the message rather than in
/// the reader's head.
pub const NO_PROVIDER_HELP: &str = "\
no embedding provider configured — semantic search is unavailable.

Building with `--features onnx` supplies the ONNX RUNTIME only; it does not
supply a model. A provider needs BOTH model files on disk AND the paths to
them in `.bobbin/config.toml`:

    [quipu.embedding]
    auto_embed = true
    model_path = \"models/all-MiniLM-L6-v2/onnx/model.onnx\"
    tokenizer_path = \"models/all-MiniLM-L6-v2/tokenizer.json\"
    dimension = 384

Note that `quipu knot` does NOT embed — only `/episode` writes auto-embed.
Run `quipu-server --embed-backfill` once after knotting to embed a store
loaded from Turtle. See docs/book/src/concepts/embeddings.md.";

/// Build embeddable text for an entity from its current facts.
///
/// Text is constructed in priority order:
/// 1. `rdfs:label` — the entity's display name
/// 2. `rdfs:comment` — descriptive text
/// 3. `rdf:type` — resolved type IRI(s)
/// 4. Other literal properties (strings, ints, floats, bools)
///
/// Returns an empty string if the entity has no facts (fully retracted).
pub fn build_entity_text(store: &Store, entity_id: i64) -> Result<String> {
    let facts = store.entity_facts(entity_id)?;
    if facts.is_empty() {
        return Ok(String::new());
    }

    // Track (tx, value) for the single-valued fields so the most recent
    // assertion wins. entity_facts orders only by attribute, so when a label or
    // comment has been updated by a plain assert that left the prior value
    // active, choosing among them by row order would be nondeterministic and
    // could embed stale text — the embedding would drift from current facts
    // (hq-xqc). Selecting the highest tx keeps the embedding tracking the
    // current value regardless of row order.
    let mut label: Option<(i64, String)> = None;
    let mut comment: Option<(i64, String)> = None;
    let mut types = Vec::new();
    let mut literals = Vec::new();

    let rdfs_label = store.lookup(&format!("{}label", namespace::RDFS))?;
    let rdfs_comment = store.lookup(&format!("{}comment", namespace::RDFS))?;
    let rdf_type = store.lookup(namespace::RDF_TYPE)?;

    for fact in &facts {
        if Some(fact.attribute) == rdfs_label {
            if let Value::Str(s) = &fact.value
                && label.as_ref().is_none_or(|(t, _)| fact.tx >= *t)
            {
                label = Some((fact.tx, s.clone()));
            }
        } else if Some(fact.attribute) == rdfs_comment {
            if let Value::Str(s) = &fact.value
                && comment.as_ref().is_none_or(|(t, _)| fact.tx >= *t)
            {
                comment = Some((fact.tx, s.clone()));
            }
        } else if Some(fact.attribute) == rdf_type {
            if let Value::Ref(type_id) = &fact.value
                && let Ok(iri) = store.resolve(*type_id)
            {
                // Use the local name after the last / or #
                let local = iri
                    .rsplit_once('/')
                    .or_else(|| iri.rsplit_once('#'))
                    .map_or(iri.as_str(), |(_, local)| local);
                types.push(local.to_string());
            }
        } else {
            match &fact.value {
                Value::Str(s) => literals.push(s.clone()),
                // Embed the LEXICAL form only — the tag/datatype is metadata,
                // not text a human wrote.
                Value::Lang { lexical, .. } | Value::Typed { lexical, .. } => {
                    literals.push(lexical.clone());
                }
                Value::Int(n) => literals.push(n.to_string()),
                Value::Float(f) => literals.push(f.to_string()),
                Value::Bool(b) => literals.push(b.to_string()),
                Value::Ref(_) | Value::Bytes(_) => {}
            }
        }
    }

    let mut parts = Vec::new();
    if let Some((_, l)) = label {
        parts.push(l);
    }
    if let Some((_, c)) = comment {
        parts.push(c);
    }
    if !types.is_empty() {
        parts.push(format!("type: {}", types.join(", ")));
    }
    for lit in literals {
        parts.push(lit);
    }

    Ok(parts.join(". "))
}

/// Collect unique entity IDs touched in a set of datums.
pub(crate) fn touched_entity_ids(datums: &[Datum]) -> Vec<i64> {
    let mut seen = BTreeSet::new();
    for d in datums {
        seen.insert(d.entity);
    }
    seen.into_iter().collect()
}

/// Embed work collected under the store lock, to be computed OUTSIDE it.
///
/// The ONNX embed of episode content is multi-second CPU work; running it
/// while holding the global store mutex let a sustained write flood starve
/// every reader (the mfg0 incident's write-side driver). The
/// text a deferred embed was built from travels with the work so the apply
/// step can detect staleness: if another transaction changed the entity in
/// the window between collect and apply, the stale vector is SKIPPED — the
/// later writer's own embed pass owns that entity now.
#[derive(Debug)]
pub struct DeferredEmbed {
    /// `(entity_id, text-as-of-collect)` for each entity needing a vector.
    items: Vec<(i64, String)>,
    /// The originating transaction's timestamp; vectors carry it so the
    /// deferred path stamps embeddings exactly as the inline path would.
    timestamp: String,
}

impl DeferredEmbed {
    /// The texts to embed, in item order — feed to
    /// [`EmbeddingProvider::embed_batch`] outside the lock, then hand the
    /// vectors to [`Store::apply_deferred_embed`](crate::Store::apply_deferred_embed).
    #[must_use]
    pub fn texts(&self) -> Vec<&str> {
        self.items.iter().map(|(_, t)| t.as_str()).collect()
    }

    /// True when there is nothing to embed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Fold another batch of work into this one (multiple transactions before
    /// a drain). Later items win on staleness anyway, so order is preserved
    /// but not load-bearing.
    pub(crate) fn merge(&mut self, mut other: DeferredEmbed) {
        self.items.append(&mut other.items);
        self.timestamp = other.timestamp;
    }
}

/// Phase 1 of auto-embed, run UNDER the store lock: close retired embeddings
/// (cheap `SQLite` writes) and build the texts to embed. The expensive
/// `embed_batch` is deliberately NOT here — the caller runs it lock-free and
/// then applies via [`apply_deferred_embed`].
pub(crate) fn collect_embed_work(
    store: &Store,
    entity_ids: &[i64],
    timestamp: &str,
    datums: &[Datum],
) -> Result<DeferredEmbed> {
    // Route vector writes through the configured backend (LanceDB, delegate,
    // or built-in SQLite) so auto-embed works with any vector backend.
    let vs = store.vector_store();

    // Determine which entities had retractions (need close_embedding).
    let retracted: BTreeSet<i64> = datums
        .iter()
        .filter(|d| d.op == Op::Retract)
        .map(|d| d.entity)
        .collect();

    // Close embeddings for entities that had retractions.
    for &eid in &retracted {
        vs.close_embedding(eid, timestamp)?;
    }

    // Build texts for all touched entities.
    let mut items: Vec<(i64, String)> = Vec::new();
    for &eid in entity_ids {
        let text = build_entity_text(store, eid)?;
        if !text.is_empty() {
            // For assertions on entities without prior retractions,
            // close the old embedding before creating a new one.
            if !retracted.contains(&eid) {
                vs.close_embedding(eid, timestamp)?;
            }
            items.push((eid, text));
        }
    }

    Ok(DeferredEmbed {
        items,
        timestamp: timestamp.to_string(),
    })
}

/// Phase 3 of the deferred path, run under a fresh store lock: write the
/// vectors computed outside it. STALENESS CHECK per entity: the current
/// entity text is rebuilt and compared against the text the vector was
/// computed from; on mismatch the vector is skipped — a later transaction
/// touched the entity and its own embed pass (inline or deferred) owns the
/// current state. Writing the stale vector anyway would silently regress the
/// embedding to pre-update text.
///
/// Returns the number of embeddings written.
pub(crate) fn apply_deferred_embed(
    store: &Store,
    work: &DeferredEmbed,
    embeddings: &[Vec<f32>],
) -> Result<usize> {
    let vs = store.vector_store();
    let mut written = 0;
    for ((eid, text), emb) in work.items.iter().zip(embeddings.iter()) {
        let current = build_entity_text(store, *eid)?;
        if current != *text {
            continue; // stale — a newer writer owns this entity's embedding
        }
        vs.embed_entity(*eid, text, emb, &work.timestamp)?;
        written += 1;
    }
    Ok(written)
}

/// Auto-embed entities after a transaction (the INLINE path — collect, embed
/// and write all under the caller's lock; the CLI and library default).
///
/// For each entity:
/// 1. Close any existing embedding (temporal retirement)
/// 2. Build entity text from current facts
/// 3. If text is non-empty, generate and store new embedding
///
/// Entities are processed in batches of `batch_size` for efficiency.
/// Returns the number of entities embedded.
pub(crate) fn auto_embed_entities(
    store: &Store,
    provider: &Arc<dyn EmbeddingProvider>,
    entity_ids: &[i64],
    timestamp: &str,
    batch_size: usize,
    datums: &[Datum],
) -> Result<usize> {
    let work = collect_embed_work(store, entity_ids, timestamp, datums)?;
    let vs = store.vector_store();

    let batch_sz = if batch_size == 0 { 32 } else { batch_size };
    let mut embedded = 0;

    for chunk in work.items.chunks(batch_sz) {
        let texts: Vec<&str> = chunk.iter().map(|(_, t)| t.as_str()).collect();
        let embeddings = provider.embed_batch(&texts)?;

        for ((eid, text), emb) in chunk.iter().zip(embeddings.iter()) {
            vs.embed_entity(*eid, text, emb, timestamp)?;
            embedded += 1;
        }
    }

    Ok(embedded)
}

#[cfg(test)]
mod tests;
