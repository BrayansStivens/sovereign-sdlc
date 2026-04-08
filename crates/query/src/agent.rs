use anyhow::Result;
use chrono::Local;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use std::path::PathBuf;
use std::process::Command;

use sovereign_core::SYSTEM_IDENTITY;

/// Actions the agent can take in the ReAct loop
#[derive(Debug, Clone)]
pub enum Action {
    /// Read a file from the filesystem
    ReadFile(PathBuf),
    /// Execute a shell command (requires user approval)
    Execute(String),
    /// Respond to the user with final answer
    Respond(String),
    /// Think — internal reasoning step
    Think(String),
}

impl std::fmt::Display for Action {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Action::ReadFile(p) => write!(f, "READ: {}", p.display()),
            Action::Execute(cmd) => write!(f, "EXEC: {cmd}"),
            Action::Respond(msg) => write!(f, "RESPOND: {}", &msg[..msg.len().min(80)]),
            Action::Think(t) => write!(f, "THINK: {}", &t[..t.len().min(80)]),
        }
    }
}

/// A single step in the ReAct thought chain
#[derive(Debug, Clone)]
pub struct ThoughtStep {
    pub timestamp: String,
    pub step_type: StepType,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepType {
    Thought,
    Action,
    Observation,
    Answer,
}

impl std::fmt::Display for StepType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StepType::Thought => write!(f, "THINK"),
            StepType::Action => write!(f, "ACT"),
            StepType::Observation => write!(f, "OBS"),
            StepType::Answer => write!(f, "ANS"),
        }
    }
}

/// The ReAct agent that orchestrates thinking, acting, and observing
pub struct ReactAgent {
    ollama: Ollama,
    pub thought_log: Vec<ThoughtStep>,
    max_iterations: usize,
}

const REACT_SYSTEM_PROMPT: &str = r#"You are a ReAct agent. For each user request, follow this loop:

1. **Thought**: Analyze what you need to do. Reason step by step.
2. **Action**: Choose ONE action:
   - `READ_FILE <path>` — read a file from the filesystem
   - `EXECUTE <command>` — run a shell command (will require user approval)
   - `ANSWER <response>` — provide your final answer to the user

3. **Observation**: You will receive the result of your action, then loop back to Thought.

Rules:
- Always start with a Thought.
- Only use EXECUTE for safe, read-only commands unless the user explicitly asks for modifications.
- After gathering enough information, use ANSWER to respond.
- Output EXACTLY one action per turn. Format: ACTION_TYPE content

Example:
Thought: I need to check what files are in the project directory.
Action: EXECUTE ls -la
Observation: [result shown]
Thought: I see a Cargo.toml, this is a Rust project. Let me read it.
Action: READ_FILE Cargo.toml
Observation: [file contents]
Thought: Now I have enough context to answer.
Action: ANSWER The project is a Rust CLI tool with these dependencies...
"#;

impl ReactAgent {
    pub fn new(ollama: Ollama) -> Self {
        Self {
            ollama,
            thought_log: Vec::new(),
            max_iterations: 8,
        }
    }

    fn log_step(&mut self, step_type: StepType, content: &str) {
        self.thought_log.push(ThoughtStep {
            timestamp: Local::now().format("%H:%M:%S").to_string(),
            step_type,
            content: content.to_string(),
        });
    }

    /// Parse the LLM response to extract the action
    fn parse_action(&self, response: &str) -> Action {
        let lines: Vec<&str> = response.lines().collect();

        for line in &lines {
            let trimmed = line.trim();

            if let Some(path) = trimmed
                .strip_prefix("Action: READ_FILE ")
                .or_else(|| trimmed.strip_prefix("READ_FILE "))
            {
                return Action::ReadFile(PathBuf::from(path.trim()));
            }

            if let Some(cmd) = trimmed
                .strip_prefix("Action: EXECUTE ")
                .or_else(|| trimmed.strip_prefix("EXECUTE "))
            {
                return Action::Execute(cmd.trim().to_string());
            }

            if let Some(answer) = trimmed
                .strip_prefix("Action: ANSWER ")
                .or_else(|| trimmed.strip_prefix("ANSWER "))
            {
                return Action::Respond(answer.trim().to_string());
            }
        }

        // If no explicit action found, treat entire response as answer
        Action::Respond(response.to_string())
    }

