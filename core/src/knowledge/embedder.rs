use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::error::{CoreError, Result};

/// Output dimension of multilingual-e5-small.
pub const EMBEDDING_DIM: usize = 384;

/// Wraps `fastembed::TextEmbedding` with the E5 prefix convention.
///
/// Stored chunks are prefixed with `passage: ` before embedding.
/// Query strings are prefixed with `query: ` before embedding.
/// Callers pass raw text — this struct handles the prefixes.
///
/// `TextEmbedding::embed` requires `&mut self`, but downstream callers
/// (indexer, retriever) share this via `Arc<Embedder>`. We wrap the model
/// in a `Mutex` so the shared reference can still invoke `embed` safely.
pub struct Embedder {
    model: Mutex<TextEmbedding>,
}

impl Embedder {
    /// Create an embedder using multilingual-e5-small.
    /// On first use, the model (~120MB) downloads automatically from HuggingFace
    /// and caches under `$HOME/.cache/fastembed/`.
    pub fn new() -> Result<Self> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small)
                .with_show_download_progress(false),
        )
        .map_err(|e| CoreError::Embedding(format!("failed to init embedding model: {}", e)))?;

        Ok(Self {
            model: Mutex::new(model),
        })
    }

    /// Embed a batch of document chunks. Caller should pass raw text — this function adds
    /// the `passage:` prefix required by E5.
    pub fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let prefixed: Vec<String> = texts.iter().map(|t| format!("passage: {}", t)).collect();
        let refs: Vec<&str> = prefixed.iter().map(|s| s.as_str()).collect();
        let mut guard = self
            .model
            .lock()
            .map_err(|e| CoreError::Embedding(format!("embedder mutex poisoned: {}", e)))?;
        guard
            .embed(refs, None)
            .map_err(|e| CoreError::Embedding(format!("embed_passages failed: {}", e)))
    }

    /// Embed a single query string. Adds the `query:` prefix required by E5.
    pub fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let prefixed = format!("query: {}", text);
        let mut guard = self
            .model
            .lock()
            .map_err(|e| CoreError::Embedding(format!("embedder mutex poisoned: {}", e)))?;
        let mut out = guard
            .embed(vec![prefixed.as_str()], None)
            .map_err(|e| CoreError::Embedding(format!("embed_query failed: {}", e)))?;
        out.pop()
            .ok_or_else(|| CoreError::Embedding("empty embedding result".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These tests require downloading the model (~120MB). They're marked #[ignore]
    // so they don't run by default. Run explicitly with:
    //   cargo test -p messagehub-core knowledge::embedder -- --ignored

    #[test]
    #[ignore = "requires model download — ~120MB"]
    fn test_embed_passages_dims() {
        let embedder = Embedder::new().unwrap();
        let result = embedder
            .embed_passages(&["hello world", "another passage"])
            .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), EMBEDDING_DIM);
        assert_eq!(result[1].len(), EMBEDDING_DIM);
    }

    #[test]
    #[ignore = "requires model download — ~120MB"]
    fn test_embed_query_dims() {
        let embedder = Embedder::new().unwrap();
        let result = embedder.embed_query("test query").unwrap();
        assert_eq!(result.len(), EMBEDDING_DIM);
    }

    #[test]
    #[ignore = "requires model download — ~120MB"]
    fn test_similar_texts_have_close_embeddings() {
        let embedder = Embedder::new().unwrap();
        let vecs = embedder
            .embed_passages(&[
                "The dog chased the cat across the yard",
                "A canine pursued a feline through the garden",
                "Quantum mechanics and the Schrodinger equation",
            ])
            .unwrap();

        let sim_ab = cosine(&vecs[0], &vecs[1]);
        let sim_ac = cosine(&vecs[0], &vecs[2]);
        assert!(
            sim_ab > sim_ac,
            "paraphrase should be more similar than unrelated text"
        );
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (mag_a * mag_b)
    }
}
