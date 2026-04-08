use anyhow::Result;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;

/// Wrapper around ollama-rs with Sovereign-specific defaults
pub struct OllamaClient {
    inner: Ollama,
}

impl OllamaClient {
    pub fn new() -> Self {
        Self {
            inner: Ollama::default(),
        }
    }

    pub fn with_url(host: &str, port: u16) -> Self {
        Self {
            inner: Ollama::new(host, port),
        }
    }

    /// Generate a response from a model
    pub async fn generate(&self, model: &str, prompt: &str) -> Result<String> {
        let request = GenerationRequest::new(model.to_string(), prompt.to_string());
        let response = self.inner.generate(request).await?;
        Ok(response.response)
    }

    /// Generate embeddings for a text using nomic-embed-text (768 dims)
    /// Returns f32 vector for efficient storage and similarity computation
    pub async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>> {
        let request = GenerateEmbeddingsRequest::new(
            model.to_string(),
            text.into(),
        );
        let response = self.inner.generate_embeddings(request).await?;
        // ollama-rs returns Vec<Vec<f32>> — take the first embedding
        Ok(response.embeddings.into_iter().next().unwrap_or_default())
    }

    /// Batch embed multiple texts (sequential — hardware-gated concurrency is at caller level)
    pub async fn embed_batch(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            let embedding = self.embed(model, text).await?;
            results.push(embedding);
        }
        Ok(results)
    }

    /// List locally available models
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let models = self.inner.list_local_models().await?;
        Ok(models.into_iter().map(|m| m.name).collect())
    }

    /// Raw access to the inner Ollama client
    pub fn inner(&self) -> &Ollama {
        &self.inner
    }
}
