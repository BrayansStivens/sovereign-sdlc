//! Built-in tools: Bash, Read, Glob, Grep, Edit, Write
//!
//! Adapted from claurst tool patterns for local Ollama models.
//! v0.5.0: persistent cwd, timeouts, line numbers, grep, replace_all

use crate::tool_trait::*;
use anyhow::Result;
use regex::RegexBuilder;
use serde_json::Value;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use walkdir::WalkDir;

// ────────────────────────────────────────────────────────
// Shared helpers
// ────────────────────────────────────────────────────────

const CWD_SENTINEL: &str = "__SOVEREIGN_STATE__";
const MAX_OUTPUT: usize = 100_000;

/// Global persistent working directory (survives across bash calls)
fn persistent_cwd() -> &'static Mutex<Option<PathBuf>> {
    static CWD: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    CWD.get_or_init(|| Mutex::new(None))
}

/// Truncate output keeping first and last half when over max_len
fn truncate_output(output: &str, max_len: usize) -> String {
    if output.len() <= max_len {
        return output.to_string();
    }
    let half = max_len / 2;
    let start = &output[..half];
    let end = &output[output.len() - half..];
    format!(
        "{}\n\n... ({} characters truncated) ...\n\n{}",
        start,
        output.len() - max_len,
        end
    )
}

/// Resolve a path: absolute paths pass through, relative join with working_dir
fn resolve_path(path: &str, working_dir: &PathBuf) -> PathBuf {
    if std::path::Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        working_dir.join(path)
    }
}

// ────────────────────────────────────────────────────────
// Bash Tool — Execute shell commands with persistent cwd
// ────────────────────────────────────────────────────────

pub struct BashTool;

impl Tool for BashTool {
    fn name(&self) -> &str { "bash" }

    fn description(&self) -> &str {
        "Execute a shell command. The working directory persists between calls \
         (cd in one call carries over to the next). Timeout: default 120s, max 600s."
    }

    fn parameters_hint(&self) -> &str {
        r#"{"command": "shell command to run", "timeout": 120000}"#
    }

    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Execute }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let cmd = input.get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if cmd.is_empty() {
            return Ok(ToolResult::error("No command provided".into()));
        }

        let timeout_ms = input.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120_000)
            .min(600_000);

        // Use persistent cwd if available, otherwise context working_dir
        let effective_cwd = persistent_cwd()
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| ctx.working_dir.clone());

        // Wrapper script: restore cwd, run command, capture final pwd
        let cwd_escaped = effective_cwd
            .display()
            .to_string()
            .replace('\'', "'\\''");

        let script = format!(
            "cd '{cwd}' 2>/dev/null || true\n\
             {cmd}\n\
             __SOVEREIGN_EXIT=$?\n\
             echo '{sentinel}'\n\
             pwd\n\
             exit $__SOVEREIGN_EXIT",
            cwd = cwd_escaped,
            cmd = cmd,
            sentinel = CWD_SENTINEL,
        );

        let mut child = match Command::new("bash")
            .arg("-c")
            .arg(&script)
            .current_dir(&ctx.working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to spawn: {e}"))),
        };

        // Read stdout/stderr in threads to prevent deadlock on full pipe buffers
        let stdout_pipe = child.stdout.take().unwrap();
        let stderr_pipe = child.stderr.take().unwrap();

        let stdout_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            std::io::BufReader::new(stdout_pipe).read_to_string(&mut buf).ok();
            buf
        });
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            std::io::BufReader::new(stderr_pipe).read_to_string(&mut buf).ok();
            buf
        });

        // Poll child with timeout
        let timeout = Duration::from_millis(timeout_ms);
        let start = Instant::now();
        let status = loop {
            match child.try_wait()? {
                Some(s) => break Some(s),
                None if start.elapsed() > timeout => {
                    let _ = child.kill();
                    return Ok(ToolResult::error(format!(
                        "Command timed out after {}ms",
                        timeout_ms
                    )));
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        };

        let stdout_raw = stdout_thread.join().unwrap_or_default();
        let stderr_output = stderr_thread.join().unwrap_or_default();

        // Split stdout at sentinel to extract user output and new cwd
        let (user_output, new_cwd) = if let Some(pos) = stdout_raw.rfind(CWD_SENTINEL) {
            let user = stdout_raw[..pos].trim_end().to_string();
            let after = stdout_raw[pos + CWD_SENTINEL.len()..].trim();
            let cwd = after
                .lines()
                .next()
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
            (user, cwd)
        } else {
            (stdout_raw, None)
        };

        // Persist the new cwd for subsequent calls
        if let Some(cwd) = new_cwd {
            *persistent_cwd().lock().unwrap() = Some(cwd);
        }

        let exit_code = status.and_then(|s| s.code()).unwrap_or(-1);

        let mut output = user_output;
        if !stderr_output.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str("STDERR:\n");
            output.push_str(&stderr_output);
        }
        if output.is_empty() {
            output = "(no output)".to_string();
        }

        output = truncate_output(&output, MAX_OUTPUT);

        if exit_code != 0 {
            Ok(ToolResult::error(format!("Exit code {}\n{}", exit_code, output)))
        } else {
            Ok(ToolResult::ok(output))
        }
    }
}

