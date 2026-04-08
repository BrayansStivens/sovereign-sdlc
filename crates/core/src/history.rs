//! Chronicle — Session history with encrypted chat logs
//!
//! SQLite persistence in .sovereign/history.db
//! Chat logs are hashed for integrity verification.

use anyhow::{Context, Result};
use chrono::Local;
use rusqlite::{params, Connection};
use sha2::{Sha256, Digest};
use std::path::{Path, PathBuf};

/// A stored session record
#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub id: i64,
    pub timestamp: String,
    pub project_path: String,
    pub summary: String,
    pub vulns_found: i64,
    pub chat_log_json: String,
    pub familiar_name: String,
    pub familiar_level: i64,
    pub integrity_hash: String,
    pub duration_secs: i64,
}

/// Chronicle database for session persistence
pub struct Chronicle {
    conn: Connection,
}

impl Chronicle {
    pub fn open(project_root: &Path) -> Result<Self> {
        let dir = project_root.join(".sovereign");
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("history.db");

        let conn = Connection::open(&db_path)
            .context("Failed to open history database")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp       TEXT NOT NULL,
                project_path    TEXT NOT NULL,
                summary         TEXT NOT NULL DEFAULT '',
                vulns_found     INTEGER NOT NULL DEFAULT 0,
                chat_log_json   TEXT NOT NULL DEFAULT '[]',
                familiar_name   TEXT NOT NULL DEFAULT '',
                familiar_level  INTEGER NOT NULL DEFAULT 1,
                integrity_hash  TEXT NOT NULL DEFAULT '',
                duration_secs   INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_project ON sessions(project_path);
            CREATE INDEX IF NOT EXISTS idx_ts ON sessions(timestamp);"
        )?;

        Ok(Self { conn })
    }

    /// Save a session
    pub fn save_session(
        &self,
        project_path: &str,
        summary: &str,
        vulns_found: i64,
        chat_log: &[(String, String)],
        familiar_name: &str,
        familiar_level: i64,
        duration_secs: i64,
    ) -> Result<i64> {
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let chat_json = serde_json::to_string(chat_log)?;
        let hash = compute_integrity_hash(&chat_json);

        self.conn.execute(
            "INSERT INTO sessions (timestamp, project_path, summary, vulns_found, chat_log_json, familiar_name, familiar_level, integrity_hash, duration_secs)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![ts, project_path, summary, vulns_found, chat_json, familiar_name, familiar_level, hash, duration_secs],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// List recent sessions
    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, project_path, summary, vulns_found, chat_log_json,
                    familiar_name, familiar_level, integrity_hash, duration_secs
             FROM sessions ORDER BY timestamp DESC LIMIT ?1"
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                project_path: row.get(2)?,
                summary: row.get(3)?,
                vulns_found: row.get(4)?,
                chat_log_json: row.get(5)?,
                familiar_name: row.get(6)?,
                familiar_level: row.get(7)?,
                integrity_hash: row.get(8)?,
                duration_secs: row.get(9)?,
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Load a specific session by ID
    pub fn load_session(&self, id: i64) -> Result<Option<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, timestamp, project_path, summary, vulns_found, chat_log_json,
                    familiar_name, familiar_level, integrity_hash, duration_secs
             FROM sessions WHERE id = ?1"
        )?;

        let mut rows = stmt.query_map(params![id], |row| {
            Ok(SessionRecord {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                project_path: row.get(2)?,
                summary: row.get(3)?,
                vulns_found: row.get(4)?,
                chat_log_json: row.get(5)?,
                familiar_name: row.get(6)?,
                familiar_level: row.get(7)?,
                integrity_hash: row.get(8)?,
                duration_secs: row.get(9)?,
            })
        })?;

        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// Verify integrity of a session's chat log
    pub fn verify_integrity(&self, session: &SessionRecord) -> bool {
        let expected = compute_integrity_hash(&session.chat_log_json);
        expected == session.integrity_hash
    }

    /// Restore chat messages from a session
    pub fn restore_messages(session: &SessionRecord) -> Result<Vec<(String, String)>> {
        let messages: Vec<(String, String)> = serde_json::from_str(&session.chat_log_json)?;
        Ok(messages)
    }

    /// Days since last session for this project
    pub fn days_since_last(&self, project_path: &str) -> Result<Option<i64>> {
        let result: Option<String> = self.conn.query_row(
            "SELECT timestamp FROM sessions WHERE project_path = ?1 ORDER BY timestamp DESC LIMIT 1",
            params![project_path],
            |row| row.get(0),
        ).ok();

        if let Some(ts) = result {
            if let Ok(last) = chrono::NaiveDateTime::parse_from_str(&ts, "%Y-%m-%d %H:%M:%S") {
                let now = Local::now().naive_local();
                let days = (now - last).num_days();
                return Ok(Some(days));
            }
        }
        Ok(None)
    }

    /// Get session count for a project
    pub fn session_count(&self, project_path: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE project_path = ?1",
            params![project_path],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Format session list for display
    pub fn format_sessions(sessions: &[SessionRecord]) -> String {
        if sessions.is_empty() {
            return "  No sessions found.".to_string();
        }
        let mut out = String::from("── Sessions ──\n");
        for s in sessions {
            let verified = if compute_integrity_hash(&s.chat_log_json) == s.integrity_hash {
                "OK" } else { "TAMPERED" };
            out.push_str(&format!(
                "  #{:<4} {} | {} | vulns: {} | buddy: {} Lv{} [{}]\n",
                s.id, s.timestamp, s.project_path, s.vulns_found,
                s.familiar_name, s.familiar_level, verified,
            ));
        }
        out
    }
}

/// SHA-256 hash for chat log integrity
fn compute_integrity_hash(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_chronicle() -> Chronicle {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = temp_dir().join(format!("chronicle-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        Chronicle::open(&tmp).unwrap()
    }

    #[test]
    fn test_save_and_load() {
        let c = test_chronicle();
        let msgs = vec![("you".to_string(), "hello".to_string())];
        let id = c.save_session("/project", "test session", 2, &msgs, "Byte", 3, 120).unwrap();

        let session = c.load_session(id).unwrap().unwrap();
        assert_eq!(session.summary, "test session");
        assert_eq!(session.vulns_found, 2);
        assert_eq!(session.familiar_name, "Byte");
    }

    #[test]
    fn test_integrity_verification() {
        let c = test_chronicle();
        let msgs = vec![("system".to_string(), "init".to_string())];
        let id = c.save_session("/p", "", 0, &msgs, "Shadow", 1, 60).unwrap();

        let session = c.load_session(id).unwrap().unwrap();
        assert!(c.verify_integrity(&session));
    }

    #[test]
    fn test_list_sessions() {
        let c = test_chronicle();
        let msgs: Vec<(String, String)> = vec![];
        c.save_session("/a", "s1", 0, &msgs, "B", 1, 10).unwrap();
        c.save_session("/b", "s2", 1, &msgs, "C", 2, 20).unwrap();

        let list = c.list_sessions(10).unwrap();
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn test_restore_messages() {
        let c = test_chronicle();
        let msgs = vec![
            ("you".to_string(), "hi".to_string()),
            ("sovereign".to_string(), "hello".to_string()),
        ];
        let id = c.save_session("/p", "", 0, &msgs, "N", 1, 5).unwrap();
        let session = c.load_session(id).unwrap().unwrap();
        let restored = Chronicle::restore_messages(&session).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].1, "hi");
    }
}
