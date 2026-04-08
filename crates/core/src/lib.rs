pub mod hardware_env;
pub mod memdir;
pub mod grimoire;
pub mod history;
pub mod system_prompt;
pub mod docs;
pub mod diff;

pub use hardware_env::{
    HardwareEnv, ModelWeight, PerformanceTier, Platform, SafeLoadResult, ModelRecommendation,
};

pub use memdir::{
    VectorStore, DocChunk, SearchResult, IndexStats, IndexProgress,
    scan_project, batch_size_for_tier, tui_refresh_ms,
    EMBEDDING_DIM, EMBEDDING_MODEL,
};

pub use grimoire::Grimoire;
pub use history::{Chronicle, SessionRecord};
pub use system_prompt::{
    SYSTEM_IDENTITY, SYSTEM_IDENTITY_COMPACT, system_prompt_for_tier,
    CODE_CONTEXT_PREFIX, DOC_CONTEXT_PREFIX, REVIEW_CONTEXT_PREFIX,
};
pub use docs::{analyze_project, architecture_prompt, module_doc_prompt, ProjectStructure, ModuleInfo};
pub use diff::{
    FileDiff, DiffLine, LineTag, ProposedAction, CommandResult,
    classify_command_risk, apply_edit, execute_command,
};