// ────────────────────────────────────────────────────────
// Read Tool — Read file with line numbers & offset/limit
// ────────────────────────────────────────────────────────

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &str { "read" }

    fn description(&self) -> &str {
        "Read a file with line numbers. Supports offset (1-based) and limit \
         for large files. Default: first 2000 lines."
    }

    fn parameters_hint(&self) -> &str {
        r#"{"path": "file.rs", "offset": 1, "limit": 2000}"#
    }

    fn permission_level(&self) -> PermissionLevel { PermissionLevel::ReadOnly }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if path_str.is_empty() {
            return Ok(ToolResult::error("No path provided".into()));
        }

        let full_path = resolve_path(path_str, &ctx.working_dir);

        if !full_path.exists() {
            return Ok(ToolResult::error(format!(
                "File not found: {}",
                full_path.display()
            )));
        }
        if full_path.is_dir() {
            return Ok(ToolResult::error(format!(
                "{} is a directory. Use bash with `ls` to list contents.",
                full_path.display()
            )));
        }

        // Detect binary/image files
        let ext = full_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        let image_exts = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "svg", "ico"];
        if image_exts.contains(&ext.as_str()) {
            return Ok(ToolResult::ok(format!(
                "[Image file: {}]",
                full_path.display()
            )));
        }

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Cannot read {}: {e}",
                    full_path.display()
                )))
            }
        };

        if content.is_empty() {
            return Ok(ToolResult::ok(format!(
                "[File {} exists but is empty]",
                full_path.display()
            )));
        }

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();

        let offset = input
            .get("offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let limit = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(2000) as usize;

        // Convert 1-based offset to 0-based index
        let start = if offset > 0 { offset - 1 } else { 0 };
        if start >= total {
            return Ok(ToolResult::error(format!(
                "Offset {} exceeds total lines ({})",
                offset, total
            )));
        }
        let end = (start + limit).min(total);

        // Format with right-aligned line numbers
        let width = format!("{}", end).len();
        let mut output = String::with_capacity(content.len());

        for (i, line) in lines[start..end].iter().enumerate() {
            output.push_str(&format!(
                "{:>width$}\t{}\n",
                start + i + 1,
                line,
                width = width
            ));
        }

        if end < total {
            output.push_str(&format!(
                "\n... ({} more lines, {} total. Use offset/limit to read more.)\n",
                total - end,
                total
            ));
        }

        Ok(ToolResult::ok(output))
    }
}

// ────────────────────────────────────────────────────────
// Glob Tool — Find files with native glob matching
// ────────────────────────────────────────────────────────

pub struct GlobTool;

impl Tool for GlobTool {
    fn name(&self) -> &str { "glob" }

    fn description(&self) -> &str {
        "Find files matching a glob pattern (e.g. \"**/*.rs\", \"src/**/*.ts\"). \
         Plain names without wildcards search recursively. Returns sorted paths."
    }

    fn parameters_hint(&self) -> &str {
        r#"{"pattern": "**/*.rs", "path": "src/"}"#
    }

    fn permission_level(&self) -> PermissionLevel { PermissionLevel::ReadOnly }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("*");

