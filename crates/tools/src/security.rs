use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Severity levels for security findings
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Info => write!(f, "INFO"),
            Severity::Warning => write!(f, "WARN"),
            Severity::Error => write!(f, "ERROR"),
            Severity::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// A single security finding from any tool
#[derive(Debug, Clone)]
pub struct Finding {
    pub tool: String,
    pub severity: Severity,
    pub rule_id: String,
    pub message: String,
    pub file: PathBuf,
    pub line: Option<usize>,
    pub owasp_category: Option<String>,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let loc = match self.line {
            Some(l) => format!("{}:{l}", self.file.display()),
            None => self.file.display().to_string(),
        };
        let owasp = self
            .owasp_category
            .as_deref()
            .map(|o| format!(" [{o}]"))
            .unwrap_or_default();
        write!(
            f,
            "[{severity}] {tool}{owasp} {loc}\n  {rule}: {msg}",
            severity = self.severity,
            tool = self.tool,
            rule = self.rule_id,
            msg = self.message,
        )
    }
}

/// Aggregated scan results
#[derive(Debug, Clone)]
pub struct ScanReport {
    pub tool: String,
    pub target: PathBuf,
    pub findings: Vec<Finding>,
}

impl ScanReport {
    pub fn summary(&self) -> String {
        let critical = self.findings.iter().filter(|f| f.severity == Severity::Critical).count();
        let errors = self.findings.iter().filter(|f| f.severity == Severity::Error).count();
        let warnings = self.findings.iter().filter(|f| f.severity == Severity::Warning).count();
        let infos = self.findings.iter().filter(|f| f.severity == Severity::Info).count();

        format!(
            "── {tool} Scan: {path} ──\n  {total} findings: {critical} critical, {errors} error, {warnings} warning, {infos} info",
            tool = self.tool,
            path = self.target.display(),
            total = self.findings.len(),
        )
    }

    /// Format findings for display in TUI
    pub fn display_findings(&self, max: usize) -> String {
        if self.findings.is_empty() {
            return format!("  {} scan clean — no findings.", self.tool);
        }

        let mut output = self.summary();
        let mut sorted = self.findings.clone();
        sorted.sort_by(|a, b| b.severity.cmp(&a.severity));

        for (i, finding) in sorted.iter().take(max).enumerate() {
            output.push_str(&format!("\n\n  #{}: {finding}", i + 1));
        }

        if self.findings.len() > max {
            output.push_str(&format!(
                "\n\n  ... and {} more findings",
                self.findings.len() - max
            ));
        }

        output
    }

    /// Generate a prompt for the LLM to suggest fixes
    pub fn auto_fix_prompt(&self) -> Option<String> {
        let actionable: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|f| f.severity >= Severity::Warning)
            .collect();

        if actionable.is_empty() {
            return None;
        }

        let findings_text: String = actionable
            .iter()
            .take(5)
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n");

        Some(format!(
            "The following security vulnerabilities were found by {tool}. \
             For each finding, propose a specific code fix following OWASP ASVS standards. \
             Show the exact file, line, and corrected code.\n\n{findings_text}",
            tool = self.tool,
        ))
    }
}

/// Trait for all security scanning tools
pub trait SecurityTool {
    /// Human-readable name of the tool
    fn name(&self) -> &str;

    /// Check if the tool is installed and available
    fn is_available(&self) -> bool;

    /// Run a scan on the given target path
    fn scan(&self, target: &Path) -> Result<ScanReport>;
}

// ──────────────────────────────────────────────
// Semgrep SAST Integration
// ──────────────────────────────────────────────

/// Semgrep JSON output structures (subset)
#[derive(Deserialize)]
struct SemgrepOutput {
    results: Vec<SemgrepResult>,
}

#[derive(Deserialize)]
struct SemgrepResult {
    check_id: String,
    path: String,
    start: SemgrepLocation,
    extra: SemgrepExtra,
}

#[derive(Deserialize)]
struct SemgrepLocation {
    line: usize,
}

