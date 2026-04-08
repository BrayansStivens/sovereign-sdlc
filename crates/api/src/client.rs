use anyhow::Result;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;

/// Token telemetry from Ollama response
#[derive(Debug, Clone, Default)]
pub struct GenMetrics {
    pub response: String,
    pub eval_count: u64,
    pub prompt_eval_count: u64,
    pub total_duration_ns: u64,
}

impl GenMetrics {
    pub fn tokens_per_sec(&self) -> f64 {
        if self.total_duration_ns == 0 { return 0.0; }
        let secs = self.total_duration_ns as f64 / 1_000_000_000.0;
        self.eval_count as f64 / secs
    }

    pub fn total_secs(&self) -> f64 {
        self.total_duration_ns as f64 / 1_000_000_000.0
    }

    pub fn total_tokens(&self) -> u64 {
        self.eval_count + self.prompt_eval_count
    }

    /// Summary line: [+] 3.2s | 847 tokens | >_ 264.7 tok/s
    pub fn summary(&self) -> String {
        format!(
            "[+] {:.1}s | {} tokens | >_ {:.1} tok/s",
            self.total_secs(),
            self.total_tokens(),
            self.tokens_per_sec(),
        )
    }
}

/// Wrapper around ollama-rs with Sovereign-specific defaults
pub struct OllamaClient {
    inner: Ollama,
}

impl OllamaClient {
    pub fn new() -> Self {
        Self { inner: Ollama::default() }
    }

    pub fn with_url(host: &str, port: u16) -> Self {
        Self { inner: Ollama::new(host, port) }
    }

    /// Generate with full metrics (tokens, duration)
    pub async fn generate_with_metrics(&self, model: &str, prompt: &str) -> Result<GenMetrics> {
        let request = GenerationRequest::new(model.to_string(), prompt.to_string());
        let response = self.inner.generate(request).await?;
        Ok(GenMetrics {
            response: response.response,
            eval_count: response.eval_count.unwrap_or(0) as u64,
            prompt_eval_count: response.prompt_eval_count.unwrap_or(0) as u64,
            total_duration_ns: response.total_duration.unwrap_or(0),
        })
    }

    /// Simple generate (just text, no metrics)
    pub async fn generate(&self, model: &str, prompt: &str) -> Result<String> {
        let m = self.generate_with_metrics(model, prompt).await?;
        Ok(m.response)
    }

    /// Generate embeddings (768 dims for nomic-embed-text)
    pub async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>> {
        let request = GenerateEmbeddingsRequest::new(model.to_string(), text.into());
        let response = self.inner.generate_embeddings(request).await?;
        Ok(response.embeddings.into_iter().next().unwrap_or_default())
    }

    /// Batch embed
    pub async fn embed_batch(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(model, text).await?);
        }
        Ok(results)
    }

    /// List locally available models
    pub async fn list_models(&self) -> Result<Vec<String>> {
        let models = self.inner.list_local_models().await?;
        Ok(models.into_iter().map(|m| m.name).collect())
    }

    pub fn inner(&self) -> &Ollama {
        &self.inner
    }
}
