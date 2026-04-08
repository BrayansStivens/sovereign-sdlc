//! Memory Directory — Embedded Vector Store for RAG
//!
//! Pure-Rust vector database using cosine similarity for semantic search.
//! Persists to disk via bincode. Hardware-adaptive indexing concurrency.

use crate::hardware_env::PerformanceTier;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

// ────────────────────────────────────────────────────────
// Constants
// ────────────────────────────────────────────────────────

/// nomic-embed-text output dimension
pub const EMBEDDING_DIM: usize = 768;

/// Default chunk size in characters (~512 tokens)
const CHUNK_SIZE: usize = 2048;

/// Chunk overlap in characters
const CHUNK_OVERLAP: usize = 256;

/// Maximum file size to index (skip huge binaries/assets)
const MAX_FILE_SIZE: u64 = 512 * 1024; // 512 KB

/// Embedding model name for Ollama
pub const EMBEDDING_MODEL: &str = "nomic-embed-text";

// ────────────────────────────────────────────────────────
// Security: Files to NEVER index (Zero-Trust)
// ────────────────────────────────────────────────────────

const SENSITIVE_PATTERNS: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    "credentials",
    "secrets",
    ".key",
    ".pem",
    ".p12",
    ".pfx",
    ".cert",
    "id_rsa",
    "id_ed25519",
    ".password",
    "token.json",
    "service_account",
    "auth.json",
];

const SENSITIVE_EXTENSIONS: &[&str] = &[
    "key", "pem", "p12", "pfx", "cert", "crt", "jks", "keystore",
];

const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "__pycache__", ".venv",
    "venv", "dist", "build", ".next", ".nuxt", ".cache",
    ".sovereign", "vendor",
];

const SKIP_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "svg", "ico", "bmp", "webp",
    "mp3", "mp4", "avi", "mov", "wav", "flac",
    "zip", "tar", "gz", "bz2", "xz", "rar", "7z",
    "wasm", "so", "dll", "dylib", "exe", "bin",
    "pdf", "doc", "docx", "xls", "xlsx",
    "lock", "sum",
    "sqlite", "db", "sqlite3",
];

// ────────────────────────────────────────────────────────
// Data Structures
// ────────────────────────────────────────────────────────

/// A single indexed document chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocChunk {
    pub id: u64,
    pub file_path: PathBuf,
    pub content: String,
    pub start_offset: usize,
    pub end_offset: usize,
}

/// Stored embedding with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    chunk: DocChunk,
    embedding: Vec<f32>,
}

/// Search result with similarity score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk: DocChunk,
    pub score: f32,
}

/// Index statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_chunks: usize,
    pub total_files: usize,
    pub index_size_bytes: u64,
    pub indexed_paths: Vec<PathBuf>,
}

/// Indexing progress callback
pub struct IndexProgress {
    pub files_scanned: usize,
    pub files_total: usize,
    pub chunks_indexed: usize,
    pub current_file: PathBuf,
}

// ────────────────────────────────────────────────────────
// Vector Store
// ────────────────────────────────────────────────────────

/// Embedded vector database for RAG
#[derive(Serialize, Deserialize)]
pub struct VectorStore {
    entries: Vec<StoredEntry>,
    next_id: u64,
    #[serde(default)]
    stats: IndexStats,
}

impl VectorStore {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 0,
            stats: IndexStats {
                total_chunks: 0,
                total_files: 0,
                index_size_bytes: 0,
                indexed_paths: Vec::new(),
            },
        }
    }

    /// Load from disk or create new
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            let data = std::fs::read(path)
                .context("Failed to read vector index")?;
            let store: Self = bincode::deserialize(&data)
                .context("Failed to deserialize vector index (may be corrupted)")?;
            tracing::info!(chunks = store.entries.len(), "Vector index loaded");
            Ok(store)
        } else {
            Ok(Self::new())
        }
    }

    /// Persist to disk
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = bincode::serialize(self)
            .context("Failed to serialize vector index")?;
        std::fs::write(path, &data)?;
        tracing::info!(
            chunks = self.entries.len(),
            bytes = data.len(),
            "Vector index saved"
        );
        Ok(())
    }

    /// Insert a chunk with its embedding
    pub fn insert(&mut self, file_path: PathBuf, content: String, start: usize, end: usize, embedding: Vec<f32>) {
        let chunk = DocChunk {
            id: self.next_id,
            file_path,
            content,
            start_offset: start,
            end_offset: end,
        };
        self.entries.push(StoredEntry { chunk, embedding });
        self.next_id += 1;
    }

    /// Semantic search — cosine similarity, returns top-k results
    pub fn search(&self, query_embedding: &[f32], top_k: usize) -> Vec<SearchResult> {
        if self.entries.is_empty() {
            return Vec::new();
        }

        let mut scores: Vec<(usize, f32)> = self.entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let score = cosine_similarity(query_embedding, &entry.embedding);
                (i, score)
            })
            .collect();

        // Sort by score descending
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        scores.into_iter()
            .take(top_k)
            .filter(|(_, score)| *score > 0.0)
            .map(|(i, score)| SearchResult {
                chunk: self.entries[i].chunk.clone(),
                score,
            })
            .collect()
    }

    /// Clear the entire index
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_id = 0;
        self.stats = IndexStats {
            total_chunks: 0,
            total_files: 0,
            index_size_bytes: 0,
            indexed_paths: Vec::new(),
        };
    }

    pub fn stats(&self) -> &IndexStats {
        &self.stats
    }

    pub fn chunk_count(&self) -> usize {
        self.entries.len()
    }

    /// Update stats after indexing
    pub fn finalize_stats(&mut self, paths: Vec<PathBuf>, files: usize) {
        self.stats.total_chunks = self.entries.len();
        self.stats.total_files = files;
        self.stats.indexed_paths = paths;
    }
}

