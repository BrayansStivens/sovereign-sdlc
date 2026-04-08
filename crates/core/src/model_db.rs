//! Model Database — Offline catalog of LLM requirements
//!
//! Built from CanIRun.ai data. Maps model name → RAM requirements per quantization.
//! Used for onboarding: recommends what to install based on detected hardware.

use crate::hardware_env::PerformanceTier;

/// A model entry in the catalog
#[derive(Debug, Clone)]
pub struct ModelSpec {
    pub name: &'static str,
    pub params_b: f64,
    pub context_k: u32,
    pub vram_q4_gb: f64,
    pub vram_q8_gb: f64,
    pub category: ModelCategory,
    pub ollama_tag: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCategory {
    Code,
    Reasoning,
    Chat,
    Embedding,
}

impl std::fmt::Display for ModelCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelCategory::Code => write!(f, "Code"),
            ModelCategory::Reasoning => write!(f, "Reasoning"),
            ModelCategory::Chat => write!(f, "Chat"),
            ModelCategory::Embedding => write!(f, "Embedding"),
        }
    }
}

/// Full model catalog (data from CanIRun.ai + Ollama library)
pub const MODEL_CATALOG: &[ModelSpec] = &[
    // ── Embedding (required for RAG) ──
    ModelSpec {
        name: "nomic-embed-text",
        params_b: 0.137,
        context_k: 8,
        vram_q4_gb: 0.3,
        vram_q8_gb: 0.3,
        category: ModelCategory::Embedding,
        ollama_tag: "nomic-embed-text",
        description: "Embeddings for RAG (required)",
    },
    // ── Extra Small (1-3B) ──
    ModelSpec {
        name: "Llama 3.2 1B",
        params_b: 1.0,
        context_k: 128,
        vram_q4_gb: 0.8,
        vram_q8_gb: 1.2,
        category: ModelCategory::Chat,
        ollama_tag: "llama3.2:1b",
        description: "Ultra-light router/chat",
    },
    ModelSpec {
        name: "Llama 3.2 3B",
        params_b: 3.0,
        context_k: 128,
        vram_q4_gb: 2.0,
        vram_q8_gb: 3.4,
        category: ModelCategory::Chat,
        ollama_tag: "llama3.2:3b",
        description: "Light general chat",
    },
    ModelSpec {
        name: "Qwen 2.5 Coder 3B",
        params_b: 3.0,
        context_k: 32,
        vram_q4_gb: 2.0,
        vram_q8_gb: 3.4,
        category: ModelCategory::Code,
        ollama_tag: "qwen2.5-coder:3b",
        description: "Light code generation",
    },
    ModelSpec {
        name: "Phi-4 Mini",
        params_b: 3.8,
        context_k: 16,
        vram_q4_gb: 2.5,
        vram_q8_gb: 4.2,
        category: ModelCategory::Reasoning,
        ollama_tag: "phi-4:mini",
        description: "Compact reasoning",
    },
    // ── Small (7B) ──
    ModelSpec {
        name: "Qwen 2.5 7B",
        params_b: 7.0,
        context_k: 128,
        vram_q4_gb: 4.1,
        vram_q8_gb: 7.7,
        category: ModelCategory::Chat,
        ollama_tag: "qwen2.5:7b",
        description: "Strong multilingual + code",
    },
    ModelSpec {
        name: "Qwen 2.5 Coder 7B",
        params_b: 7.0,
        context_k: 128,
        vram_q4_gb: 4.1,
        vram_q8_gb: 7.7,
        category: ModelCategory::Code,
        ollama_tag: "qwen2.5-coder:7b",
        description: "Dedicated code model",
    },
    ModelSpec {
        name: "DeepSeek R1 7B",
        params_b: 7.0,
        context_k: 64,
        vram_q4_gb: 4.1,
        vram_q8_gb: 7.7,
        category: ModelCategory::Reasoning,
        ollama_tag: "deepseek-r1:7b",
        description: "Reasoning + math",
    },
    ModelSpec {
        name: "Mistral 7B",
        params_b: 7.0,
        context_k: 32,
        vram_q4_gb: 4.1,
        vram_q8_gb: 7.7,
        category: ModelCategory::Chat,
        ollama_tag: "mistral:7b",
        description: "Fast general purpose",
    },
    // ── Medium (14B) ──
    ModelSpec {
        name: "Qwen 2.5 Coder 14B",
        params_b: 14.0,
        context_k: 128,
        vram_q4_gb: 8.2,
        vram_q8_gb: 15.5,
        category: ModelCategory::Code,
        ollama_tag: "qwen2.5-coder:14b",
        description: "Best open-source coder",
    },
    ModelSpec {
        name: "DeepSeek R1 14B",
        params_b: 14.0,
        context_k: 64,
        vram_q4_gb: 8.2,
        vram_q8_gb: 15.5,
        category: ModelCategory::Reasoning,
        ollama_tag: "deepseek-r1:14b",
        description: "Deep reasoning + audit",
    },
    // ── Large (32B+) ──
    ModelSpec {
        name: "Qwen 2.5 Coder 32B",
        params_b: 32.0,
        context_k: 128,
        vram_q4_gb: 18.5,
        vram_q8_gb: 34.0,
        category: ModelCategory::Code,
        ollama_tag: "qwen2.5-coder:32b",
        description: "Top-tier code (needs 20GB+)",
    },
    ModelSpec {
        name: "DeepSeek R1 32B",
        params_b: 32.0,
        context_k: 64,
        vram_q4_gb: 18.5,
        vram_q8_gb: 34.0,
        category: ModelCategory::Reasoning,
        ollama_tag: "deepseek-r1:32b",
        description: "Advanced reasoning (needs 20GB+)",
    },
];

