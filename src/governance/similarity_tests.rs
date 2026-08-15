//! Similarity-method tests. Size-exempt (`*tests.rs`).

use std::sync::Arc;

use super::*;

/// A provider that returns the same vector for every text.
struct FixedVec(Vec<f32>);

impl EmbeddingProvider for FixedVec {
    fn embed_text(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(self.0.clone())
    }
    fn dimension(&self) -> usize {
        self.0.len()
    }
}

/// A provider that returns fewer vectors than texts — the misalignment case.
struct MisCounting;

impl EmbeddingProvider for MisCounting {
    fn embed_text(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![1.0])
    }
    fn embed_batch(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        Ok(vec![vec![1.0]])
    }
    fn dimension(&self) -> usize {
        1
    }
}

#[test]
fn no_provider_means_no_method_not_an_error() {
    // The degraded state the advisory features build on: absent, not broken.
    let store = Store::open_in_memory().unwrap();
    assert!(EmbeddingCosine::from_store(&store).is_none());
}

#[test]
fn the_identity_names_the_configured_model() {
    // The falsifiability contract: the identity must say which model made the
    // score, from the same source `quipu pack` stamps into its manifest.
    let mut store = Store::open_in_memory().unwrap();
    store.embedding_config_mut().model_path =
        Some("models/all-MiniLM-L6-v2/onnx/model.onnx".into());
    store.set_embedding_provider(Arc::new(FixedVec(vec![1.0, 0.0])));
    let method = EmbeddingCosine::from_store(&store).unwrap();
    assert_eq!(method.identity(), "embedding:model.onnx");
}

#[test]
fn an_unconfigured_model_is_named_as_unnamed_rather_than_guessed() {
    // A provider attached without a model_path (tests, delegates) gets an
    // identity that SAYS the model is unidentified — an honest "unnamed" is
    // checkable, a plausible invented name is not.
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(FixedVec(vec![1.0, 0.0, 0.0])));
    let method = EmbeddingCosine::from_store(&store).unwrap();
    let id = method.identity();
    assert!(
        id.contains("unnamed") && id.contains("dim=3"),
        "the identity must admit what it does not know: {id}"
    );
}

#[test]
fn a_zero_vector_scores_zero_not_nan() {
    // The degenerate no-signal case must be the no-signal ANSWER: 0.0 filters
    // out of every advisory path, while NaN would poison sort order and land
    // an unreadable number on record.
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(FixedVec(vec![0.0, 0.0])));
    let method = EmbeddingCosine::from_store(&store).unwrap();
    let score = method.score("a", "b").unwrap();
    assert!(
        score == 0.0,
        "zero-norm vectors must score 0.0, got {score}"
    );
}

#[test]
fn a_miscounting_provider_is_refused_rather_than_misaligned() {
    // One vector for two texts: zip-truncating would silently pair scores
    // with the wrong subjects, which is worse than no scores at all.
    let mut store = Store::open_in_memory().unwrap();
    store.set_embedding_provider(Arc::new(MisCounting));
    let method = EmbeddingCosine::from_store(&store).unwrap();
    let err = method.score("a", "b");
    assert!(
        err.is_err(),
        "a provider returning the wrong count must error, got {err:?}"
    );
}