// ────────────────────────────────────────────────────────
// Project Scanner (Zero-Trust filtered)
// ────────────────────────────────────────────────────────

/// Scan a project directory and return indexable file chunks.
/// Respects .gitignore, skips sensitive files, and handles large projects.
pub fn scan_project(root: &Path) -> Result<Vec<(PathBuf, Vec<(String, usize, usize)>)>> {
    let gitignore_patterns = load_gitignore(root);

    let mut results = Vec::new();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_skipped_dir(e.file_name().to_str().unwrap_or("")))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let rel_path = path.strip_prefix(root).unwrap_or(path);

        // Security filters
        if is_sensitive_file(path) {
            tracing::debug!(path = %path.display(), "Skipping sensitive file");
            continue;
        }

        if is_skipped_extension(path) {
            continue;
        }

        if is_gitignored(rel_path, &gitignore_patterns) {
            continue;
        }

        // Size check
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.len() > MAX_FILE_SIZE {
                continue;
            }
        }

        // Read and chunk
        match std::fs::read_to_string(path) {
            Ok(content) => {
                if content.is_empty() {
                    continue;
                }
                let chunks = chunk_text(&content, CHUNK_SIZE, CHUNK_OVERLAP);
                if !chunks.is_empty() {
                    results.push((path.to_path_buf(), chunks));
                }
            }
            Err(_) => continue, // Binary file or encoding issue
        }
    }

    Ok(results)
}

/// Determine concurrent embedding batch size based on hardware tier
pub fn batch_size_for_tier(tier: PerformanceTier) -> usize {
    match tier {
        PerformanceTier::HighEnd => 16,
        PerformanceTier::Medium => 8,
        PerformanceTier::Small => 4,
        PerformanceTier::ExtraSmall => 1, // Single-threaded to avoid CPU starvation
    }
}

/// TUI refresh rate (ms) based on tier — lower tier = slower refresh to save CPU
pub fn tui_refresh_ms(tier: PerformanceTier) -> u64 {
    match tier {
        PerformanceTier::HighEnd => 100,
        PerformanceTier::Medium => 200,
        PerformanceTier::Small => 500,
        PerformanceTier::ExtraSmall => 1000,
    }
}

// ────────────────────────────────────────────────────────
// Text Chunking
// ────────────────────────────────────────────────────────

/// Split text into overlapping chunks
fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<(String, usize, usize)> {
    let mut chunks = Vec::new();
    let len = text.len();

    if len == 0 {
        return chunks;
    }

    if len <= chunk_size {
        chunks.push((text.to_string(), 0, len));
        return chunks;
    }

    let step = chunk_size.saturating_sub(overlap).max(1);
    let mut start = 0;

    while start < len {
        let mut end = (start + chunk_size).min(len);

        // Try to break at a newline for cleaner chunks
        if end < len {
            if let Some(pos) = text[start..end].rfind('\n') {
                if pos > step / 2 {
                    end = start + pos + 1;
                }
            }
        }

        // Ensure valid UTF-8 boundary
        while end < len && !text.is_char_boundary(end) {
            end += 1;
        }

        chunks.push((text[start..end].to_string(), start, end));

        if end >= len {
            break;
        }

        start += step;
        // Ensure valid UTF-8 boundary
        while start < len && !text.is_char_boundary(start) {
            start += 1;
        }
    }

    chunks
}

// ────────────────────────────────────────────────────────
// Security Filtering
// ────────────────────────────────────────────────────────

