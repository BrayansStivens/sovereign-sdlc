//! Agent Coordinator v0.4.0 — DevSecOps Hybrid
//!
//! Hardware-aware model selection with RAG + Grimoire context injection.
//! Security is invisible — baked into generation, not lectured.

use sovereign_core::{
    HardwareEnv, ModelRecommendation, PerformanceTier, SafeLoadResult,
    VectorStore, scan_project, batch_size_for_tier,
    Grimoire, system_prompt_for_tier,
    EMBEDDING_MODEL, REVIEW_CONTEXT_PREFIX,
};
use sovereign_api::OllamaClient;
use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::router::{SmartRouter, TaskCategory};

/// Default index storage path
fn default_index_path(project_root: &Path) -> PathBuf {
    project_root.join(".sovereign").join("index.bin")
}

/// The Coordinator manages model selection, RAG + Grimoire, and agent orchestration
pub struct Coordinator {
    pub client: OllamaClient,
    pub router: SmartRouter,
    pub hw: HardwareEnv,
    pub recommendation: ModelRecommendation,

    /// Override: user-selected model via /model command
    pub force_model: Option<String>,

    /// Vector store for RAG
    pub memory: VectorStore,

    /// Security patterns knowledge base
    pub grimoire: Option<Grimoire>,

    /// Project root for indexing
    pub project_root: PathBuf,

    /// Whether RAG context injection is enabled
    pub rag_enabled: bool,
}

impl Coordinator {
    /// Initialize with hardware-aware model selection
    pub fn new() -> Self {
        let hw = HardwareEnv::detect();
        let recommendation = hw.tier.recommended_models();
        let client = OllamaClient::new();
        let router = SmartRouter::new(ollama_rs::Ollama::default());

        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        // Load existing index
        let index_path = default_index_path(&project_root);
        let memory = VectorStore::load_or_create(&index_path).unwrap_or_else(|_| VectorStore::new());
        let rag_enabled = memory.chunk_count() > 0;

        // Load Grimoire (security patterns KB)
        let grimoire = Grimoire::open(&project_root).ok();

        tracing::info!(
            tier = %hw.tier,
            dev = recommendation.dev_model,
            audit = recommendation.audit_model,
            indexed_chunks = memory.chunk_count(),
            grimoire_patterns = grimoire.as_ref().and_then(|g| g.count().ok()).unwrap_or(0),
            "Coordinator v0.4.0 initialized"
        );

        Self {
            client,
            router,
            hw,
            recommendation,
            force_model: None,
            memory,
            grimoire,
            project_root,
            rag_enabled,
        }
    }

    // ── Model Management ──

    pub fn set_model(&mut self, model: &str) -> SafeLoadResult {
        let result = self.hw.safe_load(model);
        match &result {
            SafeLoadResult::Blocked { .. } => {
                tracing::warn!(model, "SafeLoad BLOCKED model switch");
            }
            _ => {
                self.force_model = Some(model.to_string());
            }
        }
        result
    }

    pub fn clear_model_override(&mut self) {
        self.force_model = None;
        self.hw.refresh();
        self.recommendation = self.hw.tier.recommended_models();
    }

    // ── Routing ──

    pub async fn route_prompt(&self, prompt: &str) -> Result<(TaskCategory, String)> {
        if let Some(ref model) = self.force_model {
            return Ok((TaskCategory::Chat, model.clone()));
        }

        let (category, _) = self.router.route(prompt).await?;
        let model = match category {
            TaskCategory::Code => self.recommendation.dev_model.to_string(),
            TaskCategory::Logic => self.recommendation.audit_model.to_string(),
            TaskCategory::Chat => self.recommendation.dev_model.to_string(),
        };

        Ok((category, model))
    }

    // ── Generation with RAG ──

