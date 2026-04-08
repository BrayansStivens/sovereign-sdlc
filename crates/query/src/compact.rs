//! Synapse — Context window management and compression
//!
//! Tracks token usage, compresses old messages when approaching limits,
//! and adapts to hardware tier.

use sovereign_core::PerformanceTier;

/// Context limits per hardware tier (in estimated tokens)
pub fn context_limit(tier: PerformanceTier) -> usize {
    match tier {
        PerformanceTier::HighEnd    => 65536,  // 64k tokens
        PerformanceTier::Medium     => 32768,  // 32k tokens
        PerformanceTier::Small      => 16384,  // 16k tokens
        PerformanceTier::ExtraSmall => 8192,   // 8k tokens
    }
}

/// Threshold percentage at which to trigger compression
const COMPRESS_THRESHOLD: f64 = 0.85;

/// Estimate token count from text (rough: 1 token ≈ 4 chars for English)
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4 + 1
}

/// Estimate total tokens in a chat history
pub fn total_tokens(messages: &[(String, String)]) -> usize {
    messages.iter().map(|(role, content)| {
        estimate_tokens(role) + estimate_tokens(content) + 4 // message overhead
    }).sum()
}

/// Check if compression is needed
pub fn needs_compression(messages: &[(String, String)], tier: PerformanceTier) -> bool {
    let limit = context_limit(tier);
    let current = total_tokens(messages);
    current as f64 > limit as f64 * COMPRESS_THRESHOLD
}

/// Determine which messages to compress (oldest first, keep recent)
/// Returns (messages_to_summarize, messages_to_keep)
pub fn split_for_compression(
    messages: &[(String, String)],
    tier: PerformanceTier,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let limit = context_limit(tier);
    let target_tokens = (limit as f64 * 0.5) as usize; // Keep 50% after compression

    let mut kept_tokens = 0;
    let mut keep_from = messages.len();

    // Walk backwards to find how many recent messages fit in target
    for (i, (role, content)) in messages.iter().enumerate().rev() {
        let msg_tokens = estimate_tokens(role) + estimate_tokens(content) + 4;
        if kept_tokens + msg_tokens > target_tokens {
            keep_from = i + 1;
            break;
        }
        kept_tokens += msg_tokens;
    }

    // Always keep at least the last 3 messages
    keep_from = keep_from.min(messages.len().saturating_sub(3));

    let to_summarize = messages[..keep_from].to_vec();
    let to_keep = messages[keep_from..].to_vec();

    (to_summarize, to_keep)
}

/// Generate a compression prompt for the LLM
pub fn compression_prompt(messages: &[(String, String)]) -> String {
    let mut chat_text = String::from(
        "Summarize the following conversation into a concise knowledge base. \
         Preserve: key decisions, code snippets discussed, security findings, \
         and any important context. Be factual and brief.\n\n"
    );

    for (role, content) in messages {
        chat_text.push_str(&format!("{role}: {}\n", truncate(content, 500)));
    }

    chat_text
}

/// Format the compression result as a system message
pub fn format_compressed_context(summary: &str) -> (String, String) {
    (
        "session-memory".to_string(),
        format!("[Session Knowledge Base (compressed)]:\n{summary}"),
    )
}

/// Status string for TUI display
pub fn compression_status(messages: &[(String, String)], tier: PerformanceTier) -> String {
    let current = total_tokens(messages);
    let limit = context_limit(tier);
    let pct = (current as f64 / limit as f64 * 100.0) as u16;

    let label = match tier {
        PerformanceTier::ExtraSmall => "Condensando neuronas para ahorrar RAM...",
        _ => "Compressing context...",
    };

    format!(
        "Context: {current}/{limit} tokens ({pct}%) {}",
        if needs_compression(messages, tier) { label } else { "" }
    )
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s }
    else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) { end -= 1; }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_limits() {
        assert_eq!(context_limit(PerformanceTier::HighEnd), 65536);
        assert_eq!(context_limit(PerformanceTier::ExtraSmall), 8192);
    }

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens("hello world"), 3); // 11/4+1
        assert_eq!(estimate_tokens(""), 1);
    }

    #[test]
    fn test_needs_compression() {
        let small_msgs: Vec<(String, String)> = vec![
            ("you".into(), "hi".into()),
        ];
        assert!(!needs_compression(&small_msgs, PerformanceTier::HighEnd));

        // Create a large chat
        let big_msgs: Vec<(String, String)> = (0..1000)
            .map(|i| ("you".into(), format!("message number {i} with some padding text to increase size")))
            .collect();
        assert!(needs_compression(&big_msgs, PerformanceTier::ExtraSmall));
    }

    #[test]
    fn test_split_preserves_recent() {
        let msgs: Vec<(String, String)> = (0..20)
            .map(|i| ("user".into(), format!("msg {i}")))
            .collect();

        let (to_summarize, to_keep) = split_for_compression(&msgs, PerformanceTier::ExtraSmall);
        assert!(!to_keep.is_empty());
        assert!(to_keep.len() >= 3); // Always keep at least 3
        assert_eq!(to_summarize.len() + to_keep.len(), 20);
    }

    #[test]
    fn test_compression_prompt_format() {
        let msgs = vec![
            ("you".into(), "how do I fix this SQL injection?".into()),
            ("sovereign".into(), "Use parameterized queries...".into()),
        ];
        let prompt = compression_prompt(&msgs);
        assert!(prompt.contains("knowledge base"));
        assert!(prompt.contains("SQL injection"));
    }

    #[test]
    fn test_status_display() {
        let msgs = vec![("you".into(), "hi".into())];
        let status = compression_status(&msgs, PerformanceTier::HighEnd);
        assert!(status.contains("tokens"));
        assert!(status.contains("65536"));
    }
}
