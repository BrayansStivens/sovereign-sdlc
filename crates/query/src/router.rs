use anyhow::Result;
use ollama_rs::{generation::completion::request::GenerationRequest, Ollama};

/// Task categories for smart routing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCategory {
    Code,
    Logic,
    Chat,
}

impl std::fmt::Display for TaskCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaskCategory::Code => write!(f, "CODE"),
            TaskCategory::Logic => write!(f, "LOGIC"),
            TaskCategory::Chat => write!(f, "CHAT"),
        }
    }
}

impl TaskCategory {
    /// Map each category to its optimal model
    pub fn model(&self) -> &'static str {
        match self {
            TaskCategory::Code => "qwen2.5-coder:14b-q8_0",
            TaskCategory::Logic => "deepseek-r1:14b",
            TaskCategory::Chat => "mistral-small-4",
        }
    }
}

/// The router model used for fast classification
const ROUTER_MODEL: &str = "qwen2.5:7b";

const CLASSIFICATION_PROMPT: &str = r#"Classify the following user prompt into exactly one category. Reply with ONLY the category name, nothing else.

Categories:
- CODE: Writing, debugging, reviewing, or explaining code. Anything about programming languages, APIs, scripts, or software.
- LOGIC: Math, reasoning, puzzles, algorithms, analysis, planning, or problem-solving that is NOT about writing code.
- CHAT: General conversation, questions, creative writing, summaries, translations, or anything else.

User prompt: "#;

pub struct SmartRouter {
    ollama: Ollama,
    /// Override: if set, skip classification and always use this model
    pub force_model: Option<String>,
}

impl SmartRouter {
    pub fn new(ollama: Ollama) -> Self {
        Self {
            ollama,
            force_model: None,
        }
    }

    /// Classify a prompt using the lightweight router model
    pub async fn classify(&self, prompt: &str) -> Result<TaskCategory> {
        // Fast heuristic check first — avoids an LLM call for obvious cases
        if let Some(cat) = Self::heuristic_classify(prompt) {
            return Ok(cat);
        }

        let full_prompt = format!("{CLASSIFICATION_PROMPT}{prompt}");

        let request = GenerationRequest::new(ROUTER_MODEL.to_string(), full_prompt);
        let response = self.ollama.generate(request).await?;

        let answer = response.response.trim().to_uppercase();

        Ok(if answer.contains("CODE") {
            TaskCategory::Code
        } else if answer.contains("LOGIC") {
            TaskCategory::Logic
        } else {
            TaskCategory::Chat
        })
    }

    /// Determine which model to use for a given prompt
    pub async fn route(&self, prompt: &str) -> Result<(TaskCategory, String)> {
        if let Some(ref model) = self.force_model {
            // When user explicitly set a model with /model, use it for everything
            return Ok((TaskCategory::Chat, model.clone()));
        }

        let category = self.classify(prompt).await?;
        Ok((category, category.model().to_string()))
    }

    /// Fast keyword-based heuristic to avoid LLM call for obvious prompts
    fn heuristic_classify(prompt: &str) -> Option<TaskCategory> {
        let lower = prompt.to_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();

        // Strong code signals — patterns that are unambiguous
        let code_phrases = [
            "```",
            "stack trace",
            "import {",
            "from import",
            "#include",
            "require(",
        ];

        for phrase in &code_phrases {
            if lower.contains(phrase) {
                return Some(TaskCategory::Code);
            }
        }

        // Word-level code keywords (must appear as whole words)
        let code_words = [
            "function", "fn", "def", "class", "impl", "compile", "debug",
            "refactor", "bug", "error", "exception", "syntax", "api",
            "endpoint", "database", "query", "sql", "git", "docker",
            "deploy", "unittest", "cargo", "npm", "pip", "rust", "python",
            "javascript", "typescript",
        ];

        for kw in &code_words {
            if words.iter().any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *kw) {
                return Some(TaskCategory::Code);
            }
        }

        // Strong logic signals (word-level)
        let logic_words = [
            "calculate", "prove", "solve", "equation", "algorithm",
            "optimize", "probability", "theorem", "induction", "deduce",
            "mathematical",
        ];

        for kw in &logic_words {
            if words.iter().any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *kw) {
                return Some(TaskCategory::Logic);
            }
        }

        let logic_phrases = ["analyze the complexity", "big-o"];
        for phrase in &logic_phrases {
            if lower.contains(phrase) {
                return Some(TaskCategory::Logic);
            }
        }

        // No strong signal — let the LLM decide
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heuristic_code() {
        assert_eq!(
            SmartRouter::heuristic_classify("write a function to sort a list"),
            Some(TaskCategory::Code)
        );
        assert_eq!(
            SmartRouter::heuristic_classify("debug this error in my Rust code"),
            Some(TaskCategory::Code)
        );
    }

    #[test]
    fn test_heuristic_logic() {
        assert_eq!(
            SmartRouter::heuristic_classify("prove that sqrt(2) is irrational"),
            Some(TaskCategory::Logic)
        );
        assert_eq!(
            SmartRouter::heuristic_classify("calculate the probability of drawing 2 aces"),
            Some(TaskCategory::Logic)
        );
    }

    #[test]
    fn test_heuristic_ambiguous() {
        // Should return None — needs LLM
        assert_eq!(
            SmartRouter::heuristic_classify("what is the capital of France?"),
            None
        );
        assert_eq!(
            SmartRouter::heuristic_classify("tell me a story about dragons"),
            None
        );
    }

    #[test]
    fn test_category_model_mapping() {
        assert_eq!(TaskCategory::Code.model(), "qwen2.5-coder:14b-q8_0");
        assert_eq!(TaskCategory::Logic.model(), "deepseek-r1:14b");
        assert_eq!(TaskCategory::Chat.model(), "mistral-small-4");
    }
}