/// Filter catalog to models that fit in the available RAM budget
pub fn models_for_budget(budget_gb: f64) -> Vec<&'static ModelSpec> {
    MODEL_CATALOG.iter()
        .filter(|m| m.vram_q4_gb <= budget_gb)
        .collect()
}

/// Get recommended setup for a tier (dev + audit + embedding)
pub fn recommended_setup(tier: PerformanceTier) -> RecommendedSetup {
    match tier {
        PerformanceTier::HighEnd => RecommendedSetup {
            dev: "qwen2.5-coder:14b",
            audit: "deepseek-r1:14b",
            embed: "nomic-embed-text",
            total_gb: 16.7,  // 8.2 + 8.2 + 0.3
        },
        PerformanceTier::Medium => RecommendedSetup {
            dev: "qwen2.5-coder:7b",
            audit: "deepseek-r1:7b",
            embed: "nomic-embed-text",
            total_gb: 8.5,
        },
        PerformanceTier::Small => RecommendedSetup {
            dev: "qwen2.5:7b",
            audit: "phi-4:mini",
            embed: "nomic-embed-text",
            total_gb: 6.9,
        },
        PerformanceTier::ExtraSmall => RecommendedSetup {
            dev: "llama3.2:3b",
            audit: "phi-4:mini",
            embed: "nomic-embed-text",
            total_gb: 4.8,
        },
    }
}

#[derive(Debug, Clone)]
pub struct RecommendedSetup {
    pub dev: &'static str,
    pub audit: &'static str,
    pub embed: &'static str,
    pub total_gb: f64,
}

impl RecommendedSetup {
    /// Generate ollama pull commands
    pub fn install_commands(&self) -> Vec<String> {
        vec![
            format!("ollama pull {}", self.dev),
            format!("ollama pull {}", self.audit),
            format!("ollama pull {}", self.embed),
        ]
    }

    /// Check which models from this setup are missing
    pub fn missing_models(&self, installed: &[String]) -> Vec<&'static str> {
        let mut missing = Vec::new();
        for model in &[self.dev, self.audit, self.embed] {
            let base = model.split(':').next().unwrap_or(model);
            if !installed.iter().any(|m| m.starts_with(base)) {
                missing.push(*model);
            }
        }
        missing
    }
}

