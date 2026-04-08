//! Diff Engine — Unified diff generation for file edits
//!
//! Uses `similar` for line-level comparison.
//! Outputs colored +/- format for TUI display.

use similar::{ChangeTag, TextDiff};
use std::path::Path;

/// A single diff hunk with context
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub tag: LineTag,
    pub content: String,
    pub line_num: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineTag {
    Context,
    Insert,
    Delete,
    Header,
}

/// A complete diff between old and new content
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub file_path: String,
    pub lines: Vec<DiffLine>,
    pub insertions: usize,
    pub deletions: usize,
}

impl FileDiff {
    /// Compute diff between old content and new content
    pub fn compute(file_path: &str, old: &str, new: &str) -> Self {
        let diff = TextDiff::from_lines(old, new);
        let mut lines = Vec::new();
        let mut insertions = 0;
        let mut deletions = 0;

        // Header
        lines.push(DiffLine {
            tag: LineTag::Header,
            content: format!("--- a/{file_path}"),
            line_num: None,
        });
        lines.push(DiffLine {
            tag: LineTag::Header,
            content: format!("+++ b/{file_path}"),
            line_num: None,
        });

        let mut old_line = 1usize;
        let mut new_line = 1usize;

        for change in diff.iter_all_changes() {
            let (tag, line_num) = match change.tag() {
                ChangeTag::Equal => {
                    let ln = old_line;
                    old_line += 1;
                    new_line += 1;
                    (LineTag::Context, Some(ln))
                }
                ChangeTag::Insert => {
                    insertions += 1;
                    let ln = new_line;
                    new_line += 1;
                    (LineTag::Insert, Some(ln))
                }
                ChangeTag::Delete => {
                    deletions += 1;
                    let ln = old_line;
                    old_line += 1;
                    (LineTag::Delete, Some(ln))
                }
            };

            let content = change.to_string_lossy().trim_end_matches('\n').to_string();

            lines.push(DiffLine {
                tag,
                content,
                line_num,
            });
        }

        Self { file_path: file_path.to_string(), lines, insertions, deletions }
    }

    /// Summary line: "+3 -1 in src/main.rs"
    pub fn summary(&self) -> String {
        format!("+{} -{} in {}", self.insertions, self.deletions, self.file_path)
    }

    /// Format for plain terminal output (with +/- prefixes)
    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            let prefix = match line.tag {
                LineTag::Context => "  ",
                LineTag::Insert  => "+ ",
                LineTag::Delete  => "- ",
                LineTag::Header  => "",
            };
            out.push_str(&format!("{prefix}{}\n", line.content));
        }
        out
    }

    /// Check if there are actual changes
    pub fn has_changes(&self) -> bool {
        self.insertions > 0 || self.deletions > 0
    }
}

/// Proposed action that needs user approval
#[derive(Debug, Clone)]
pub enum ProposedAction {
    /// Edit a file: show diff, apply on approval
    EditFile {
        path: String,
        diff: FileDiff,
        new_content: String,
    },
    /// Execute a shell command
    RunCommand {
        command: String,
        working_dir: String,
        is_dangerous: bool,
        danger_reason: Option<String>,
    },
    /// Create a new file
    CreateFile {
        path: String,
        content: String,
    },
}

impl ProposedAction {
    pub fn description(&self) -> String {
        match self {
            ProposedAction::EditFile { path, diff, .. } => {
                format!("Edit {} ({})", path, diff.summary())
            }
            ProposedAction::RunCommand { command, is_dangerous, .. } => {
                if *is_dangerous {
                    format!("[!] Execute: {command}")
                } else {
                    format!("Execute: {command}")
                }
            }
            ProposedAction::CreateFile { path, content } => {
                format!("Create {} ({} bytes)", path, content.len())
            }
        }
    }
}

/// Check if a shell command is potentially dangerous
pub fn classify_command_risk(cmd: &str) -> (bool, Option<String>) {
    let lower = cmd.to_lowercase();
    let dangerous_patterns = [
        ("rm -rf", "Recursive force delete"),
        ("rm -r /", "Deleting root filesystem"),
        ("mkfs", "Formatting filesystem"),
        (":(){:|:&};:", "Fork bomb"),
        ("dd if=", "Raw disk write"),
        ("> /dev/sd", "Writing to raw device"),
        ("chmod 777", "World-writable permissions"),
        ("curl | sh", "Piping remote script to shell"),
        ("wget | sh", "Piping remote script to shell"),
        ("sudo rm", "Privileged deletion"),
        ("drop table", "SQL table deletion"),
        ("drop database", "SQL database deletion"),
        ("--no-preserve-root", "Root filesystem override"),
        ("format c:", "Windows drive format"),
    ];

    for (pattern, reason) in &dangerous_patterns {
        if lower.contains(pattern) {
            return (true, Some(reason.to_string()));
        }
    }

    // Moderate risk
    let moderate = [
        ("sudo", "Elevated privileges"),
        ("rm ", "File deletion"),
        ("mv /", "Moving system files"),
        ("chmod", "Changing permissions"),
        ("chown", "Changing ownership"),
        ("kill -9", "Force killing process"),
        ("pkill", "Killing processes by name"),
        ("reboot", "System reboot"),
        ("shutdown", "System shutdown"),
    ];

    for (pattern, reason) in &moderate {
        if lower.contains(pattern) {
            return (false, Some(format!("Caution: {reason}")));
        }
    }

    (false, None)
}