#[derive(Deserialize)]
struct SemgrepExtra {
    message: String,
    severity: String,
    #[serde(default)]
    metadata: SemgrepMetadata,
}

#[derive(Deserialize, Default)]
struct SemgrepMetadata {
    #[serde(default)]
    owasp: Vec<String>,
}

pub struct Semgrep {
    /// Semgrep config ruleset (default: "p/default")
    config: String,
}

impl Semgrep {
    pub fn new() -> Self {
        Self {
            config: "p/default".to_string(),
        }
    }

    pub fn with_config(config: &str) -> Self {
        Self {
            config: config.to_string(),
        }
    }

    fn parse_severity(s: &str) -> Severity {
        match s.to_uppercase().as_str() {
            "ERROR" => Severity::Error,
            "WARNING" => Severity::Warning,
            "INFO" => Severity::Info,
            _ => Severity::Warning,
        }
    }
}

impl SecurityTool for Semgrep {
    fn name(&self) -> &str {
        "Semgrep"
    }

    fn is_available(&self) -> bool {
        Command::new("semgrep")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn scan(&self, target: &Path) -> Result<ScanReport> {
        let output = Command::new("semgrep")
            .args(["--config", &self.config, "--json", "--quiet"])
            .arg(target)
            .output()
            .context("Failed to execute semgrep. Is it installed? (pip install semgrep)")?;

        let findings = if output.status.success() || !output.stdout.is_empty() {
            let parsed: SemgrepOutput =
                serde_json::from_slice(&output.stdout).context("Failed to parse semgrep JSON")?;

            parsed
                .results
                .into_iter()
                .map(|r| {
                    let owasp = r.extra.metadata.owasp.first().cloned();
                    Finding {
                        tool: "Semgrep".to_string(),
                        severity: Self::parse_severity(&r.extra.severity),
                        rule_id: r.check_id,
                        message: r.extra.message,
                        file: PathBuf::from(r.path),
                        line: Some(r.start.line),
                        owasp_category: owasp,
                    }
                })
                .collect()
        } else {
            vec![]
        };

        Ok(ScanReport {
            tool: "Semgrep".to_string(),
            target: target.to_path_buf(),
            findings,
        })
    }
}

// ──────────────────────────────────────────────
// Cargo Audit SCA Integration
// ──────────────────────────────────────────────

#[derive(Deserialize)]
struct CargoAuditOutput {
    vulnerabilities: CargoAuditVulns,
}

#[derive(Deserialize)]
struct CargoAuditVulns {
    list: Vec<CargoAuditEntry>,
}

#[derive(Deserialize)]
struct CargoAuditEntry {
    advisory: CargoAdvisory,
}

#[derive(Deserialize)]
struct CargoAdvisory {
    id: String,
    title: String,
    #[serde(default)]
    description: String,
}

pub struct CargoAudit;

impl SecurityTool for CargoAudit {
    fn name(&self) -> &str {
        "cargo-audit"
    }

    fn is_available(&self) -> bool {
        Command::new("cargo")
            .args(["audit", "--version"])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn scan(&self, target: &Path) -> Result<ScanReport> {
        let cargo_toml = target.join("Cargo.toml");
        if !cargo_toml.exists() {
            return Ok(ScanReport {
                tool: "cargo-audit".to_string(),
                target: target.to_path_buf(),
                findings: vec![],
            });
        }

        let output = Command::new("cargo")
            .args(["audit", "--json"])
            .current_dir(target)
            .output()
            .context("Failed to run cargo audit. Install with: cargo install cargo-audit")?;

        let findings = if !output.stdout.is_empty() {
            match serde_json::from_slice::<CargoAuditOutput>(&output.stdout) {
                Ok(parsed) => parsed
                    .vulnerabilities
                    .list
                    .into_iter()
                    .map(|entry| Finding {
                        tool: "cargo-audit".to_string(),
                        severity: Severity::Critical,
                        rule_id: entry.advisory.id,
                        message: format!(
                            "{}: {}",
                            entry.advisory.title, entry.advisory.description
                        ),
                        file: cargo_toml.clone(),
                        line: None,
                        owasp_category: Some("A06:2021 Vulnerable Components".to_string()),
                    })
                    .collect(),
                Err(_) => vec![],
            }
        } else {
            vec![]
        };

        Ok(ScanReport {
            tool: "cargo-audit".to_string(),
            target: target.to_path_buf(),
            findings,
        })
    }
}

// ──────────────────────────────────────────────
// Clippy SAST Integration (Rust-specific)
// ──────────────────────────────────────────────

pub struct ClippyLint;

impl SecurityTool for ClippyLint {
    fn name(&self) -> &str {
        "clippy"
    }

