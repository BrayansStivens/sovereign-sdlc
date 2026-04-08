//! Permission System — Multi-tier approval for tool execution
//!
//! Inspired by claurst: once / session / always / deny

use std::collections::HashMap;

/// Permission decision
#[derive(Debug, Clone, PartialEq)]
pub enum PermissionDecision {
    /// Allow this one time
    AllowOnce,
    /// Allow for the rest of this session
    AllowSession,
    /// Allow permanently (saved to config)
    AllowAlways,
    /// Deny this action
    Deny,
    /// Need to ask the user
    Ask,
}

/// A permission rule stored in the manager
#[derive(Debug, Clone)]
struct PermissionRule {
    tool_name: String,
    decision: PermissionDecision,
}

/// Manages tool permissions across the session
pub struct PermissionManager {
    /// Session-level rules (cleared on restart)
    session_rules: HashMap<String, PermissionDecision>,
    /// Persistent rules (survive restart) — TODO: save to .sovereign/permissions.json
    persistent_rules: HashMap<String, PermissionDecision>,
}

impl PermissionManager {
    pub fn new() -> Self {
        Self {
            session_rules: HashMap::new(),
            persistent_rules: HashMap::new(),
        }
    }

    /// Check if a tool call is allowed
    pub fn check(&self, tool_name: &str, description: &str) -> PermissionDecision {
        // Check persistent rules first
        if let Some(decision) = self.persistent_rules.get(tool_name) {
            return decision.clone();
        }

        // Check session rules
        if let Some(decision) = self.session_rules.get(tool_name) {
            return decision.clone();
        }

        // Read-only tools are auto-allowed
        if is_readonly_tool(tool_name) {
            return PermissionDecision::AllowOnce;
        }

        // Everything else needs approval
        PermissionDecision::Ask
    }

    /// Record a user's decision
    pub fn record_decision(&mut self, tool_name: &str, decision: PermissionDecision) {
        match &decision {
            PermissionDecision::AllowSession => {
                self.session_rules.insert(tool_name.to_string(), PermissionDecision::AllowSession);
            }
            PermissionDecision::AllowAlways => {
                self.persistent_rules.insert(tool_name.to_string(), PermissionDecision::AllowAlways);
            }
            PermissionDecision::Deny => {
                self.session_rules.insert(tool_name.to_string(), PermissionDecision::Deny);
            }
            _ => {} // AllowOnce and Ask don't get stored
        }
    }

    /// Check if we have a session rule allowing this tool
    pub fn is_session_allowed(&self, tool_name: &str) -> bool {
        matches!(
            self.session_rules.get(tool_name),
            Some(PermissionDecision::AllowSession)
        ) || matches!(
            self.persistent_rules.get(tool_name),
            Some(PermissionDecision::AllowAlways)
        )
    }

    /// Clear session rules
    pub fn clear_session(&mut self) {
        self.session_rules.clear();
    }
}

/// Tools that are safe to auto-approve
fn is_readonly_tool(name: &str) -> bool {
    matches!(name, "read" | "glob")
}

/// Permission request sent to the TUI for display
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub description: String,
    pub input_preview: String,
    pub is_dangerous: bool,
}

impl PermissionRequest {
    pub fn options_text(&self) -> &str {
        if self.is_dangerous {
            "(y) Allow once  (n) Deny  (!) Allow session"
        } else {
            "(y) Allow once  (Y) Allow session  (n) Deny"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_readonly_auto_allowed() {
        let pm = PermissionManager::new();
        assert_eq!(pm.check("read", "Read file.rs"), PermissionDecision::AllowOnce);
        assert_eq!(pm.check("glob", "Search *.rs"), PermissionDecision::AllowOnce);
    }

    #[test]
    fn test_execute_needs_ask() {
        let pm = PermissionManager::new();
        assert_eq!(pm.check("bash", "Run pwd"), PermissionDecision::Ask);
        assert_eq!(pm.check("edit", "Edit file"), PermissionDecision::Ask);
    }

    #[test]
    fn test_session_rule() {
        let mut pm = PermissionManager::new();
        pm.record_decision("bash", PermissionDecision::AllowSession);
        assert!(pm.is_session_allowed("bash"));
        assert_eq!(pm.check("bash", "Run pwd"), PermissionDecision::AllowSession);
    }

    #[test]
    fn test_deny_rule() {
        let mut pm = PermissionManager::new();
        pm.record_decision("bash", PermissionDecision::Deny);
        assert_eq!(pm.check("bash", "Run rm"), PermissionDecision::Deny);
    }

    #[test]
    fn test_clear_session() {
        let mut pm = PermissionManager::new();
        pm.record_decision("bash", PermissionDecision::AllowSession);
        pm.clear_session();
        assert_eq!(pm.check("bash", "Run pwd"), PermissionDecision::Ask);
    }
}