/// Apply a file edit (write new content)
pub fn apply_edit(path: &str, new_content: &str) -> std::io::Result<()> {
    // Create parent dirs if needed
    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, new_content)
}

/// Execute a shell command, capturing output
pub fn execute_command(cmd: &str, working_dir: &str) -> Result<CommandResult, String> {
    let output = std::process::Command::new("sh")
        .args(["-c", cmd])
        .current_dir(working_dir)
        .output()
        .map_err(|e| format!("Failed to execute: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(CommandResult {
        success: output.status.success(),
        exit_code: output.status.code().unwrap_or(-1),
        stdout: if stdout.len() > 4000 { format!("{}... (truncated)", &stdout[..4000]) } else { stdout },
        stderr: if stderr.len() > 2000 { format!("{}... (truncated)", &stderr[..2000]) } else { stderr },
    })
}

#[derive(Debug, Clone)]
pub struct CommandResult {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandResult {
    pub fn summary(&self) -> String {
        if self.success {
            format!("[+] Exit 0 ({} bytes output)", self.stdout.len())
        } else {
            format!("[-] Exit {} : {}", self.exit_code, self.stderr.lines().next().unwrap_or(""))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_no_changes() {
        let diff = FileDiff::compute("test.rs", "hello\n", "hello\n");
        assert!(!diff.has_changes());
        assert_eq!(diff.insertions, 0);
        assert_eq!(diff.deletions, 0);
    }

    #[test]
    fn test_diff_insertion() {
        let diff = FileDiff::compute("test.rs", "line1\nline3\n", "line1\nline2\nline3\n");
        assert!(diff.has_changes());
        assert_eq!(diff.insertions, 1);
        assert_eq!(diff.deletions, 0);
        assert!(diff.summary().contains("+1 -0"));
    }

    #[test]
    fn test_diff_deletion() {
        let diff = FileDiff::compute("test.rs", "a\nb\nc\n", "a\nc\n");
        assert_eq!(diff.deletions, 1);
    }

    #[test]
    fn test_diff_modification() {
        let diff = FileDiff::compute("main.rs", "fn old() {}\n", "fn new() {}\n");
        assert_eq!(diff.insertions, 1);
        assert_eq!(diff.deletions, 1);
    }

    #[test]
    fn test_diff_plain_text() {
        let diff = FileDiff::compute("f.rs", "old\n", "new\n");
        let text = diff.to_plain_text();
        assert!(text.contains("+ new"));
        assert!(text.contains("- old"));
        assert!(text.contains("--- a/f.rs"));
        assert!(text.contains("+++ b/f.rs"));
    }

    #[test]
    fn test_dangerous_command() {
        let (danger, reason) = classify_command_risk("rm -rf /");
        assert!(danger);
        assert!(reason.is_some());
    }

    #[test]
    fn test_safe_command() {
        let (danger, _) = classify_command_risk("ls -la");
        assert!(!danger);
    }

    #[test]
    fn test_moderate_command() {
        let (danger, reason) = classify_command_risk("sudo apt install nginx");
        assert!(!danger);
        assert!(reason.unwrap().contains("Elevated"));
    }

    #[test]
    fn test_execute_echo() {
        let result = execute_command("echo hello", ".").unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("hello"));
    }

    #[test]
    fn test_execute_failure() {
        let result = execute_command("false", ".").unwrap();
        assert!(!result.success);
    }

    #[test]
    fn test_proposed_action_descriptions() {
        let diff = FileDiff::compute("a.rs", "old\n", "new\n");
        let action = ProposedAction::EditFile {
            path: "a.rs".into(), diff, new_content: "new\n".into(),
        };
        assert!(action.description().contains("Edit a.rs"));

        let action = ProposedAction::RunCommand {
            command: "cargo build".into(),
            working_dir: ".".into(),
            is_dangerous: false,
            danger_reason: None,
        };
        assert!(action.description().contains("cargo build"));
    }
}
