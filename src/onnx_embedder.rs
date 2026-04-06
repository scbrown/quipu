//! ONNX-based embedding provider for standalone quipu-server.
//!
//! Loads an ONNX sentence-transformer model (e.g. all-MiniLM-L6-v2) and
//! provides the [`EmbeddingProvider`] trait for vector search support.

use std::path::Path;
use std::sync::Mutex;

use ndarray::Array2;
use ort::session::Session;
use ort::value::Tensor;
use tokenizers::Tokenizer;

use crate::embedding::EmbeddingProvider;
use crate::error::Result;

/// ONNX Runtime embedding provider.
pub struct OnnxEmbeddingProvider {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    dim: usize,
}

impl OnnxEmbeddingProvider {
    /// Load an ONNX model and tokenizer from disk.
    ///
    /// # Arguments
    /// * `model_path` — Path to the ONNX model file (model.onnx)
    /// * `tokenizer_path` — Path to the tokenizer.json file
    /// * `dim` — Expected embedding dimension (e.g. 384)
    pub fn load(model_path: &Path, tokenizer_path: &Path, dim: usize) -> Result<Self> {
        let session = Session::builder()
            .and_then(|b| b.with_intra_threads(1))
            .and_then(|b| b.commit_from_file(model_path))
            .map_err(|e| crate::Error::Store(format!("ONNX session init failed: {e}")))?;

        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| crate::Error::Store(format!("Tokenizer load failed: {e}")))?;

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            dim,
        })
    }

    /// Embed a batch of texts, returning vectors and performing mean pooling.
    fn embed_batch_inner(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| crate::Error::Store(format!("Tokenization failed: {e}")))?;

        let batch_size = encodings.len();
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);

        // Build padded input tensors.
        let mut input_ids = vec![0i64; batch_size * max_len];
        let mut attention_mask = vec![0i64; batch_size * max_len];
        let mut token_type_ids = vec![0i64; batch_size * max_len];

        for (i, enc) in encodings.iter().enumerate() {
            for (j, &id) in enc.get_ids().iter().enumerate() {
                input_ids[i * max_len + j] = i64::from(id);
                attention_mask[i * max_len + j] = 1;
            }
            for (j, &tt) in enc.get_type_ids().iter().enumerate() {
                token_type_ids[i * max_len + j] = i64::from(tt);
            }
        }

        let shape = [batch_size, max_len];

        let ids_tensor = Tensor::from_array((shape, input_ids.into_boxed_slice()))
            .map_err(|e| crate::Error::Store(format!("Tensor creation failed: {e}")))?;
        let mask_tensor = Tensor::from_array((shape, attention_mask.clone().into_boxed_slice()))
            .map_err(|e| crate::Error::Store(format!("Tensor creation failed: {e}")))?;
        let type_tensor = Tensor::from_array((shape, token_type_ids.into_boxed_slice()))
            .map_err(|e| crate::Error::Store(format!("Tensor creation failed: {e}")))?;

        let mut session = self.session.lock().unwrap();
        let outputs = session
            .run(ort::inputs![ids_tensor, mask_tensor, type_tensor])
            .map_err(|e| crate::Error::Store(format!("ONNX inference failed: {e}")))?;

        // Extract the last_hidden_state output (shape: [batch, seq_len, dim]).
        // Extract the first output tensor (last_hidden_state).
        let output_name = outputs
            .keys()
            .next()
            .ok_or_else(|| crate::Error::Store("No output from ONNX model".into()))?
            .to_string();
        let output = outputs
            .get(&*output_name)
            .ok_or_else(|| crate::Error::Store("Failed to get output tensor".into()))?;

        let (_shape, raw_data) = output
            .try_extract_tensor::<f32>()
            .map_err(|e| crate::Error::Store(format!("Output extraction failed: {e}")))?;
        let raw: Vec<f32> = raw_data.to_vec();

        // Mean pooling with attention mask.
        let hidden = Array2::from_shape_vec((batch_size * max_len, self.dim), raw)
            .map_err(|e| crate::Error::Store(format!("Array reshape failed: {e}")))?;

        let mut results = Vec::with_capacity(batch_size);
        for i in 0..batch_size {
            let mut sum = vec![0.0f32; self.dim];
            let mut count = 0.0f32;
            for j in 0..max_len {
                if attention_mask[i * max_len + j] == 1 {
                    let row = hidden.row(i * max_len + j);
                    for (k, val) in row.iter().enumerate() {
                        sum[k] += val;
                    }
                    count += 1.0;
                }
            }
            if count > 0.0 {
                for val in &mut sum {
                    *val /= count;
                }
            }
            // L2 normalize.
            let norm: f32 = sum.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for val in &mut sum {
                    *val /= norm;
                }
            }
            results.push(sum);
        }

        Ok(results)
    }
}

impl EmbeddingProvider for OnnxEmbeddingProvider {
    fn embed_text(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed_batch_inner(&[text])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| crate::Error::Store("Empty embedding result".into()))
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.embed_batch_inner(texts)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}