    fn is_available(&self) -> bool {
        Command::new("cargo")
            .args(["clippy", "--version"])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn scan(&self, target: &Path) -> Result<ScanReport> {
        let cargo_toml = target.join("Cargo.toml");
        if !cargo_toml.exists() {
            return Ok(ScanReport {
                tool: "clippy".to_string(),
                target: target.to_path_buf(),
                findings: vec![],
            });
        }

        let output = Command::new("cargo")
            .args([
                "clippy",
                "--message-format=json",
                "--quiet",
                "--",
                "-W",
                "clippy::all",
            ])
            .current_dir(target)
            .output()
            .context("Failed to run cargo clippy")?;

        let findings: Vec<Finding> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let v: serde_json::Value = serde_json::from_str(line).ok()?;
                if v["reason"].as_str()? != "compiler-message" {
                    return None;
                }
                let msg = &v["message"];
                let level = msg["level"].as_str()?;
                if level == "note" || level == "help" {
                    return None;
                }
                let text = msg["message"].as_str()?.to_string();
                let code = msg["code"]["code"].as_str().unwrap_or("unknown").to_string();
                let span = msg["spans"].as_array()?.first()?;
                let file = span["file_name"].as_str()?.to_string();
                let line = span["line_start"].as_u64()? as usize;

                Some(Finding {
                    tool: "clippy".to_string(),
                    severity: match level {
                        "error" => Severity::Error,
                        "warning" => Severity::Warning,
                        _ => Severity::Info,
                    },
                    rule_id: code,
                    message: text,
                    file: PathBuf::from(file),
                    line: Some(line),
                    owasp_category: None,
                })
            })
            .collect();

        Ok(ScanReport {
            tool: "clippy".to_string(),
            target: target.to_path_buf(),
            findings,
        })
    }
}

// ──────────────────────────────────────────────
// Security Scanner Orchestrator
// ──────────────────────────────────────────────

pub struct SecurityScanner {
    tools: Vec<Box<dyn SecurityTool>>,
}

impl SecurityScanner {
    pub fn new() -> Self {
        let tools: Vec<Box<dyn SecurityTool>> = vec![
            Box::new(Semgrep::new()),
            Box::new(CargoAudit),
            Box::new(ClippyLint),
        ];
        Self { tools }
    }

    /// List which tools are available on this system
    pub fn available_tools(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|t| t.is_available())
            .map(|t| t.name())
            .collect()
    }

    /// Run all available security tools on a target
    pub fn scan_all(&self, target: &Path) -> Vec<ScanReport> {
        self.tools
            .iter()
            .filter(|t| t.is_available())
            .filter_map(|t| match t.scan(target) {
                Ok(report) => Some(report),
                Err(e) => {
                    tracing::warn!("{} scan failed: {e}", t.name());
                    None
                }
            })
            .collect()
    }

