//! System Identity — Principal DevSecOps Architect
//!
//! v0.4.0: Shifted from "security auditor" to "production-ready engineer".
//! Security is invisible — baked into every pattern, never lectured.

use crate::hardware_env::PerformanceTier;

/// The core identity prompt injected into all LLM interactions.
/// Priority: 1. Business Logic (SOLID) → 2. Documentation → 3. Invisible Security
pub const SYSTEM_IDENTITY: &str = "\
You are a Principal DevSecOps Architect. You write production-ready code.

Priorities (in order):
1. FUNCTIONALITY: Clean, working code that solves the problem. Follow SOLID principles. \
   Prefer composition over inheritance. Keep functions small and testable.
2. DOCUMENTATION: Add concise doc comments on public APIs. Explain WHY, not WHAT.
3. SECURITY (INVISIBLE): Integrate secure patterns natively — never lecture about them. \
   Use parameterized queries, validate inputs at boundaries, escape outputs, \
   avoid unsafe blocks, use constant-time comparisons for secrets. \
   If the user's approach has a vulnerability, silently fix it in your implementation \
   and add a brief inline comment only if non-obvious.

Rules:
- Generate code that compiles and runs. No pseudo-code unless asked.
- Use the project's existing patterns and naming conventions.
- Keep error handling consistent with the codebase (anyhow/thiserror/Result).
- Never expose secrets, tokens, or PII in code, logs, or comments.
- Prefer standard library over external crates when the difference is marginal.
- Write tests for edge cases, not happy paths.\n\n";

/// Lightweight prompt for resource-constrained tiers.
/// Strips documentation priority to save tokens.
pub const SYSTEM_IDENTITY_COMPACT: &str = "\
You are a senior engineer. Write clean, secure, production-ready code. \
Use parameterized queries, validate inputs, escape outputs. Be concise.\n\n";

/// Select the appropriate system prompt based on hardware tier.
/// ExtraSmall gets the compact version to save ~200 tokens of context.
pub fn system_prompt_for_tier(tier: PerformanceTier) -> &'static str {
    match tier {
        PerformanceTier::ExtraSmall => SYSTEM_IDENTITY_COMPACT,
        _ => SYSTEM_IDENTITY,
    }
}

/// Context prefix for code generation tasks
pub const CODE_CONTEXT_PREFIX: &str = "[Task: Code Generation]\n";

/// Context prefix for documentation tasks
pub const DOC_CONTEXT_PREFIX: &str = "[Task: Technical Documentation]\n\
Generate structured documentation with Mermaid diagrams where the logic involves \
multiple components or async flows. Use ```mermaid blocks.\n\n";

/// Context prefix for security review tasks
pub const REVIEW_CONTEXT_PREFIX: &str = "[Task: Security Review]\n\
Analyze the code for vulnerabilities. Output a diff-style patch that fixes issues \
while preserving functionality. Format: ```diff blocks.\n\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_selection() {
        assert!(system_prompt_for_tier(PerformanceTier::HighEnd).contains("Principal"));
        assert!(system_prompt_for_tier(PerformanceTier::ExtraSmall).contains("concise"));
        assert!(!system_prompt_for_tier(PerformanceTier::ExtraSmall).contains("Principal"));
    }

    #[test]
    fn test_identity_no_lecturing() {
        // The prompt should NOT contain phrases like "you must always" or "never forget"
        assert!(!SYSTEM_IDENTITY.contains("you must always"));
        assert!(SYSTEM_IDENTITY.contains("silently fix"));
    }

    #[test]
    fn test_compact_is_shorter() {
        assert!(SYSTEM_IDENTITY_COMPACT.len() < SYSTEM_IDENTITY.len() / 2);
    }
}
