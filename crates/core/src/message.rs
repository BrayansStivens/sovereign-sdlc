//! Conversation message types shared across crates.
//!
//! These types are the lingua franca between agent loop, TUI, and API client.
//! They map 1:1 to ollama-rs ChatMessage for API calls.

use serde::{Deserialize, Serialize};

/// A message in the conversation history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: ConversationRole,
    pub content: String,
}

/// Message roles matching Ollama's chat API
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationRole {
    System,
    User,
    Assistant,
    /// Tool result — maps to ollama-rs MessageRole::Tool
    Tool,
}

impl ConversationMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: ConversationRole::System, content: content.into() }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self { role: ConversationRole::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: ConversationRole::Assistant, content: content.into() }
    }

    pub fn tool(content: impl Into<String>) -> Self {
        Self { role: ConversationRole::Tool, content: content.into() }
    }
}

impl std::fmt::Display for ConversationRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => write!(f, "system"),
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
            Self::Tool => write!(f, "tool"),
        }
    }
}