    /// Run a specific tool by name
    pub fn scan_with(&self, tool_name: &str, target: &Path) -> Result<ScanReport> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name().eq_ignore_ascii_case(tool_name))
            .context(format!("Unknown tool: {tool_name}"))?;

        if !tool.is_available() {
            anyhow::bail!("{} is not installed on this system", tool.name());
        }

        tool.scan(target)
    }

    /// Total findings across all reports
    pub fn total_findings(reports: &[ScanReport]) -> usize {
        reports.iter().map(|r| r.findings.len()).sum()
    }

    /// Count findings by severity across all reports
    pub fn severity_counts(reports: &[ScanReport]) -> (usize, usize, usize, usize) {
        let mut critical = 0;
        let mut error = 0;
        let mut warning = 0;
        let mut info = 0;
        for r in reports {
            for f in &r.findings {
                match f.severity {
                    Severity::Critical => critical += 1,
                    Severity::Error => error += 1,
                    Severity::Warning => warning += 1,
                    Severity::Info => info += 1,
                }
            }
        }
        (critical, error, warning, info)
    }
}

/// OWASP ASVS security prefix injected into all prompts
pub const SECURITY_SYSTEM_PROMPT: &str = "\
You are a secure code generation assistant. Follow these rules strictly:
- Generate code following OWASP ASVS (Application Security Verification Standard).
- Never generate code with SQL injection, XSS, command injection, or path traversal vulnerabilities.
- Always use parameterized queries for database operations.
- Sanitize and validate all user inputs at system boundaries.
- Use safe memory patterns — no unchecked indexing, no raw pointer arithmetic without justification.
- Prefer standard library cryptographic primitives over custom implementations.
- Log security-relevant events but never log secrets, tokens, or PII.
If the user asks for something that would be insecure, warn them and provide the secure alternative.\n\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::Error);
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn test_finding_display() {
        let f = Finding {
            tool: "Semgrep".into(),
            severity: Severity::Error,
            rule_id: "python.flask.security.injection".into(),
            message: "Possible SQL injection".into(),
            file: PathBuf::from("app/routes.py"),
            line: Some(42),
            owasp_category: Some("A03:2021".into()),
        };
        let display = format!("{f}");
        assert!(display.contains("[ERROR]"));
        assert!(display.contains("A03:2021"));
        assert!(display.contains("routes.py:42"));
    }

    #[test]
    fn test_scan_report_summary() {
        let report = ScanReport {
            tool: "Semgrep".into(),
            target: PathBuf::from("/project"),
            findings: vec![
                Finding {
                    tool: "Semgrep".into(),
                    severity: Severity::Critical,
                    rule_id: "rule1".into(),
                    message: "bad".into(),
                    file: PathBuf::from("a.py"),
                    line: Some(1),
                    owasp_category: None,
                },
                Finding {
                    tool: "Semgrep".into(),
                    severity: Severity::Warning,
                    rule_id: "rule2".into(),
                    message: "meh".into(),
                    file: PathBuf::from("b.py"),
                    line: Some(2),
                    owasp_category: None,
                },
            ],
        };
        let summary = report.summary();
        assert!(summary.contains("2 findings"));
        assert!(summary.contains("1 critical"));
    }

    #[test]
    fn test_auto_fix_prompt_generation() {
        let report = ScanReport {
            tool: "Semgrep".into(),
            target: PathBuf::from("/project"),
            findings: vec![Finding {
                tool: "Semgrep".into(),
                severity: Severity::Error,
                rule_id: "injection".into(),
                message: "SQL injection found".into(),
                file: PathBuf::from("db.py"),
                line: Some(10),
                owasp_category: Some("A03:2021".into()),
            }],
        };
        let prompt = report.auto_fix_prompt().unwrap();
        assert!(prompt.contains("OWASP ASVS"));
        assert!(prompt.contains("SQL injection"));
    }

    #[test]
    fn test_empty_report_no_fix_prompt() {
        let report = ScanReport {
            tool: "test".into(),
            target: PathBuf::from("/"),
            findings: vec![],
        };
        assert!(report.auto_fix_prompt().is_none());
    }

    #[test]
    fn test_security_scanner_creation() {
        let scanner = SecurityScanner::new();
        // Should have 3 tools registered
        assert_eq!(scanner.tools.len(), 3);
    }
}
