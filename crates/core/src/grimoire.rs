//! The Grimoire — Security Patterns knowledge base
//!
//! Persists error→fix pairs to SQLite so the agent learns from past fixes.
//! Lightweight on CPU-only tiers, semantic search enabled on HighEnd.

use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};

/// A single security pattern: error context + successful fix
#[derive(Debug, Clone)]
pub struct SecurityPattern {
    pub id: i64,
    pub timestamp: String,
    pub error_context: String,
    pub fix_applied: String,
    pub rule_id: String,
    pub severity: String,
    pub file_path: String,
    pub language: String,
}

/// The Grimoire database
pub struct Grimoire {
    conn: Connection,
    db_path: PathBuf,
}

impl Grimoire {
    /// Open or create the grimoire database
    pub fn open(project_root: &Path) -> Result<Self> {
        let dir = project_root.join(".sovereign");
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("grimoire.db");

        let conn = Connection::open(&db_path)
            .context("Failed to open grimoire database")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS patterns (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   TEXT NOT NULL,
                error_ctx   TEXT NOT NULL,
                fix_applied TEXT NOT NULL,
                rule_id     TEXT NOT NULL DEFAULT '',
                severity    TEXT NOT NULL DEFAULT 'unknown',
                file_path   TEXT NOT NULL DEFAULT '',
                language    TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_rule ON patterns(rule_id);
            CREATE INDEX IF NOT EXISTS idx_lang ON patterns(language);"
        )?;

        Ok(Self { conn, db_path })
    }

    /// Record a successful fix
    pub fn record_fix(
        &self,
        error_context: &str,
        fix_applied: &str,
        rule_id: &str,
        severity: &str,
        file_path: &str,
        language: &str,
    ) -> Result<i64> {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        self.conn.execute(
            "INSERT INTO patterns (timestamp, error_ctx, fix_applied, rule_id, severity, file_path, language)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![ts, error_context, fix_applied, rule_id, severity, file_path, language],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Search patterns by rule_id (exact match — fast, works on all tiers)
    pub fn find_by_rule(&self, rule_id: &str) -> Result<Vec<SecurityPattern>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, error_ctx, fix_applied, rule_id, severity, file_path, language
             FROM patterns WHERE rule_id = ?1 ORDER BY timestamp DESC LIMIT 10"
        )?;
        self.query_patterns(&mut stmt, params![rule_id])
    }

    /// Search patterns by keyword in error context (for lightweight tiers)
    pub fn search_keyword(&self, keyword: &str) -> Result<Vec<SecurityPattern>> {
        let pattern = format!("%{keyword}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, error_ctx, fix_applied, rule_id, severity, file_path, language
             FROM patterns WHERE error_ctx LIKE ?1 OR fix_applied LIKE ?1
             ORDER BY timestamp DESC LIMIT 10"
        )?;
        self.query_patterns(&mut stmt, params![pattern])
    }

    /// Get all patterns for a language
    pub fn patterns_for_language(&self, lang: &str) -> Result<Vec<SecurityPattern>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, error_ctx, fix_applied, rule_id, severity, file_path, language
             FROM patterns WHERE language = ?1 ORDER BY timestamp DESC LIMIT 20"
        )?;
        self.query_patterns(&mut stmt, params![lang])
    }

    /// Get recent patterns (for context injection)
    pub fn recent(&self, limit: usize) -> Result<Vec<SecurityPattern>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, error_ctx, fix_applied, rule_id, severity, file_path, language
             FROM patterns ORDER BY timestamp DESC LIMIT ?1"
        )?;
        self.query_patterns(&mut stmt, params![limit as i64])
    }

    /// Total patterns count
    pub fn count(&self) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM patterns", [], |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Format patterns for LLM context injection
    pub fn format_for_context(&self, patterns: &[SecurityPattern]) -> String {
        if patterns.is_empty() {
            return String::new();
        }

        let mut out = String::from("[Grimoire — Known Security Patterns]:\n");
        for p in patterns.iter().take(5) {
            out.push_str(&format!(
                "- Rule: {} | Severity: {} | File: {}\n  Error: {}\n  Fix: {}\n\n",
                p.rule_id,
                p.severity,
                p.file_path,
                truncate(&p.error_context, 200),
                truncate(&p.fix_applied, 300),
            ));
        }
        out
    }

    fn query_patterns(
        &self,
        stmt: &mut rusqlite::Statement,
        params: impl rusqlite::Params,
    ) -> Result<Vec<SecurityPattern>> {
        let rows = stmt.query_map(params, |row| {
            Ok(SecurityPattern {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                error_context: row.get(2)?,
                fix_applied: row.get(3)?,
                rule_id: row.get(4)?,
                severity: row.get(5)?,
                file_path: row.get(6)?,
                language: row.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() }
    else { format!("{}...", &s[..max]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_grimoire() -> Grimoire {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = temp_dir().join(format!("grimoire-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        Grimoire::open(&tmp).unwrap()
    }

    #[test]
    fn test_record_and_find() {
        let g = test_grimoire();
        g.record_fix(
            "SQL injection in user input",
            "Use parameterized queries",
            "python.flask.sql-injection",
            "ERROR",
            "app/routes.py",
            "python",
        ).unwrap();

        let results = g.find_by_rule("python.flask.sql-injection").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].fix_applied.contains("parameterized"));
    }

    #[test]
    fn test_keyword_search() {
        let g = test_grimoire();
        g.record_fix("buffer overflow in parser", "Add bounds check", "c.overflow", "CRITICAL", "src/parse.c", "c").unwrap();
        g.record_fix("XSS in template", "Escape output", "js.xss", "ERROR", "views/index.ejs", "javascript").unwrap();

        let results = g.search_keyword("overflow").unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].error_context.contains("overflow"));
    }

    #[test]
    fn test_count() {
        let g = test_grimoire();
        assert_eq!(g.count().unwrap(), 0);
        g.record_fix("err", "fix", "r1", "WARN", "f.rs", "rust").unwrap();
        assert_eq!(g.count().unwrap(), 1);
    }
}
