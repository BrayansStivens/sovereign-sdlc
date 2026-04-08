//! Agent Loop v0.5.0 — Native tool-calling agent
//!
//! Uses Ollama's native function calling API for reliable tool detection.
//! No more text-based ```tool parsing — the model returns structured tool_calls.

use sovereign_api::{GenMetrics, OllamaClient, build_native_tool_schemas};
use sovereign_core::ConversationMessage;
use sovereign_tools::{
    PermissionLevel, ToolCall, ToolContext, ToolRegistry, ToolResult,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

/// Events sent from the agent loop to the TUI
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Full text response (shown all at once after model finishes)
    TextResponse(String),
    /// Tool execution started
    ToolStart {
        name: String,
        input_summary: String,
    },
    /// Tool execution completed
    ToolEnd {
        name: String,
        output: String,
        is_error: bool,
        duration_ms: u64,
    },
    /// A tool needs user approval (Write/Execute permission)
    ToolApprovalNeeded {
        tool_name: String,
        tool_input: String,
        permission: PermissionLevel,
    },
    /// Informational (model name, RAG status, etc.)
    RouteInfo(String),
    /// Agent finished — no more tool calls
    Done(GenMetrics),
    /// Error during agent loop
    Error(String),
}

/// Commands sent from TUI to the agent loop
#[derive(Debug, Clone)]
pub enum AgentCommand {
    /// User approved the pending tool execution
    Approve,
    /// User denied the pending tool execution
    Deny,
    /// User cancelled the entire generation
    Cancel,
}

const MAX_TURNS: usize = 25;

/// Run the agent loop with native tool calling.
///
/// Uses Ollama's function calling API — the model returns structured tool_calls
/// instead of text-based ```tool blocks. This makes tool detection 100% reliable.
pub async fn run_agent_loop(
    client: Arc<OllamaClient>,
    model: String,
    system_prompt: String,
    user_prompt: String,
    registry: Arc<ToolRegistry>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
    mut command_rx: mpsc::UnboundedReceiver<AgentCommand>,
) {
    let tool_ctx = ToolContext::new();
    let cwd = tool_ctx.working_dir.display().to_string();

    // Build native tool schemas for Ollama API
    let tool_defs: Vec<(String, String, String)> = registry
        .names()
        .iter()
        .filter_map(|name| {
            registry.get(name).map(|t| {
                (
                    t.name().to_string(),
                    t.description().to_string(),
                    t.parameters_hint().to_string(),
                )
            })
        })
        .collect();
    let native_tools = build_native_tool_schemas(&tool_defs);

    // System prompt with cwd
    let full_system = format!(
        "{system_prompt}\n## Environment\nWorking directory: {cwd}\n\n"
    );

    let mut conversation = vec![
        ConversationMessage::system(full_system),
        ConversationMessage::user(user_prompt),
    ];

    for _turn in 0..MAX_TURNS {
        // Check for cancellation
        if let Ok(AgentCommand::Cancel) = command_rx.try_recv() {
            return;
        }

        // Call Ollama with native tools
        let response = match client
            .chat_with_native_tools(&model, &conversation, &native_tools)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = event_tx.send(AgentEvent::Error(format!("Ollama error: {e}")));
                return;
            }
        };

        // Check if model returned tool calls
        if !response.tool_calls.is_empty() {
            // Show any text the model produced before calling tools
            if !response.content.trim().is_empty() {
                let _ = event_tx.send(AgentEvent::TextResponse(response.content.clone()));
            }

            // Add assistant message to conversation
            conversation.push(ConversationMessage::assistant(&response.content));

            // Execute each tool call
            for tc in &response.tool_calls {
                let tool_input = serde_json::json!(tc.arguments);
                let our_call = ToolCall {
                    name: tc.name.clone(),
                    input: tool_input.clone(),
                };

                // Check permission
                let tool_ref = registry.get(&tc.name);
                let permission = tool_ref
                    .map(|t| t.permission_level())
                    .unwrap_or(PermissionLevel::Execute);

                let approved = match permission {
                    PermissionLevel::ReadOnly => true,
                    _ => {
                        let input_summary = summarize_input(&tool_input);
                        let _ = event_tx.send(AgentEvent::ToolApprovalNeeded {
                            tool_name: tc.name.clone(),
                            tool_input: input_summary,
                            permission,
                        });
                        match command_rx.recv().await {
                            Some(AgentCommand::Approve) => true,
                            Some(AgentCommand::Cancel) => return,
                            _ => false,
                        }
                    }
                };

                if approved {
                    let input_summary = summarize_input(&tool_input);
                    let _ = event_tx.send(AgentEvent::ToolStart {
                        name: tc.name.clone(),
                        input_summary,
                    });

                    let start = Instant::now();
                    let reg = Arc::clone(&registry);
                    let ctx = tool_ctx.clone();
                    let call = our_call.clone();

                    let result = tokio::task::spawn_blocking(move || {
                        reg.execute(&call, &ctx)
                    })
                    .await;

                    let duration_ms = start.elapsed().as_millis() as u64;

                    let tool_result = match result {
                        Ok(Ok(r)) => r,
                        Ok(Err(e)) => ToolResult::error(format!("Tool error: {e}")),
                        Err(e) => ToolResult::error(format!("Task panic: {e}")),
                    };

                    let _ = event_tx.send(AgentEvent::ToolEnd {
                        name: tc.name.clone(),
                        output: truncate(&tool_result.output, 2000),
                        is_error: tool_result.is_error,
                        duration_ms,
                    });

                    // Feed result back — use "tool" role for models that support it
                    conversation.push(ConversationMessage::tool(format!(
                        "{}", tool_result.output
                    )));
                } else {
                    conversation.push(ConversationMessage::tool(
                        "Permission denied by user.".to_string(),
                    ));
                }
            }
            // Loop back — model will see tool results
            continue;
        }

        // No tool calls — this is the final response
        if !response.content.trim().is_empty() {
            let _ = event_tx.send(AgentEvent::TextResponse(response.content.clone()));
        }
        conversation.push(ConversationMessage::assistant(&response.content));
        let _ = event_tx.send(AgentEvent::Done(response.metrics));
        return;
    }

    let _ = event_tx.send(AgentEvent::Error(format!(
        "Agent reached maximum turns ({MAX_TURNS})"
    )));
}

fn summarize_input(input: &serde_json::Value) -> String {
    let s = serde_json::to_string(input).unwrap_or_default();
    if s.len() > 200 {
        format!("{}...", &s[..200])
    } else {
        s
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = (0..=max)
            .rev()
            .find(|&i| s.is_char_boundary(i))
            .unwrap_or(0);
        format!("{}...\n[truncated, {} bytes total]", &s[..end], s.len())
    }
}