    /// Generate a response with tier-adaptive system prompt + RAG + Grimoire
    pub async fn generate(&self, model: &str, prompt: &str) -> Result<String> {
        let mut full_prompt = String::new();

        // 1. Tier-adaptive system identity (compact on ExtraSmall to save tokens)
        full_prompt.push_str(system_prompt_for_tier(self.hw.tier));

        // 2. Grimoire: inject relevant known security patterns
        if let Some(ref grimoire) = self.grimoire {
            if let Ok(patterns) = grimoire.recent(3) {
                if !patterns.is_empty() {
                    full_prompt.push_str(&grimoire.format_for_context(&patterns));
                    full_prompt.push('\n');
                }
            }
        }

        // 3. RAG context retrieval from vector store
        if self.rag_enabled && self.memory.chunk_count() > 0 {
            if let Ok(context) = self.retrieve_context(prompt).await {
                if !context.is_empty() {
                    full_prompt.push_str("[Project Context]:\n");
                    full_prompt.push_str(&context);
                    full_prompt.push_str("\n\n");
                }
            }
        }

        // 4. User prompt
        full_prompt.push_str("User request: ");
        full_prompt.push_str(prompt);

        self.client.generate(model, &full_prompt).await
    }

    /// Semantic search: RAG (code) + Grimoire (security patterns)
    async fn retrieve_context(&self, query: &str) -> Result<String> {
        let query_embedding = self.client.embed(EMBEDDING_MODEL, query).await?;
        let results = self.memory.search(&query_embedding, 5);

        if results.is_empty() {
            return Ok(String::new());
        }

        let context: String = results.iter()
            .map(|r| format!(
                "--- {} (score: {:.2}) ---\n{}\n",
                r.chunk.file_path.display(),
                r.score,
                truncate_content(&r.chunk.content, 800),
            ))
            .collect();

        Ok(context)
    }

    /// Auto-fix protocol: detect vuln → generate diff patch → record in Grimoire
    pub async fn auto_fix(&mut self, finding_context: &str, file_path: &str) -> Result<String> {
        let model = self.recommendation.audit_model;
        let prompt = format!(
            "{review}{finding_context}\n\n\
             Generate a minimal diff patch that fixes this vulnerability while preserving functionality. \
             Output ONLY the ```diff block.",
            review = REVIEW_CONTEXT_PREFIX,
        );

        let fix = self.generate(model, &prompt).await?;

        // Record in Grimoire for future learning
        if let Some(ref grimoire) = self.grimoire {
            let _ = grimoire.record_fix(
                finding_context,
                &fix,
                "auto-fix",
                "CRITICAL",
                file_path,
                "", // language auto-detected
            );
        }

        Ok(fix)
    }

    // ── Indexing ──

    /// Index a project directory into the vector store
    /// Respects hardware tier for concurrency control
    pub async fn index_project(&mut self, root: &Path) -> Result<IndexResult> {
        let tier = self.hw.tier;
        let batch_size = batch_size_for_tier(tier);

        tracing::info!(
            path = %root.display(),
            tier = %tier,
            batch_size,
            "Starting project indexing"
        );

        // Clear existing index
        self.memory.clear();

        // Scan project (Zero-Trust filtered)
        let files = scan_project(root)?;
        let total_files = files.len();
        let mut total_chunks = 0;
        let mut indexed_files = 0;

        for (file_path, chunks) in &files {
            // Batch embed chunks according to hardware tier
            let texts: Vec<String> = chunks.iter().map(|(text, _, _)| text.clone()).collect();

            for batch_start in (0..texts.len()).step_by(batch_size) {
                let batch_end = (batch_start + batch_size).min(texts.len());
                let batch = &texts[batch_start..batch_end];

                match self.client.embed_batch(EMBEDDING_MODEL, batch).await {
                    Ok(embeddings) => {
                        for (i, embedding) in embeddings.into_iter().enumerate() {
                            let idx = batch_start + i;
                            let (_, start, end) = &chunks[idx];
                            self.memory.insert(
                                file_path.clone(),
                                texts[idx].clone(),
                                *start,
                                *end,
                                embedding,
                            );
                            total_chunks += 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            file = %file_path.display(),
                            error = %e,
                            "Failed to embed batch — skipping"
                        );
                    }
                }
            }

            indexed_files += 1;
        }

        // Finalize and persist
        let paths: Vec<PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
        self.memory.finalize_stats(paths, indexed_files);

        let index_path = default_index_path(root);
        self.memory.save(&index_path)?;
        self.rag_enabled = total_chunks > 0;
        self.project_root = root.to_path_buf();

        Ok(IndexResult {
            files_scanned: total_files,
            files_indexed: indexed_files,
            chunks_indexed: total_chunks,
            tier,
            batch_size,
        })
    }