        let base_dir = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(p, &ctx.working_dir))
            .unwrap_or_else(|| ctx.working_dir.clone());

        // Build full glob pattern
        let full_pattern = if std::path::Path::new(pattern).is_absolute() {
            pattern.to_string()
        } else if pattern == "." {
            // List current directory contents
            format!("{}/*", base_dir.display())
        } else if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') {
            // No wildcards → recursive search for filename
            format!("{}/**/{}", base_dir.display(), pattern)
        } else {
            format!("{}/{}", base_dir.display(), pattern)
        };

        match glob::glob(&full_pattern) {
            Ok(paths) => {
                let mut matches: Vec<String> = Vec::new();
                for entry in paths {
                    if matches.len() >= 200 {
                        break;
                    }
                    if let Ok(path) = entry {
                        if !path.is_file() {
                            continue;
                        }
                        // Skip hidden paths
                        let has_hidden = path.components().any(|c| {
                            let s = c.as_os_str().to_string_lossy();
                            s.starts_with('.') && s != "." && s != ".."
                        });
                        if has_hidden {
                            continue;
                        }
                        matches.push(path.display().to_string());
                    }
                }
                if matches.is_empty() {
                    Ok(ToolResult::ok(format!("No files matching '{pattern}'")))
                } else {
                    matches.sort();
                    Ok(ToolResult::ok(matches.join("\n")))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("Invalid glob pattern: {e}"))),
        }
    }
}

// ────────────────────────────────────────────────────────
// Grep Tool — Regex search across files (NEW)
// ────────────────────────────────────────────────────────

pub struct GrepTool;

/// Map file type shorthand to extensions (ripgrep-style)
fn extensions_for_type(t: &str) -> Vec<&'static str> {
    match t {
        "rust" | "rs" => vec!["rs"],
        "js" => vec!["js", "jsx", "mjs", "cjs"],
        "ts" => vec!["ts", "tsx", "mts", "cts"],
        "py" | "python" => vec!["py", "pyi"],
        "go" => vec!["go"],
        "java" => vec!["java"],
        "c" => vec!["c", "h"],
        "cpp" => vec!["cpp", "hpp", "cc", "hh", "cxx"],
        "rb" | "ruby" => vec!["rb"],
        "swift" => vec!["swift"],
        "css" => vec!["css", "scss", "sass", "less"],
        "html" => vec!["html", "htm"],
        "json" => vec!["json"],
        "yaml" | "yml" => vec!["yaml", "yml"],
        "toml" => vec!["toml"],
        "md" | "markdown" => vec!["md", "markdown"],
        "sh" | "shell" | "bash" => vec!["sh", "bash", "zsh"],
        _ => vec![],
    }
}

impl Tool for GrepTool {
    fn name(&self) -> &str { "grep" }

    fn description(&self) -> &str {
        "Search file contents with regex. Output modes: \"files_with_matches\" \
         (default, just paths), \"content\" (matching lines with line numbers), \
         \"count\" (match counts per file). Filter by file type or glob."
    }

    fn parameters_hint(&self) -> &str {
        r#"{"pattern": "regex", "path": "dir/", "output_mode": "content", "type": "rs", "glob": "*.rs", "context": 2, "case_insensitive": false, "head_limit": 250}"#
    }

    fn permission_level(&self) -> PermissionLevel { PermissionLevel::ReadOnly }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = input
            .get("pattern")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if pattern.is_empty() {
            return Ok(ToolResult::error("No pattern provided".into()));
        }

