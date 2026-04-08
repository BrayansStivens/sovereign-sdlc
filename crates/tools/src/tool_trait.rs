//! Tool System — Trait, registry, and tool call detection
//!
//! Inspired by claurst's tool architecture but adapted for local Ollama models
//! that don't have native function calling.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

// ────────────────────────────────────────────────────────
// Tool Trait
// ────────────────────────────────────────────────────────

/// Every tool implements this trait
pub trait Tool: Send + Sync {
    /// Tool name (used in prompts and parsing)
    fn name(&self) -> &str;

    /// One-line description for the system prompt
    fn description(&self) -> &str;

    /// JSON schema of expected parameters (for the system prompt)
    fn parameters_hint(&self) -> &str;

    /// Permission level required
    fn permission_level(&self) -> PermissionLevel;

    /// Execute the tool with given input
    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult>;
}

/// Permission levels — higher = more dangerous
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionLevel {
    /// Safe read-only (file read, glob search)
    ReadOnly,
    /// Writes to disk (file edit, create)
    Write,
    /// Executes code (bash, python)
    Execute,
    /// Dangerous (rm, sudo, etc.)
    Dangerous,
}

// ────────────────────────────────────────────────────────
// Tool Result
// ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

impl ToolResult {
    pub fn ok(output: String) -> Self {
        Self { output, is_error: false }
    }
    pub fn error(msg: String) -> Self {
        Self { output: msg, is_error: true }
    }
}

// ────────────────────────────────────────────────────────
// Tool Context
// ────────────────────────────────────────────────────────

/// Context passed to every tool execution
#[derive(Clone)]
pub struct ToolContext {
    pub working_dir: PathBuf,
}

impl ToolContext {
    pub fn new() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

// ────────────────────────────────────────────────────────
// Tool Call (parsed from LLM response)
// ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(alias = "tool")]
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

// ────────────────────────────────────────────────────────
// Tool Registry
// ────────────────────────────────────────────────────────

pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.as_ref())
    }

    pub fn names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name()).collect()
    }

    /// Generate the system prompt section describing all available tools
    pub fn system_prompt(&self) -> String {
        let mut prompt = String::from(
            "# TOOLS\n\n\
             You have tools to interact with the filesystem and run commands.\n\
             To use a tool, output EXACTLY this format:\n\n\
             ```tool\n\
             {\"tool\": \"TOOL_NAME\", \"input\": {\"param\": \"value\"}}\n\
             ```\n\n\
             EXAMPLE — read a file:\n\
             ```tool\n\
             {\"tool\": \"read\", \"input\": {\"path\": \"Cargo.toml\"}}\n\
             ```\n\n\
             EXAMPLE — list files:\n\
             ```tool\n\
             {\"tool\": \"glob\", \"input\": {\"pattern\": \"**/*.rs\"}}\n\
             ```\n\n\
             EXAMPLE — run a command:\n\
             ```tool\n\
             {\"tool\": \"bash\", \"input\": {\"command\": \"ls -la\"}}\n\
             ```\n\n\
             EXAMPLE — search code:\n\
             ```tool\n\
             {\"tool\": \"grep\", \"input\": {\"pattern\": \"fn main\", \"type\": \"rs\"}}\n\
             ```\n\n\
             Available tools:\n\n"
        );

        for tool in &self.tools {
            prompt.push_str(&format!(
                "### {}\n{}\nParameters: {}\n\n",
                tool.name(), tool.description(), tool.parameters_hint(),
            ));
        }

        prompt.push_str(
            "RULES:\n\
             - Use ```tool blocks (NOT ```json) when calling tools.\n\
             - One tool call per response. Wait for the result before calling another.\n\
             - After receiving a tool result, use it to answer the user.\n\
             - If you don't need a tool, just respond normally with text.\n\n"
        );

        prompt
    }

    /// Execute a tool call
    pub fn execute(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult> {
        match self.get(&call.name) {
            Some(tool) => tool.execute(&call.input, ctx),
            None => Ok(ToolResult::error(format!("Unknown tool: {}", call.name))),
        }
    }
}

// ────────────────────────────────────────────────────────
// Tool Call Parser
// ────────────────────────────────────────────────────────

/// Parse a tool call from LLM response text.
/// Looks for ```tool blocks with JSON inside.
/// Also handles common variations from local models.
pub fn parse_tool_call(response: &str) -> Option<(ToolCall, String)> {
    // Try ```tool block first (our standard format)
    if let Some(call) = try_parse_tool_block(response) {
        let text_before = response.split("```tool").next().unwrap_or("").trim().to_string();
        return Some((call, text_before));
    }

    // Try ```json block with tool/input keys
    if let Some(call) = try_parse_json_block(response) {
        let text_before = response.split("```json").next().unwrap_or("").trim().to_string();
        return Some((call, text_before));
    }

    // Try inline JSON { "tool": "...", "input": ... }
    if let Some(call) = try_parse_inline_json(response) {
        return Some((call, String::new()));
    }

    // Try single-key JSON blocks (local models often emit {"bash": {"command": "ls"}})
    if let Some(call) = try_parse_single_key_json(response) {
        let text_before = response.split("```").next().unwrap_or("").trim().to_string();
        return Some((call, text_before));
    }

    None
}