fn is_sensitive_file(path: &Path) -> bool {
    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    for pattern in SENSITIVE_PATTERNS {
        if name.contains(pattern) {
            return true;
        }
    }

    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        if SENSITIVE_EXTENSIONS.contains(&ext.to_lowercase().as_str()) {
            return true;
        }
    }

    false
}

fn is_skipped_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name)
}

fn is_skipped_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| SKIP_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn load_gitignore(root: &Path) -> HashSet<String> {
    let gitignore = root.join(".gitignore");
    match std::fs::read_to_string(gitignore) {
        Ok(content) => content
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| l.trim().trim_end_matches('/').to_string())
            .collect(),
        Err(_) => HashSet::new(),
    }
}

fn is_gitignored(rel_path: &Path, patterns: &HashSet<String>) -> bool {
    let path_str = rel_path.to_string_lossy();
    for pattern in patterns {
        if path_str.contains(pattern.as_str()) {
            return true;
        }
    }
    false
}

// ────────────────────────────────────────────────────────
// Vector Math (pure Rust, no deps)
// ────────────────────────────────────────────────────────

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

// ────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 0.001);
    }

    #[test]
    fn test_chunk_text_small() {
        let text = "hello world";
        let chunks = chunk_text(text, 2048, 256);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, "hello world");
    }

    #[test]
    fn test_chunk_text_large() {
        let text = "a\n".repeat(2000);
        let chunks = chunk_text(&text, 100, 20);
        assert!(chunks.len() > 1);
        // Check overlap: last chars of chunk N should appear at start of chunk N+1
    }

    #[test]
    fn test_sensitive_file_detection() {
        assert!(is_sensitive_file(Path::new(".env")));
        assert!(is_sensitive_file(Path::new(".env.local")));
        assert!(is_sensitive_file(Path::new("secrets.yaml")));
        assert!(is_sensitive_file(Path::new("server.key")));
        assert!(is_sensitive_file(Path::new("cert.pem")));
        assert!(!is_sensitive_file(Path::new("main.rs")));
        assert!(!is_sensitive_file(Path::new("README.md")));
    }

    #[test]
    fn test_skipped_dirs() {
        assert!(is_skipped_dir(".git"));
        assert!(is_skipped_dir("node_modules"));
        assert!(is_skipped_dir("target"));
        assert!(!is_skipped_dir("src"));
    }

    #[test]
    fn test_skipped_extensions() {
        assert!(is_skipped_extension(Path::new("photo.png")));
        assert!(is_skipped_extension(Path::new("archive.zip")));
        assert!(is_skipped_extension(Path::new("lib.so")));
        assert!(!is_skipped_extension(Path::new("main.rs")));
        assert!(!is_skipped_extension(Path::new("app.py")));
    }

    #[test]
    fn test_vector_store_insert_and_search() {
        let mut store = VectorStore::new();

        store.insert(
            PathBuf::from("a.rs"),
            "fn main() { println!(\"hello\"); }".into(),
            0, 33,
            vec![1.0, 0.0, 0.0],
        );
        store.insert(
            PathBuf::from("b.rs"),
            "fn add(a: i32, b: i32) -> i32 { a + b }".into(),
            0, 40,
            vec![0.9, 0.1, 0.0],
        );
        store.insert(
            PathBuf::from("c.md"),
            "This is documentation about the API".into(),
            0, 36,
            vec![0.0, 0.0, 1.0],
        );

        let query = vec![1.0, 0.0, 0.0]; // Should match a.rs best
        let results = store.search(&query, 2);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].chunk.file_path, PathBuf::from("a.rs"));
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_vector_store_persistence() {
        let tmp = std::env::temp_dir().join("sovereign-test-index.bin");

        let mut store = VectorStore::new();
        store.insert(
            PathBuf::from("test.rs"),
            "test content".into(),
            0, 12,
            vec![0.5; 10],
        );
        store.save(&tmp).unwrap();

        let loaded = VectorStore::load_or_create(&tmp).unwrap();
        assert_eq!(loaded.chunk_count(), 1);
        assert_eq!(loaded.entries[0].chunk.content, "test content");

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_batch_size_for_tiers() {
        assert_eq!(batch_size_for_tier(PerformanceTier::HighEnd), 16);
        assert_eq!(batch_size_for_tier(PerformanceTier::ExtraSmall), 1);
    }

    #[test]
    fn test_tui_refresh_rates() {
        assert_eq!(tui_refresh_ms(PerformanceTier::HighEnd), 100);
        assert_eq!(tui_refresh_ms(PerformanceTier::ExtraSmall), 1000);
    }
}