        let search_path = input
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| resolve_path(p, &ctx.working_dir))
            .unwrap_or_else(|| ctx.working_dir.clone());

        let case_insensitive = input
            .get("case_insensitive")
            .or_else(|| input.get("-i"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let multiline = input
            .get("multiline")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let regex = match RegexBuilder::new(pattern)
            .case_insensitive(case_insensitive)
            .dot_matches_new_line(multiline)
            .multi_line(multiline)
            .build()
        {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Invalid regex: {e}"))),
        };

        let output_mode = input
            .get("output_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("files_with_matches");

        let context_lines = input
            .get("context")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        let head_limit = input
            .get("head_limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(250) as usize;

        let type_exts: Vec<&str> = input
            .get("type")
            .and_then(|v| v.as_str())
            .map(extensions_for_type)
            .unwrap_or_default();

        let glob_filter = input.get("glob").and_then(|v| v.as_str());

        // Single file search
        if search_path.is_file() {
            return Ok(self.search_file(
                &search_path,
                &regex,
                output_mode,
                context_lines,
            ));
        }

        // Directory walk
        let mut results: Vec<String> = Vec::new();
        let mut match_count = 0usize;

        for entry in WalkDir::new(&search_path)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !name.starts_with('.')
                    && name != "node_modules"
                    && name != "target"
                    && name != "__pycache__"
            })
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }

            let path = entry.path();

            // Type filter
            if !type_exts.is_empty() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !type_exts.contains(&ext) {
                    continue;
                }
            }

            // Glob filter
            if let Some(gp) = glob_filter {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if let Ok(m) = glob::Pattern::new(gp) {
                    if !m.matches(name) {
                        continue;
                    }
                }
            }

            // Read file (skip binary / unreadable)
            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let lines: Vec<&str> = content.lines().collect();
            let file_matches: Vec<usize> = lines
                .iter()
                .enumerate()
                .filter(|(_, l)| regex.is_match(l))
                .map(|(i, _)| i)
                .collect();

            if file_matches.is_empty() {
                continue;
            }

            match output_mode {
                "files_with_matches" => {
                    results.push(path.display().to_string());
                    match_count += 1;
                }
                "count" => {
                    results.push(format!("{}:{}", path.display(), file_matches.len()));
                    match_count += 1;
                }
                _ => {
                    // "content" mode — show matching lines with context
                    for &idx in &file_matches {
                        let start = idx.saturating_sub(context_lines);
                        let end = (idx + context_lines + 1).min(lines.len());
                        for ci in start..end {
                            results.push(format!(
                                "{}:{}:{}",
                                path.display(),
                                ci + 1,
                                lines[ci]
                            ));
                        }
                        if context_lines > 0 {
                            results.push("--".to_string());
                        }
                        match_count += 1;
                    }
                }
            }

            if match_count >= head_limit {
                break;
            }
        }

        if results.is_empty() {
            Ok(ToolResult::ok(format!(
                "No matches for \"{}\" in {}",
                pattern,
                search_path.display()
            )))
        } else {
            Ok(ToolResult::ok(results.join("\n")))
        }
    }
}

impl GrepTool {
    fn search_file(
        &self,
        path: &PathBuf,
        regex: &regex::Regex,
        output_mode: &str,
        context_lines: usize,
    ) -> ToolResult {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                return ToolResult::error(format!("Cannot read {}: {e}", path.display()))
            }
        };

        let lines: Vec<&str> = content.lines().collect();
        let matching: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| regex.is_match(l))
            .map(|(i, _)| i)
            .collect();

        if matching.is_empty() {
            return ToolResult::ok(format!("No matches in {}", path.display()));
        }

        match output_mode {
            "files_with_matches" => ToolResult::ok(path.display().to_string()),
            "count" => {
                ToolResult::ok(format!("{}:{}", path.display(), matching.len()))
            }
            _ => {
                let mut results = Vec::new();
                for &idx in &matching {
                    let start = idx.saturating_sub(context_lines);
                    let end = (idx + context_lines + 1).min(lines.len());
                    for ci in start..end {
                        results.push(format!("{}:{}", ci + 1, lines[ci]));
                    }
                    if context_lines > 0 {
                        results.push("--".to_string());
                    }
                }
                ToolResult::ok(results.join("\n"))
            }
        }
    }
}

// ────────────────────────────────────────────────────────
// Edit Tool — Replace text with uniqueness check
// ────────────────────────────────────────────────────────

pub struct EditTool;

impl Tool for EditTool {
    fn name(&self) -> &str { "edit" }

    fn description(&self) -> &str {
        "Replace text in a file. Fails if old_text appears multiple times \
         (unless replace_all is true). You must read the file first."
    }

