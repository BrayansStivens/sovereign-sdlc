//! Agent Loop v0.5.0 — Streaming tool-calling agent
//!
//! The core conversation loop: streams chat from Ollama, detects ```tool
//! blocks, executes tools via ToolRegistry, feeds results back, loops.

use sovereign_api::{GenMetrics, OllamaClient, StreamChunk};
use sovereign_core::ConversationMessage;
use sovereign_tools::{
    PermissionLevel, ToolCall, ToolContext, ToolRegistry, ToolResult, parse_tool_call,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::mpsc;

/// Events sent from the agent loop to the TUI
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Incremental text token from streaming response
    StreamDelta(String),
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

/// Run the agent loop: stream → parse tools → execute → loop.
///
/// This function is meant to be `tokio::spawn`ed from the coordinator.
/// It communicates with the TUI exclusively via channels.
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

    // Inject cwd into system prompt so the model knows where it is
    let full_system = format!(
        "{system_prompt}\n## Environment\nWorking directory: {cwd}\n\n"
    );

    let mut conversation = vec![
        ConversationMessage::system(full_system),
        ConversationMessage::user(user_prompt),
    ];

    for turn in 0..MAX_TURNS {
        // Check for cancellation before each turn
        if let Ok(AgentCommand::Cancel) = command_rx.try_recv() {
            return;
        }

        // ── Stream chat response ──
        let stream_rx = match client.chat_stream(&model, &conversation).await {
            Ok(rx) => rx,
            Err(e) => {
                let _ = event_tx.send(AgentEvent::Error(format!("Ollama error: {e}")));
                return;
            }
        };

        let mut full_response = String::new();
        let mut final_metrics = GenMetrics::default();

        let stream_result = stream_with_cancel(stream_rx, &mut command_rx, &event_tx).await;

        match stream_result {
            StreamResult::Completed { response, metrics } => {
                full_response = response;
                final_metrics = metrics;
            }
            StreamResult::Cancelled => return,
            StreamResult::Error(e) => {
                let _ = event_tx.send(AgentEvent::Error(e));
                return;
            }
        }

        // ── Check for tool call in accumulated response ──
        if let Some((tool_call, _text_before)) = parse_tool_call(&full_response) {
            // Add assistant's response to conversation
            conversation.push(ConversationMessage::assistant(&full_response));

            // Check permission level
            let tool_ref = registry.get(&tool_call.name);
            let permission = tool_ref
                .map(|t| t.permission_level())
                .unwrap_or(PermissionLevel::Execute);

            let approved = match permission {
                PermissionLevel::ReadOnly => true, // Auto-approve
                _ => {
                    // Ask TUI for approval
                    let input_summary = summarize_tool_input(&tool_call);
                    let _ = event_tx.send(AgentEvent::ToolApprovalNeeded {
                        tool_name: tool_call.name.clone(),
                        tool_input: input_summary,
                        permission,
                    });

                    // Wait for user decision
                    match command_rx.recv().await {
                        Some(AgentCommand::Approve) => true,
                        Some(AgentCommand::Cancel) => return,
                        _ => false,
                    }
                }
            };

            if approved {
                let input_summary = summarize_tool_input(&tool_call);
                let _ = event_tx.send(AgentEvent::ToolStart {
                    name: tool_call.name.clone(),
                    input_summary,
                });

                // Execute tool on blocking thread
                let start = Instant::now();
                let registry_clone = Arc::clone(&registry);
                let ctx_clone = tool_ctx.clone();
                let call_clone = tool_call.clone();

                let tool_result = tokio::task::spawn_blocking(move || {
                    registry_clone.execute(&call_clone, &ctx_clone)
                })
                .await;

                let duration_ms = start.elapsed().as_millis() as u64;

                let result = match tool_result {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => ToolResult::error(format!("Tool error: {e}")),
                    Err(e) => ToolResult::error(format!("Task panic: {e}")),
                };

                let _ = event_tx.send(AgentEvent::ToolEnd {
                    name: tool_call.name.clone(),
                    output: truncate_for_display(&result.output, 2000),
                    is_error: result.is_error,
                    duration_ms,
                });

                // Feed result back to conversation
                // Use User role with prefix as fallback (some models don't understand Tool role)
                conversation.push(ConversationMessage::user(format!(
                    "[Tool Result for {}]:\n{}",
                    tool_call.name, result.output
                )));

                // Continue loop — LLM will see the tool result
                continue;
            } else {
                // User denied — tell LLM
                conversation.push(ConversationMessage::user(
                    "[Tool Result]: Permission denied by user.".to_string(),
                ));
                continue;
            }
        } else {
            // No tool call — response is final
            conversation.push(ConversationMessage::assistant(&full_response));
            final_metrics.response = full_response;
            let _ = event_tx.send(AgentEvent::Done(final_metrics));
            return;
        }
    }

    let _ = event_tx.send(AgentEvent::Error(format!(
        "Agent reached maximum turns ({MAX_TURNS})"
    )));
}

// ── Internal helpers ──

enum StreamResult {
    Completed {
        response: String,
        metrics: GenMetrics,
    },
    Cancelled,
    Error(String),
}

/// Consume the stream, forwarding deltas to TUI, while checking for cancel commands.
async fn stream_with_cancel(
    mut stream_rx: mpsc::UnboundedReceiver<StreamChunk>,
    command_rx: &mut mpsc::UnboundedReceiver<AgentCommand>,
    event_tx: &mpsc::UnboundedSender<AgentEvent>,
) -> StreamResult {
    let mut full_response = String::new();
    let mut metrics = GenMetrics::default();

    loop {
        tokio::select! {
            biased;

            // Check for cancel command
            cmd = command_rx.recv() => {
                match cmd {
                    Some(AgentCommand::Cancel) | None => return StreamResult::Cancelled,
                    _ => {} // Ignore other commands during streaming
                }
            }

            // Receive stream chunks
            chunk = stream_rx.recv() => {
                match chunk {
                    Some(StreamChunk::Delta(text)) => {
                        full_response.push_str(&text);
                        if event_tx.send(AgentEvent::StreamDelta(text)).is_err() {
                            return StreamResult::Cancelled;
                        }
                    }
                    Some(StreamChunk::Done(m)) => {
                        metrics = m;
                        metrics.response = full_response.clone();
                        return StreamResult::Completed { response: full_response, metrics };
                    }
                    Some(StreamChunk::Error(e)) => {
                        return StreamResult::Error(e);
                    }
                    None => {
                        // Stream ended without Done — treat as complete
                        metrics.response = full_response.clone();
                        return StreamResult::Completed { response: full_response, metrics };
                    }
                }
            }
        }
    }
}

/// Create a short summary of tool input for display
fn summarize_tool_input(call: &ToolCall) -> String {
    let input_str = serde_json::to_string(&call.input).unwrap_or_default();
    if input_str.len() > 200 {
        format!("{}...", &input_str[..200])
    } else {
        input_str
    }
}

/// Truncate output for TUI display
fn truncate_for_display(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let end = (0..=max).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
        format!("{}...\n[truncated, {} bytes total]", &s[..end], s.len())
    }
}