fn try_parse_tool_block(response: &str) -> Option<ToolCall> {
    let start = response.find("```tool")?;
    let content_start = start + 7;
    let end = response[content_start..].find("```")? + content_start;
    let json_str = response[content_start..end].trim();
    serde_json::from_str::<ToolCall>(json_str).ok()
}

fn try_parse_json_block(response: &str) -> Option<ToolCall> {
    let start = response.find("```json")?;
    let content_start = start + 7;
    let end = response[content_start..].find("```")? + content_start;
    let json_str = response[content_start..end].trim();

    // Try direct ToolCall parse
    if let Ok(call) = serde_json::from_str::<ToolCall>(json_str) {
        if !call.name.is_empty() {
            return Some(call);
        }
    }

    // Try as generic object with "tool" key
    let obj: Value = serde_json::from_str(json_str).ok()?;
    let name = obj.get("tool")?.as_str()?.to_string();
    let input = obj.get("input").cloned().unwrap_or(Value::Object(Default::default()));
    Some(ToolCall { name, input })
}

/// Parse single-key JSON blocks that local models often produce.
/// Handles: ```json\n{"glob": "**/*"}\n```  → tool=glob, input={"pattern": "**/*"}
/// Handles: ```json\n{"bash": {"command": "ls"}}\n``` → tool=bash, input={"command": "ls"}
fn try_parse_single_key_json(response: &str) -> Option<ToolCall> {
    // Known tool names
    const KNOWN_TOOLS: &[&str] = &["bash", "read", "glob", "grep", "edit", "write"];

    // Look for any code block with JSON
    for marker in &["```json", "```tool", "```"] {
        if let Some(start) = response.find(marker) {
            let content_start = start + marker.len();
            if let Some(end) = response[content_start..].find("```") {
                let json_str = response[content_start..content_start + end].trim();
                if let Ok(obj) = serde_json::from_str::<Value>(json_str) {
                    if let Some(map) = obj.as_object() {
                        // Single key that matches a tool name
                        if map.len() == 1 {
                            if let Some((key, val)) = map.iter().next() {
                                if KNOWN_TOOLS.contains(&key.as_str()) {
                                    // Value is the input — wrap it properly
                                    let input = match val {
                                        Value::Object(_) => val.clone(),
                                        Value::String(s) => {
                                            // Guess the primary parameter
                                            let param = match key.as_str() {
                                                "bash" => "command",
                                                "read" => "path",
                                                "glob" => "pattern",
                                                "grep" => "pattern",
                                                "edit" => "path",
                                                "write" => "path",
                                                _ => "input",
                                            };
                                            serde_json::json!({ param: s })
                                        }
                                        _ => serde_json::json!({ "input": val }),
                                    };
                                    return Some(ToolCall {
                                        name: key.clone(),
                                        input,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn try_parse_inline_json(response: &str) -> Option<ToolCall> {
    // Find JSON-like pattern with "tool" key
    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{') && trimmed.contains("\"tool\"") {
            if let Ok(call) = serde_json::from_str::<ToolCall>(trimmed) {
                if !call.name.is_empty() {
                    return Some(call);
                }
            }
            // Try as Value
            if let Ok(obj) = serde_json::from_str::<Value>(trimmed) {
                if let Some(name) = obj.get("tool").and_then(|v| v.as_str()) {
                    let input = obj.get("input").cloned().unwrap_or_default();
                    return Some(ToolCall { name: name.to_string(), input });
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tool_block() {
        let resp = "I need to check the directory.\n```tool\n{\"tool\": \"bash\", \"input\": {\"command\": \"pwd\"}}\n```";
        let (call, text) = parse_tool_call(resp).unwrap();
        assert_eq!(call.name, "bash");
        assert_eq!(call.input["command"], "pwd");
        assert!(text.contains("check the directory"));
    }

    #[test]
    fn test_parse_json_block() {
        let resp = "```json\n{\"tool\": \"read\", \"input\": {\"path\": \"main.rs\"}}\n```";
        let (call, _) = parse_tool_call(resp).unwrap();
        assert_eq!(call.name, "read");
        assert_eq!(call.input["path"], "main.rs");
    }

    #[test]
    fn test_parse_inline_json() {
        let resp = "Let me check:\n{\"tool\": \"bash\", \"input\": {\"command\": \"ls\"}}";
        let (call, _) = parse_tool_call(resp).unwrap();
        assert_eq!(call.name, "bash");
    }

    #[test]
    fn test_no_tool_call() {
        let resp = "The answer is 42. No tools needed.";
        assert!(parse_tool_call(resp).is_none());
    }

    #[test]
    fn test_registry_prompt() {
        let mut reg = ToolRegistry::new();
        let prompt = reg.system_prompt();
        assert!(prompt.contains("Available tools"));
    }
}