    /// Execute one iteration of the ReAct loop.
    /// Returns Some(answer) when done, None if needs more iterations.
    /// `pending_approval` is set when an EXECUTE action needs user confirmation.
    pub async fn step(
        &mut self,
        model: &str,
        context: &str,
    ) -> Result<ReActResult> {
        let prompt = format!(
            "{SYSTEM_IDENTITY}{REACT_SYSTEM_PROMPT}\n\n\
             Previous context:\n{context}\n\n\
             Continue the ReAct loop. Output your Thought, then your Action."
        );

        let request = GenerationRequest::new(model.to_string(), prompt);
        let response = self.ollama.generate(request).await?;
        let text = response.response.trim().to_string();

        // Extract thought (everything before the action line)
        let thought = text
            .lines()
            .take_while(|l| {
                let t = l.trim();
                !t.starts_with("Action:") && !t.starts_with("READ_FILE")
                    && !t.starts_with("EXECUTE") && !t.starts_with("ANSWER")
            })
            .collect::<Vec<_>>()
            .join("\n");

        if !thought.is_empty() {
            self.log_step(StepType::Thought, &thought);
        }

        let action = self.parse_action(&text);
        self.log_step(StepType::Action, &format!("{action}"));

        match action {
            Action::ReadFile(path) => {
                let observation = match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        // Truncate large files
                        if content.len() > 4000 {
                            format!("{}\n... [truncated, {} total bytes]", &content[..4000], content.len())
                        } else {
                            content
                        }
                    }
                    Err(e) => format!("Error reading {}: {e}", path.display()),
                };
                self.log_step(StepType::Observation, &observation);
                Ok(ReActResult::Continue(observation))
            }
            Action::Execute(cmd) => {
                // Needs user approval — return the command for confirmation
                Ok(ReActResult::NeedsApproval(cmd))
            }
            Action::Respond(answer) => {
                self.log_step(StepType::Answer, &answer);
                Ok(ReActResult::Done(answer))
            }
            Action::Think(t) => {
                self.log_step(StepType::Thought, &t);
                Ok(ReActResult::Continue(t))
            }
        }
    }

    /// Execute an approved shell command and return observation
    pub fn execute_command(&mut self, cmd: &str) -> String {
        let output = Command::new("sh")
            .args(["-c", cmd])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let result = if out.status.success() {
                    if stdout.len() > 4000 {
                        format!("{}\n... [truncated]", &stdout[..4000])
                    } else {
                        stdout.to_string()
                    }
                } else {
                    format!("Exit code: {}\nstdout: {stdout}\nstderr: {stderr}", out.status)
                };
                self.log_step(StepType::Observation, &result);
                result
            }
            Err(e) => {
                let err = format!("Failed to execute: {e}");
                self.log_step(StepType::Observation, &err);
                err
            }
        }
    }

    /// Run the full ReAct loop for a prompt
    pub async fn run(
        &mut self,
        model: &str,
        user_prompt: &str,
        approve_fn: &mut dyn FnMut(&str) -> bool,
    ) -> Result<String> {
        let mut context = format!("User: {user_prompt}");

        for i in 0..self.max_iterations {
            self.log_step(
                StepType::Thought,
                &format!("--- Iteration {}/{} ---", i + 1, self.max_iterations),
            );

            match self.step(model, &context).await? {
                ReActResult::Done(answer) => return Ok(answer),
                ReActResult::Continue(observation) => {
                    context.push_str(&format!("\nObservation: {observation}"));
                }
                ReActResult::NeedsApproval(cmd) => {
                    if approve_fn(&cmd) {
                        let observation = self.execute_command(&cmd);
                        context.push_str(&format!("\nObservation: {observation}"));
                    } else {
                        let denied = "Command denied by user.";
                        self.log_step(StepType::Observation, denied);
                        context.push_str(&format!("\nObservation: {denied}"));
                    }
                }
            }
        }

        Ok("Max iterations reached. Here's what I found so far based on my analysis.".to_string())
    }

    /// Get formatted thought log for display
    pub fn format_log(&self) -> String {
        self.thought_log
            .iter()
            .map(|s| format!("[{}] {}: {}", s.timestamp, s.step_type, s.content))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Get the last N thoughts for display in TUI panel
    pub fn recent_thoughts(&self, n: usize) -> Vec<&ThoughtStep> {
        let start = self.thought_log.len().saturating_sub(n);
        self.thought_log[start..].iter().collect()
    }

    /// Clear thought log
    pub fn clear_log(&mut self) {
        self.thought_log.clear();
    }
}

/// Result of a single ReAct step
pub enum ReActResult {
    /// Agent produced a final answer
    Done(String),
    /// Agent needs more information — observation from previous action
    Continue(String),
    /// Agent wants to execute a command — needs user approval
    NeedsApproval(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_read_file() {
        let agent = ReactAgent::new(Ollama::default());
        let action = agent.parse_action("Thought: I need to read the config.\nAction: READ_FILE config.toml");
        assert!(matches!(action, Action::ReadFile(p) if p == PathBuf::from("config.toml")));
    }

    #[test]
    fn test_parse_execute() {
        let agent = ReactAgent::new(Ollama::default());
        let action = agent.parse_action("Thought: Let me check files.\nAction: EXECUTE ls -la");
        assert!(matches!(action, Action::Execute(cmd) if cmd == "ls -la"));
    }

    #[test]
    fn test_parse_answer() {
        let agent = ReactAgent::new(Ollama::default());
        let action = agent.parse_action("Thought: I know the answer.\nAction: ANSWER The result is 42.");
        assert!(matches!(action, Action::Respond(a) if a == "The result is 42."));
    }

    #[test]
    fn test_parse_fallback() {
        let agent = ReactAgent::new(Ollama::default());
        let action = agent.parse_action("Just a plain response without action markers.");
        assert!(matches!(action, Action::Respond(_)));
    }

    #[test]
    fn test_thought_log() {
        let mut agent = ReactAgent::new(Ollama::default());
        agent.log_step(StepType::Thought, "Analyzing the problem");
        agent.log_step(StepType::Action, "READ_FILE main.rs");
        agent.log_step(StepType::Observation, "fn main() { }");

        assert_eq!(agent.thought_log.len(), 3);
        assert_eq!(agent.thought_log[0].step_type, StepType::Thought);
        assert_eq!(agent.thought_log[1].step_type, StepType::Action);
    }

    #[test]
    fn test_recent_thoughts() {
        let mut agent = ReactAgent::new(Ollama::default());
        for i in 0..10 {
            agent.log_step(StepType::Thought, &format!("step {i}"));
        }
        let recent = agent.recent_thoughts(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].content.contains("step 7"));
    }

    #[test]
    fn test_execute_command() {
        let mut agent = ReactAgent::new(Ollama::default());
        let result = agent.execute_command("echo hello");
        assert!(result.contains("hello"));
        assert_eq!(agent.thought_log.len(), 1);
        assert_eq!(agent.thought_log[0].step_type, StepType::Observation);
    }
}
