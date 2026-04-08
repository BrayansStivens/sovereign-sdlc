pub mod hardware_env;
pub mod memdir;
pub mod grimoire;
pub mod history;
pub mod system_prompt;
pub mod docs;
pub mod diff;
pub mod permissions;
pub mod model_db;
pub mod message;

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
    agent_system_prompt, TOOL_USE_GUIDELINES, SAFETY_GUIDELINES,
    CODE_CONTEXT_PREFIX, DOC_CONTEXT_PREFIX, REVIEW_CONTEXT_PREFIX,
};
pub use docs::{analyze_project, architecture_prompt, module_doc_prompt, ProjectStructure, ModuleInfo};
pub use model_db::{
    ModelSpec, ModelCategory, RecommendedSetup, MODEL_CATALOG,
    models_for_budget, recommended_setup, onboarding_message,
};
pub use permissions::{PermissionManager, PermissionDecision, PermissionRequest};
pub use diff::{
    FileDiff, DiffLine, LineTag, ProposedAction, CommandResult,
    classify_command_risk, apply_edit, execute_command,
};
pub use message::{ConversationMessage, ConversationRole};
