//! Built-in tools: Bash, Read, Glob, Edit, Ls

use crate::tool_trait::*;
use anyhow::Result;
use serde_json::Value;
use std::process::Command;

// ────────────────────────────────────────────────────────
// Bash Tool — Execute shell commands
// ────────────────────────────────────────────────────────

pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }
    fn description(&self) -> &str {
        "Execute a shell command and return stdout/stderr."
    }
    fn parameters_hint(&self) -> &str {
        r#"{"command": "the shell command to run"}"#
    }
    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Execute }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let cmd = input.get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if cmd.is_empty() {
            return Ok(ToolResult::error("No command provided".into()));
        }

        let output = Command::new("sh")
            .args(["-c", cmd])
            .current_dir(&ctx.working_dir)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let result = if output.status.success() {
            let out = stdout.to_string();
            if out.len() > 4000 {
                format!("{}...\n[truncated, {} bytes total]", &out[..4000], out.len())
            } else {
                out
            }
        } else {
            format!("Exit code {}\nstdout: {}\nstderr: {}",
                output.status.code().unwrap_or(-1), stdout, stderr)
        };

        Ok(ToolResult::ok(result))
    }
}

// ────────────────────────────────────────────────────────
// Read Tool — Read file contents
// ────────────────────────────────────────────────────────

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &str { "read" }
    fn description(&self) -> &str {
        "Read the contents of a file."
    }
    fn parameters_hint(&self) -> &str {
        r#"{"path": "path/to/file"}"#
    }
    fn permission_level(&self) -> PermissionLevel { PermissionLevel::ReadOnly }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path = input.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if path.is_empty() {
            return Ok(ToolResult::error("No path provided".into()));
        }

        let full_path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            ctx.working_dir.join(path)
        };

        match std::fs::read_to_string(&full_path) {
            Ok(content) => {
                if content.len() > 8000 {
                    let end = (0..=8000).rev().find(|&i| content.is_char_boundary(i)).unwrap_or(0);
                    Ok(ToolResult::ok(format!(
                        "{}\n...[truncated, {} bytes total]", &content[..end], content.len()
                    )))
                } else {
                    Ok(ToolResult::ok(content))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Cannot read {}: {e}", full_path.display()))),
        }
    }
}

// ────────────────────────────────────────────────────────
// Glob Tool — Find files by pattern
// ────────────────────────────────────────────────────────

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }
    fn description(&self) -> &str {
        "List files matching a pattern. Use to explore the project structure."
    }
    fn parameters_hint(&self) -> &str {
        r#"{"pattern": "**/*.rs" or "src/" or "."}"#
    }
    fn permission_level(&self) -> PermissionLevel { PermissionLevel::ReadOnly }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = input.get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or(".");

        // Use `find` for simplicity (works everywhere)
        let cmd = if pattern.contains('*') {
            format!("find . -name '{}' -type f 2>/dev/null | head -50", pattern.replace("**/", ""))
        } else if pattern == "." || pattern.ends_with('/') {
            format!("ls -la {pattern} 2>/dev/null")
        } else {
            format!("find . -path '*{pattern}*' -type f 2>/dev/null | head -50")
        };

        let output = Command::new("sh")
            .args(["-c", &cmd])
            .current_dir(&ctx.working_dir)
            .output()?;

        let result = String::from_utf8_lossy(&output.stdout).to_string();
        if result.is_empty() {
            Ok(ToolResult::ok(format!("No files matching '{pattern}'")))
        } else {
            Ok(ToolResult::ok(result))
        }
    }
}

// ────────────────────────────────────────────────────────
// Edit Tool — Edit a file
// ────────────────────────────────────────────────────────

pub struct EditTool;

impl Tool for EditTool {
    fn name(&self) -> &str { "edit" }
    fn description(&self) -> &str {
        "Replace text in a file. Provide the old text and new text."
    }
    fn parameters_hint(&self) -> &str {
        r#"{"path": "file.rs", "old_text": "text to find", "new_text": "replacement text"}"#
    }
    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Write }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let old_text = input.get("old_text").and_then(|v| v.as_str()).unwrap_or("");
        let new_text = input.get("new_text").and_then(|v| v.as_str()).unwrap_or("");

        if path.is_empty() || old_text.is_empty() {
            return Ok(ToolResult::error("Need path and old_text".into()));
        }

        let full_path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            ctx.working_dir.join(path)
        };

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Cannot read {path}: {e}"))),
        };

        if !content.contains(old_text) {
            return Ok(ToolResult::error(format!(
                "old_text not found in {path}. Make sure it matches exactly."
            )));
        }

        let new_content = content.replacen(old_text, new_text, 1);
        std::fs::write(&full_path, &new_content)?;

        Ok(ToolResult::ok(format!(
            "Edited {path}: replaced {} chars with {} chars",
            old_text.len(), new_text.len()
        )))
    }
}

// ────────────────────────────────────────────────────────
// Write Tool — Create or overwrite a file
// ────────────────────────────────────────────────────────

pub struct WriteTool;

impl Tool for WriteTool {
    fn name(&self) -> &str { "write" }
    fn description(&self) -> &str {
        "Create a new file or overwrite an existing one."
    }
    fn parameters_hint(&self) -> &str {
        r#"{"path": "file.rs", "content": "file contents"}"#
    }
    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Write }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = input.get("content").and_then(|v| v.as_str()).unwrap_or("");

        if path.is_empty() {
            return Ok(ToolResult::error("No path provided".into()));
        }

        let full_path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            ctx.working_dir.join(path)
        };

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&full_path, content)?;
        Ok(ToolResult::ok(format!("Wrote {} bytes to {path}", content.len())))
    }
}

// ────────────────────────────────────────────────────────
// Build default registry
// ────────────────────────────────────────────────────────

pub fn default_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(BashTool));
    reg.register(Box::new(ReadTool));
    reg.register(Box::new(GlobTool));
    reg.register(Box::new(EditTool));
    reg.register(Box::new(WriteTool));
    reg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bash_pwd() {
        let tool = BashTool;
        let ctx = ToolContext::new();
        let input = serde_json::json!({"command": "echo hello"});
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello"));
    }

    #[test]
    fn test_read_nonexistent() {
        let tool = ReadTool;
        let ctx = ToolContext::new();
        let input = serde_json::json!({"path": "/nonexistent/file.txt"});
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_glob_current() {
        let tool = GlobTool;
        let ctx = ToolContext::new();
        let input = serde_json::json!({"pattern": "."});
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
    }

    #[test]
    fn test_default_registry() {
        let reg = default_registry();
        assert!(reg.get("bash").is_some());
        assert!(reg.get("read").is_some());
        assert!(reg.get("glob").is_some());
        assert!(reg.get("edit").is_some());
        assert!(reg.get("write").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_prompt_lists_all_tools() {
        let reg = default_registry();
        let prompt = reg.system_prompt();
        assert!(prompt.contains("### bash"));
        assert!(prompt.contains("### read"));
        assert!(prompt.contains("### glob"));
        assert!(prompt.contains("### edit"));
        assert!(prompt.contains("### write"));
    }
}
