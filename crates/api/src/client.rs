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

    /// Chat with native tool calling — Ollama returns structured tool_calls
    /// instead of relying on text-based ```tool parsing.
    /// Uses reqwest directly for full control over the tools JSON.
    pub async fn chat_with_native_tools(
        &self,
        model: &str,
        messages: &[ConversationMessage],
        tools: &[serde_json::Value],
    ) -> Result<NativeToolResponse> {
        let http = reqwest::Client::new();

        let ollama_msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role.to_string(),
                    "content": m.content,
                })
            })
            .collect();

        let body = serde_json::json!({
            "model": model,
            "messages": ollama_msgs,
            "tools": tools,
            "stream": false,
        });

        let resp = http
            .post("http://localhost:11434/api/chat")
            .json(&body)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        let content = resp["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let tool_calls: Vec<NativeToolCall> = resp["message"]["tool_calls"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        let name = tc["function"]["name"].as_str()?.to_string();
                        let arguments = tc["function"]["arguments"].clone();
                        Some(NativeToolCall { name, arguments })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let metrics = GenMetrics {
            response: content.clone(),
            eval_count: resp["eval_count"].as_u64().unwrap_or(0),
            prompt_eval_count: resp["prompt_eval_count"].as_u64().unwrap_or(0),
            total_duration_ns: resp["total_duration"].as_u64().unwrap_or(0),
        };

        Ok(NativeToolResponse {
            content,
            tool_calls,
            metrics,
        })
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

/// Response from native tool calling
#[derive(Debug, Clone)]
pub struct NativeToolResponse {
    pub content: String,
    pub tool_calls: Vec<NativeToolCall>,
    pub metrics: GenMetrics,
}

/// A tool call returned by Ollama's native API
#[derive(Debug, Clone)]
pub struct NativeToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Build Ollama-compatible tool schemas from tool name/description/parameters_hint.
/// This converts our simple tool hints into the function-calling JSON format.
pub fn build_native_tool_schemas(tools: &[(String, String, String)]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|(name, description, params_hint)| {
            // Try to parse parameters_hint as JSON, otherwise build a simple schema
            let parameters = parse_tool_parameters(name, params_hint);
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": parameters,
                }
            })
        })
        .collect()
}

/// Parse a tool's parameters_hint into a proper JSON Schema
fn parse_tool_parameters(name: &str, hint: &str) -> serde_json::Value {
    // Try to parse the hint as JSON to extract parameter names and types
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(hint) {
        if let Some(map) = obj.as_object() {
            let mut properties = serde_json::Map::new();
            let mut required = Vec::new();

            for (key, val) in map {
                let type_str = match val {
                    serde_json::Value::String(_) => "string",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::Bool(_) => "boolean",
                    _ => "string",
                };
                properties.insert(
                    key.clone(),
                    serde_json::json!({
                        "type": type_str,
                        "description": format!("{}", val),
                    }),
                );
                // First param is usually required
                if required.is_empty() {
                    required.push(key.clone());
                }
            }

            return serde_json::json!({
                "type": "object",
                "properties": properties,
                "required": required,
            });
        }
    }

    // Fallback: known tool schemas
    match name {
        "bash" => serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The shell command to run" }
            },
            "required": ["command"]
        }),
        "read" => serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to read" },
                "offset": { "type": "number", "description": "Line number to start from (1-based)" },
                "limit": { "type": "number", "description": "Number of lines to read" }
            },
            "required": ["path"]
        }),
        "glob" => serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern (e.g. **/*.rs)" },
                "path": { "type": "string", "description": "Directory to search in" }
            },
            "required": ["pattern"]
        }),
        "grep" => serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regex pattern to search" },
                "path": { "type": "string", "description": "Directory or file to search" },
                "type": { "type": "string", "description": "File type filter (e.g. rs, js, py)" },
                "output_mode": { "type": "string", "description": "content, files_with_matches, or count" }
            },
            "required": ["pattern"]
        }),
        "edit" => serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to edit" },
                "old_text": { "type": "string", "description": "Text to find" },
                "new_text": { "type": "string", "description": "Replacement text" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences" }
            },
            "required": ["path", "old_text", "new_text"]
        }),
        "write" => serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File path to write" },
                "content": { "type": "string", "description": "File content" }
            },
            "required": ["path", "content"]
        }),
        _ => serde_json::json!({
            "type": "object",
            "properties": {},
        }),
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