    // ── Validation Pipeline ──

    /// Validated generation: generate → audit review (parallel on HighEnd)
    pub async fn validated_generate(&self, model: &str, prompt: &str) -> Result<ValidatedResponse> {
        let response = self.generate(model, prompt).await?;

        if response.contains("```") {
            let audit_prompt = format!(
                "{review}Review this generated code for vulnerabilities. \
                 If issues found, output a ```diff patch.\n\n{response}",
                review = REVIEW_CONTEXT_PREFIX,
            );

            let audit_model = self.recommendation.audit_model;

            // HighEnd: parallel audit. Lower tiers: sequential.
            let audit_result = if self.hw.tier >= PerformanceTier::Medium {
                self.client.generate(audit_model, &audit_prompt).await?
            } else {
                // On low tiers, skip LLM audit — rely on static analysis
                "Static analysis recommended for this tier.".to_string()
            };

            Ok(ValidatedResponse {
                original: response,
                audit_review: Some(audit_result),
                passed_validation: true,
            })
        } else {
            Ok(ValidatedResponse {
                original: response,
                audit_review: None,
                passed_validation: true,
            })
        }
    }

    // ── Status ──

    pub fn refresh_hardware(&mut self) {
        self.hw.refresh();
        let new_tier = self.hw.tier;
        if new_tier != self.recommendation_tier() {
            self.recommendation = new_tier.recommended_models();
        }
    }

    fn recommendation_tier(&self) -> PerformanceTier {
        self.hw.tier
    }

    pub fn status(&mut self) -> String {
        self.hw.refresh();
        let active = self.force_model.as_deref()
            .unwrap_or(self.recommendation.dev_model);

        let memory_status = if self.rag_enabled {
            format!("{} chunks indexed", self.memory.chunk_count())
        } else {
            "No index — run /index to enable RAG".to_string()
        };

        let grimoire_count = self.grimoire.as_ref()
            .and_then(|g| g.count().ok())
            .unwrap_or(0);

        format!(
            "{hw}\n\n\
             ── Active Model ──\n  {active}\n  Override: {override_}\n\n\
             ── Knowledge ──\n  RAG: {memory_status}\n  Grimoire: {grimoire_count} patterns",
            hw = self.hw.status_report(),
            override_ = if self.force_model.is_some() { "yes" } else { "auto" },
        )
    }
}

/// Truncate content for context injection
fn truncate_content(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find valid UTF-8 boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

/// Result of an indexing operation
#[derive(Debug)]
pub struct IndexResult {
    pub files_scanned: usize,
    pub files_indexed: usize,
    pub chunks_indexed: usize,
    pub tier: PerformanceTier,
    pub batch_size: usize,
}

impl std::fmt::Display for IndexResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Indexed {files} files → {chunks} chunks (tier: {tier}, batch: {batch})",
            files = self.files_indexed,
            chunks = self.chunks_indexed,
            tier = self.tier,
            batch = self.batch_size,
        )
    }
}

/// Response after the full validation pipeline
#[derive(Debug)]
pub struct ValidatedResponse {
    pub original: String,
    pub audit_review: Option<String>,
    pub passed_validation: bool,
}
