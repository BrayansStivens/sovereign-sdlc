//! Ollama API client wrapper with streaming chat support.

use anyhow::Result;
use ollama_rs::generation::chat::request::ChatMessageRequest;
use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse, MessageRole};
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;
use sovereign_core::{ConversationMessage, ConversationRole};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

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
        if self.total_duration_ns == 0 {
            return 0.0;
        }
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

/// Chunks emitted by the streaming chat
#[derive(Debug, Clone)]
pub enum StreamChunk {
    /// Incremental text token
    Delta(String),
    /// Stream completed with final metrics
    Done(GenMetrics),
    /// Error during streaming
    Error(String),
}

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

    // ── Legacy generate API (kept for compatibility) ──

    /// Generate with full metrics (tokens, duration)
    pub async fn generate_with_metrics(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<GenMetrics> {
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

    // ── Chat API (new — multi-turn with streaming) ──

    /// Non-streaming chat — sends messages and returns full response + metrics.
    /// Used internally for context compression, routing, etc.
    pub async fn chat(
        &self,
        model: &str,
        messages: &[ConversationMessage],
    ) -> Result<(String, GenMetrics)> {
        let chat_messages = conv_to_ollama(messages);
        let request = ChatMessageRequest::new(model.to_string(), chat_messages);

        let response = self.inner.send_chat_messages(request).await?;
        let metrics = extract_metrics(&response);

        Ok((response.message.content, metrics))
    }

    /// Streaming chat — sends messages and returns a channel that receives
    /// token deltas as they arrive. The final chunk is `StreamChunk::Done`
    /// with metrics.
    pub async fn chat_stream(
        &self,
        model: &str,
        messages: &[ConversationMessage],
    ) -> Result<mpsc::UnboundedReceiver<StreamChunk>> {
        let chat_messages = conv_to_ollama(messages);
        let request = ChatMessageRequest::new(model.to_string(), chat_messages);

        let mut stream = self.inner.send_chat_messages_stream(request).await?;
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut full_response = String::new();
            let mut final_metrics = GenMetrics::default();

            while let Some(result) = stream.next().await {
                match result {
                    Ok(chunk) => {
                        let delta = &chunk.message.content;
                        if !delta.is_empty() {
                            full_response.push_str(delta);
                            if tx.send(StreamChunk::Delta(delta.clone())).is_err() {
                                return; // Receiver dropped (cancelled)
                            }
                        }

                        if chunk.done {
                            if let Some(data) = chunk.final_data {
                                final_metrics = GenMetrics {
                                    response: full_response.clone(),
                                    eval_count: data.eval_count as u64,
                                    prompt_eval_count: data.prompt_eval_count as u64,
                                    total_duration_ns: data.total_duration,
                                };
                            } else {
                                final_metrics.response = full_response.clone();
                            }
                        }
                    }
                    Err(_) => {
                        let _ = tx.send(StreamChunk::Error(
                            "Stream chunk deserialization error".into(),
                        ));
                    }
                }
            }

            let _ = tx.send(StreamChunk::Done(final_metrics));
        });

        Ok(rx)
    }

    // ── Embeddings ──

    /// Generate embeddings (768 dims for nomic-embed-text)
    pub async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>> {
        let request = GenerateEmbeddingsRequest::new(model.to_string(), text.into());
        let response = self.inner.generate_embeddings(request).await?;
        Ok(response.embeddings.into_iter().next().unwrap_or_default())
    }

    /// Batch embed
    pub async fn embed_batch(
        &self,
        model: &str,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>> {
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

// ── Conversion helpers ──

/// Convert our ConversationMessage to ollama-rs ChatMessage
fn conv_to_ollama(messages: &[ConversationMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                ConversationRole::System => MessageRole::System,
                ConversationRole::User => MessageRole::User,
                ConversationRole::Assistant => MessageRole::Assistant,
                ConversationRole::Tool => MessageRole::User, // Fallback: wrap tool as user
            };
            ChatMessage::new(role, m.content.clone())
        })
        .collect()
}

/// Extract metrics from a non-streaming ChatMessageResponse
fn extract_metrics(response: &ChatMessageResponse) -> GenMetrics {
    match &response.final_data {
        Some(data) => GenMetrics {
            response: response.message.content.clone(),
            eval_count: data.eval_count as u64,
            prompt_eval_count: data.prompt_eval_count as u64,
            total_duration_ns: data.total_duration,
        },
        None => GenMetrics {
            response: response.message.content.clone(),
            ..Default::default()
        },
    }
}