    fn parameters_hint(&self) -> &str {
        r#"{"path": "file.rs", "old_text": "text to find", "new_text": "replacement", "replace_all": false}"#
    }

    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Write }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let old_text = input
            .get("old_text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let new_text = input
            .get("new_text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let replace_all = input
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if path_str.is_empty() || old_text.is_empty() {
            return Ok(ToolResult::error("Need path and old_text".into()));
        }
        if old_text == new_text {
            return Ok(ToolResult::error(
                "old_text and new_text must be different".into(),
            ));
        }

        let full_path = resolve_path(path_str, &ctx.working_dir);

        let content = match std::fs::read_to_string(&full_path) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Cannot read {path_str}: {e}"
                )))
            }
        };

        let count = content.matches(old_text).count();

        if count == 0 {
            return Ok(ToolResult::error(format!(
                "old_text not found in {path_str}. Make sure it matches exactly, \
                 including whitespace and indentation."
            )));
        }

        if count > 1 && !replace_all {
            return Ok(ToolResult::error(format!(
                "old_text appears {count} times in {path_str}. Provide more \
                 context to make it unique, or set replace_all to true."
            )));
        }

        let new_content = if replace_all {
            content.replace(old_text, new_text)
        } else {
            content.replacen(old_text, new_text, 1)
        };

        std::fs::write(&full_path, &new_content)?;

        let n = if replace_all { count } else { 1 };
        Ok(ToolResult::ok(format!(
            "Edited {path_str} ({n} replacement{})",
            if n != 1 { "s" } else { "" }
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
        "Create a new file or overwrite an existing one. Creates parent \
         directories automatically."
    }

    fn parameters_hint(&self) -> &str {
        r#"{"path": "file.rs", "content": "file contents"}"#
    }

    fn permission_level(&self) -> PermissionLevel { PermissionLevel::Write }

    fn execute(&self, input: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path_str = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = input
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if path_str.is_empty() {
            return Ok(ToolResult::error("No path provided".into()));
        }

        let full_path = resolve_path(path_str, &ctx.working_dir);

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&full_path, content)?;
        Ok(ToolResult::ok(format!(
            "Wrote {} bytes to {path_str}",
            content.len()
        )))
    }
}

// ────────────────────────────────────────────────────────
// Build default registry (6 tools)
// ────────────────────────────────────────────────────────

