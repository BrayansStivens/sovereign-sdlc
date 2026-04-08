pub mod security;
pub mod report;
pub mod tool_trait;
pub mod tools_impl;

pub use security::{
    Finding, ScanReport, SecurityScanner, SecurityTool, Severity,
    Semgrep, CargoAudit, ClippyLint,
    SECURITY_SYSTEM_PROMPT,
};

pub use report::generate_report;

pub use tool_trait::{
    Tool, ToolResult, ToolContext, ToolCall, ToolRegistry,
    PermissionLevel, parse_tool_call,
};

pub use tools_impl::default_registry;
