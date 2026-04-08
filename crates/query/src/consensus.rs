//! The Council — Dual-inference consensus with security diff
//!
//! On HighEnd hardware: fires two models in parallel and compares outputs.
//! On lower tiers: sequential comparison with the audit model.

use sovereign_core::PerformanceTier;
use sovereign_api::OllamaClient;
use anyhow::Result;

/// Result of a Council deliberation
#[derive(Debug)]
pub struct CouncilVerdict {
    pub dev_response: String,
    pub audit_response: String,
    pub consensus: ConsensusLevel,
    pub diff_summary: String,
}

/// Level of agreement between the two models
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusLevel {
    /// Models agree on the approach
    Aligned,
    /// Minor differences — informational
    MinorDivergence,
    /// Significant disagreement — buddy enters Confused state
    Conflicted,
}

impl std::fmt::Display for ConsensusLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConsensusLevel::Aligned => write!(f, "ALIGNED"),
            ConsensusLevel::MinorDivergence => write!(f, "MINOR DIVERGENCE"),
            ConsensusLevel::Conflicted => write!(f, "CONFLICTED"),
        }
    }
}

/// The Council orchestrates dual-inference comparison
pub struct Council {
    client: OllamaClient,
}

impl Council {
    pub fn new() -> Self {
        Self {
            client: OllamaClient::new(),
        }
    }

    /// Run dual inference — parallel on HighEnd, sequential on lower tiers
    pub async fn deliberate(
        &self,
        prompt: &str,
        dev_model: &str,
        audit_model: &str,
        tier: PerformanceTier,
    ) -> Result<CouncilVerdict> {
        let (dev_response, audit_response) = if tier >= PerformanceTier::Medium {
            // Parallel inference on capable hardware
            let dev_fut = self.client.generate(dev_model, prompt);
            let audit_prompt = format!(
                "Review this request from a security perspective. \
                 Identify potential vulnerabilities in any suggested approach.\n\n{prompt}"
            );
            let audit_fut = self.client.generate(audit_model, &audit_prompt);

            let (dev, audit) = tokio::join!(dev_fut, audit_fut);
            (dev?, audit?)
        } else {
            // Sequential on lower-tier hardware
            let dev = self.client.generate(dev_model, prompt).await?;
            let audit_prompt = format!(
                "Review this code/approach for security issues:\n\n{dev}"
            );
            let audit = self.client.generate(audit_model, &audit_prompt).await?;
            (dev, audit)
        };

        let consensus = analyze_consensus(&dev_response, &audit_response);
        let diff_summary = generate_diff_summary(&dev_response, &audit_response, &consensus);

        Ok(CouncilVerdict {
            dev_response,
            audit_response,
            consensus,
            diff_summary,
        })
    }
}

/// Analyze how much the two responses agree
fn analyze_consensus(dev: &str, audit: &str) -> ConsensusLevel {
    let dev_lower = dev.to_lowercase();
    let audit_lower = audit.to_lowercase();

    // Check for strong disagreement signals in the audit
    let conflict_signals = [
        "vulnerability", "insecure", "dangerous", "should not",
        "do not use", "critical issue", "injection", "exploit",
        "reject", "unsafe", "flaw",
    ];

    let conflict_count = conflict_signals.iter()
        .filter(|s| audit_lower.contains(*s))
        .count();

    // Check for approval signals
    let approval_signals = [
        "looks good", "secure", "no issues", "safe",
        "correct approach", "well done", "approved",
    ];

    let approval_count = approval_signals.iter()
        .filter(|s| audit_lower.contains(*s))
        .count();

    if conflict_count >= 3 {
        ConsensusLevel::Conflicted
    } else if conflict_count >= 1 && approval_count == 0 {
        ConsensusLevel::MinorDivergence
    } else {
        ConsensusLevel::Aligned
    }
}

/// Generate a human-readable diff summary
fn generate_diff_summary(dev: &str, audit: &str, consensus: &ConsensusLevel) -> String {
    let dev_lines = dev.lines().count();
    let audit_lines = audit.lines().count();

    match consensus {
        ConsensusLevel::Aligned => {
            format!(
                "Council: ALIGNED. Dev ({dev_lines} lines) and Audit ({audit_lines} lines) agree."
            )
        }
        ConsensusLevel::MinorDivergence => {
            format!(
                "Council: MINOR DIVERGENCE. Audit flagged potential concerns.\n\
                 Dev: {dev_lines} lines | Audit: {audit_lines} lines\n\
                 Review the audit notes below."
            )
        }
        ConsensusLevel::Conflicted => {
            format!(
                "Council: CONFLICTED! Models disagree significantly.\n\
                 Dev: {dev_lines} lines | Audit: {audit_lines} lines\n\
                 Manual review required. Familiar is confused."
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_consensus_aligned() {
        let consensus = analyze_consensus(
            "Here's a function that adds two numbers",
            "The code looks good and is safe to use",
        );
        assert_eq!(consensus, ConsensusLevel::Aligned);
    }

    #[test]
    fn test_consensus_conflicted() {
        let consensus = analyze_consensus(
            "Use eval() to parse the user input",
            "This is a critical issue! Eval is dangerous and creates an injection vulnerability. Do not use eval, it's insecure and exploitable.",
        );
        assert_eq!(consensus, ConsensusLevel::Conflicted);
    }

    #[test]
    fn test_consensus_minor() {
        let consensus = analyze_consensus(
            "Connect to the database with password in env",
            "There is a potential vulnerability if the env var is not set",
        );
        assert_eq!(consensus, ConsensusLevel::MinorDivergence);
    }

    #[test]
    fn test_diff_summary_format() {
        let summary = generate_diff_summary("a\nb\nc", "d\ne", &ConsensusLevel::Conflicted);
        assert!(summary.contains("CONFLICTED"));
        assert!(summary.contains("3 lines"));
    }
}