pub fn default_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(BashTool));
    reg.register(Box::new(ReadTool));
    reg.register(Box::new(GlobTool));
    reg.register(Box::new(GrepTool));
    reg.register(Box::new(EditTool));
    reg.register(Box::new(WriteTool));
    reg
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bash_echo() {
        let tool = BashTool;
        let ctx = ToolContext::new();
        let input = serde_json::json!({"command": "echo hello"});
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error, "Unexpected error: {}", result.output);
        assert!(result.output.contains("hello"));
    }

    #[test]
    fn test_bash_cwd_tracking() {
        let tool = BashTool;
        let ctx = ToolContext::new();
        let input = serde_json::json!({"command": "cd /tmp && pwd"});
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error, "Unexpected error: {}", result.output);
        assert!(
            result.output.contains("tmp"),
            "Expected /tmp in output: {}",
            result.output
        );
    }

    #[test]
    fn test_bash_empty_command() {
        let tool = BashTool;
        let ctx = ToolContext::new();
        let input = serde_json::json!({"command": ""});
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_read_with_line_numbers() {
        let tool = ReadTool;
        let ctx = ToolContext::new();

        let dir = std::env::temp_dir();
        let test_file = dir.join("sovereign_test_read.txt");
        std::fs::write(&test_file, "line one\nline two\nline three\n").unwrap();

        let input = serde_json::json!({
            "path": test_file.display().to_string(),
            "limit": 2
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error, "Unexpected error: {}", result.output);
        assert!(result.output.contains("1\t"), "Missing line number 1");
        assert!(result.output.contains("line one"));
        assert!(result.output.contains("2\t"), "Missing line number 2");
        assert!(result.output.contains("line two"));
        assert!(
            result.output.contains("more lines"),
            "Should indicate more lines available"
        );

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_read_with_offset() {
        let tool = ReadTool;
        let ctx = ToolContext::new();

        let dir = std::env::temp_dir();
        let test_file = dir.join("sovereign_test_offset.txt");
        std::fs::write(&test_file, "a\nb\nc\nd\ne\n").unwrap();

        let input = serde_json::json!({
            "path": test_file.display().to_string(),
            "offset": 3,
            "limit": 2
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("3\t"));
        assert!(result.output.contains("c"));
        assert!(result.output.contains("4\t"));
        assert!(result.output.contains("d"));

        std::fs::remove_file(&test_file).ok();
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
    fn test_glob_wildcard() {
        let tool = GlobTool;
        let ctx = ToolContext::new();

        let dir = std::env::temp_dir().join("sovereign_glob_test");
        std::fs::create_dir_all(&dir).ok();
        std::fs::write(dir.join("test.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("test.txt"), "hello").unwrap();

        let input = serde_json::json!({
            "pattern": "*.rs",
            "path": dir.display().to_string()
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("test.rs"));
        assert!(!result.output.contains("test.txt"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_grep_in_file() {
        let tool = GrepTool;
        let ctx = ToolContext::new();

        let dir = std::env::temp_dir();
        let test_file = dir.join("sovereign_test_grep.rs");
        std::fs::write(
            &test_file,
            "fn main() {\n    println!(\"hello world\");\n}\n",
        )
        .unwrap();

        let input = serde_json::json!({
            "pattern": "println",
            "path": test_file.display().to_string(),
            "output_mode": "content"
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("println"));
        assert!(result.output.contains("2:"), "Should show line number");

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_grep_case_insensitive() {
        let tool = GrepTool;
        let ctx = ToolContext::new();

        let dir = std::env::temp_dir();
        let test_file = dir.join("sovereign_test_grep_ci.txt");
        std::fs::write(&test_file, "Hello World\nhello world\nHELLO WORLD\n")
            .unwrap();

        let input = serde_json::json!({
            "pattern": "hello",
            "path": test_file.display().to_string(),
            "output_mode": "count",
            "case_insensitive": true
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(
            result.output.contains(":3"),
            "Should find 3 matches: {}",
            result.output
        );

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_grep_files_with_matches() {
        let tool = GrepTool;
        let ctx = ToolContext::new();

        let dir = std::env::temp_dir().join("sovereign_grep_fwm");
        std::fs::create_dir_all(&dir).ok();
        std::fs::write(dir.join("a.rs"), "fn foo() {}").unwrap();
        std::fs::write(dir.join("b.rs"), "fn bar() {}").unwrap();
        std::fs::write(dir.join("c.txt"), "no match here").unwrap();

        let input = serde_json::json!({
            "pattern": "fn \\w+",
            "path": dir.display().to_string(),
            "output_mode": "files_with_matches",
            "type": "rs"
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("a.rs"));
        assert!(result.output.contains("b.rs"));
        assert!(!result.output.contains("c.txt"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_edit_replace_all() {
        let ctx = ToolContext::new();
        let dir = std::env::temp_dir();
        let test_file = dir.join("sovereign_test_edit_all.txt");
        std::fs::write(&test_file, "foo bar foo baz foo").unwrap();

        let tool = EditTool;
        let input = serde_json::json!({
            "path": test_file.display().to_string(),
            "old_text": "foo",
            "new_text": "qux",
            "replace_all": true
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("3 replacements"));

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(content, "qux bar qux baz qux");

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_edit_rejects_ambiguous() {
        let ctx = ToolContext::new();
        let dir = std::env::temp_dir();
        let test_file = dir.join("sovereign_test_edit_ambig.txt");
        std::fs::write(&test_file, "foo bar foo").unwrap();

        let tool = EditTool;
        let input = serde_json::json!({
            "path": test_file.display().to_string(),
            "old_text": "foo",
            "new_text": "qux"
        });
        let result = tool.execute(&input, &ctx).unwrap();
        assert!(
            result.is_error,
            "Should reject ambiguous match: {}",
            result.output
        );
        assert!(result.output.contains("2 times"));

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_default_registry() {
        let reg = default_registry();
        assert!(reg.get("bash").is_some());
        assert!(reg.get("read").is_some());
        assert!(reg.get("glob").is_some());
        assert!(reg.get("grep").is_some());
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
        assert!(prompt.contains("### grep"));
        assert!(prompt.contains("### edit"));
        assert!(prompt.contains("### write"));
    }
}