/// Format the onboarding message shown at startup
pub fn onboarding_message(
    tier: PerformanceTier,
    installed: &[String],
    budget_gb: f64,
) -> Option<String> {
    let setup = recommended_setup(tier);
    let missing = setup.missing_models(installed);

    if missing.is_empty() {
        return None; // All good, no onboarding needed
    }

    let mut msg = String::new();

    if installed.is_empty() {
        msg.push_str("No models installed. You need at least one to start.\n\n");
    } else {
        msg.push_str(&format!(
            "Missing {} model(s) for optimal {} setup.\n\n",
            missing.len(), tier,
        ));
    }

    msg.push_str(&format!("Your hardware: {tier} ({budget_gb:.0} GB available)\n\n"));
    msg.push_str("Recommended setup:\n");
    msg.push_str(&format!("  Dev:   {} ({:.1} GB)\n", setup.dev,
        MODEL_CATALOG.iter().find(|m| m.ollama_tag == setup.dev).map(|m| m.vram_q4_gb).unwrap_or(0.0)));
    msg.push_str(&format!("  Audit: {} ({:.1} GB)\n", setup.audit,
        MODEL_CATALOG.iter().find(|m| m.ollama_tag == setup.audit).map(|m| m.vram_q4_gb).unwrap_or(0.0)));
    msg.push_str(&format!("  Embed: {} ({:.1} GB)\n", setup.embed, 0.3));

    if !missing.is_empty() {
        msg.push_str("\nRun these commands to install:\n");
        for m in &missing {
            msg.push_str(&format!("  ollama pull {m}\n"));
        }
    }

    // Also show what else fits
    let viable = models_for_budget(budget_gb);
    let extra: Vec<&&ModelSpec> = viable.iter()
        .filter(|m| m.ollama_tag != setup.dev && m.ollama_tag != setup.audit
            && m.ollama_tag != setup.embed && m.category != ModelCategory::Embedding)
        .collect();

    if !extra.is_empty() {
        msg.push_str("\nOther models that fit your hardware:\n");
        for m in extra.iter().take(5) {
            msg.push_str(&format!("  {} ({:.1} GB) - {}\n", m.ollama_tag, m.vram_q4_gb, m.description));
        }
    }

    Some(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_models_for_budget() {
        let small = models_for_budget(4.0);
        assert!(small.iter().all(|m| m.vram_q4_gb <= 4.0));
        assert!(small.iter().any(|m| m.ollama_tag == "llama3.2:3b"));
    }

    #[test]
    fn test_models_for_large_budget() {
        let large = models_for_budget(20.0);
        assert!(large.iter().any(|m| m.ollama_tag == "qwen2.5-coder:14b"));
    }

    #[test]
    fn test_recommended_setup_highend() {
        let setup = recommended_setup(PerformanceTier::HighEnd);
        assert!(setup.dev.contains("14b"));
        assert!(setup.audit.contains("14b"));
    }

    #[test]
    fn test_missing_models() {
        let setup = recommended_setup(PerformanceTier::Small);
        let installed = vec!["qwen2.5:7b".to_string()];
        let missing = setup.missing_models(&installed);
        assert!(!missing.contains(&"qwen2.5:7b"));
        assert!(missing.contains(&"nomic-embed-text"));
    }

    #[test]
    fn test_no_onboarding_when_complete() {
        let installed = vec![
            "qwen2.5-coder:14b".into(),
            "deepseek-r1:14b".into(),
            "nomic-embed-text:latest".into(),
        ];
        let msg = onboarding_message(PerformanceTier::HighEnd, &installed, 20.0);
        assert!(msg.is_none());
    }

    #[test]
    fn test_onboarding_when_empty() {
        let msg = onboarding_message(PerformanceTier::Small, &[], 10.0);
        assert!(msg.is_some());
        let text = msg.unwrap();
        assert!(text.contains("ollama pull"));
        assert!(text.contains("No models installed"));
    }

    #[test]
    fn test_install_commands() {
        let setup = recommended_setup(PerformanceTier::ExtraSmall);
        let cmds = setup.install_commands();
        assert_eq!(cmds.len(), 3);
        assert!(cmds[0].starts_with("ollama pull"));
    }
}
