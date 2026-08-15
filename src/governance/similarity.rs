//! Text similarity as ADVISORY grounding — the method half of the design's
//! "similarity grounds or advises, never hard-denies" rule
//! (docs/design/semantic-grounded-edit-policies.md, "Further applications").
//!
//! A similarity score is a fact about a COMPARISON, not about the world: it
//! means nothing outside the method and model that produced it (the
//! trust-chain rule, applied to embeddings). So the one interface here couples
//! the score to its identity — a caller cannot obtain a number without also
//! obtaining the string that makes the number falsifiable, and everything the
//! governance modules record alongside a score comes from [`TextSimilarity::identity`],
//! never from a constant that could drift from the method actually run.
//!
//! There is deliberately NO fallback method. When the store has no embedding
//! provider, [`EmbeddingCosine::from_store`] returns `None` and the advisory
//! features degrade to ABSENT — never to a cheaper heuristic silently wearing
//! the embedding tier's label, and never to a fabricated score. A future
//! deterministic method (token overlap, say) is admissible only as its own
//! honestly-named implementation of this trait.

use std::sync::Arc;

use crate::embedding::EmbeddingProvider;
use crate::error::{Error, Result};
use crate::store::Store;
use crate::vector::cosine_similarity;

/// A method that scores how near two texts are AND names itself.
///
/// Injectable by design: the governance tests drive the three-outcome and
/// precedent logic with fixed-score stubs, so the logic is provable without a
/// live ONNX model on the test host.
pub trait TextSimilarity {
    /// The method/model identity that must ride every recorded score —
    /// re-running the same identity over the same texts must reproduce the
    /// score, which is what makes a similarity claim falsifiable rather than
    /// merely asserted.
    fn identity(&self) -> String;

    /// Similarity of `a` to `b`. Cosine-flavoured: `1.0` is identical
    /// direction, `0.0` is unrelated (and the degenerate no-signal answer —
    /// see [`cosine_similarity`]'s zero-norm guard).
    fn score(&self, a: &str, b: &str) -> Result<f64>;

    /// Score `text` against each of `others`, in order. The default loops
    /// over [`TextSimilarity::score`]; batching backends override so `text`
    /// is embedded once, not once per comparison.
    fn score_many(&self, text: &str, others: &[String]) -> Result<Vec<f64>> {
        others.iter().map(|o| self.score(text, o)).collect()
    }
}

/// Cosine similarity over the store's configured [`EmbeddingProvider`] — the
/// only method quipu ships, because it is the only one the design gives a tier
/// ("embedding": reproducible but approximate).
pub struct EmbeddingCosine {
    provider: Arc<dyn EmbeddingProvider>,
    identity: String,
}

impl EmbeddingCosine {
    /// The store's embedding path, or `None` when no provider is configured.
    ///
    /// `None` is the honest degraded state, not an error: the advisory
    /// features that consume this attach nothing rather than failing, and a
    /// caller who needs the provider outright already has
    /// [`crate::embedding::NO_PROVIDER_HELP`] for the loud path.
    pub fn from_store(store: &Store) -> Option<Self> {
        let provider = store.embedding_provider()?;
        // The model identity comes from the CONFIGURED model file, the same
        // source `quipu pack` stamps into its manifest. A provider attached
        // without a configured path (tests, delegates) gets an identity that
        // SAYS the model is unnamed rather than inventing a name — an honest
        // "unidentified" is checkable, a plausible guess is not.
        let identity = match store
            .embedding_config()
            .model_path
            .as_ref()
            .and_then(|p| p.file_name())
        {
            Some(model) => format!("embedding:{}", model.to_string_lossy()),
            None => format!("embedding:unnamed-model(dim={})", provider.dimension()),
        };
        Some(Self { provider, identity })
    }
}

impl TextSimilarity for EmbeddingCosine {
    fn identity(&self) -> String {
        self.identity.clone()
    }

    fn score(&self, a: &str, b: &str) -> Result<f64> {
        self.score_many(a, std::slice::from_ref(&b.to_string()))
            .map(|scores| scores[0])
    }

    fn score_many(&self, text: &str, others: &[String]) -> Result<Vec<f64>> {
        let mut texts: Vec<&str> = Vec::with_capacity(others.len() + 1);
        texts.push(text);
        texts.extend(others.iter().map(String::as_str));
        let embeddings = self.provider.embed_batch(&texts)?;
        // A provider returning the wrong count would misalign every score
        // with its text — refuse rather than zip-truncate into wrong answers.
        if embeddings.len() != texts.len() {
            return Err(Error::Store(format!(
                "embedding provider returned {} vectors for {} texts — \
                 cannot align scores with their subjects",
                embeddings.len(),
                texts.len()
            )));
        }
        Ok(embeddings[1..]
            .iter()
            .map(|e| cosine_similarity(&embeddings[0], e))
            .collect())
    }
}

#[cfg(test)]
#[path = "similarity_tests.rs"]
mod tests;
