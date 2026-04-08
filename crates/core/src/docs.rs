//! Documentation Generator — /doc command
//!
//! Uses VectorStore context + file analysis to generate
//! ARCHITECTURE.md or per-module docs with Mermaid diagrams.

use crate::memdir::VectorStore;
use anyhow::Result;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Detected module info for documentation
#[derive(Debug, Clone)]
pub struct ModuleInfo {
    pub path: PathBuf,
    pub name: String,
    pub language: String,
    pub public_items: Vec<String>,
    pub line_count: usize,
}

/// Project structure summary for ARCHITECTURE.md generation
#[derive(Debug, Clone)]
pub struct ProjectStructure {
    pub root: PathBuf,
    pub modules: Vec<ModuleInfo>,
    pub total_files: usize,
    pub total_lines: usize,
    pub languages: Vec<(String, usize)>,
}

/// Scan a project and build structural summary
pub fn analyze_project(root: &Path) -> Result<ProjectStructure> {
    let mut modules = Vec::new();
    let mut lang_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut total_lines = 0;

    let skip_dirs = [".git", "node_modules", "target", "__pycache__", ".venv", "dist", "build", ".sovereign"];

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            e.file_name().to_str()
                .map(|n| !skip_dirs.contains(&n))
                .unwrap_or(true)
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let lang = match ext {
            "rs" => "rust",
            "py" => "python",
            "js" | "jsx" => "javascript",
            "ts" | "tsx" => "typescript",
            "go" => "go",
            "java" => "java",
            "c" | "h" => "c",
            "cpp" | "hpp" => "cpp",
            "toml" => "toml",
            "yaml" | "yml" => "yaml",
            "md" => "markdown",
            _ => continue,
        };

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let lines = content.lines().count();
        total_lines += lines;
        *lang_counts.entry(lang.to_string()).or_default() += lines;

        let public_items = extract_public_items(&content, lang);

        let name = path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        modules.push(ModuleInfo {
            path: path.to_path_buf(),
            name,
            language: lang.to_string(),
            public_items,
            line_count: lines,
        });
    }

    let mut languages: Vec<(String, usize)> = lang_counts.into_iter().collect();
    languages.sort_by(|a, b| b.1.cmp(&a.1));

    Ok(ProjectStructure {
        root: root.to_path_buf(),
        modules,
        total_files: 0, // will be set below
        total_lines,
        languages,
    })
}

/// Extract public function/struct/trait names from source code
fn extract_public_items(content: &str, lang: &str) -> Vec<String> {
    let mut items = Vec::new();

    match lang {
        "rust" => {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("pub struct ")
                    || trimmed.starts_with("pub enum ")
                    || trimmed.starts_with("pub trait ")
                    || trimmed.starts_with("pub type ")
                    || trimmed.starts_with("pub const ")
                    || trimmed.starts_with("pub async fn ")
                {
                    // Extract name: take until ( or { or <
                    let sig = trimmed.trim_start_matches("pub async ")
                        .trim_start_matches("pub ");
                    let end = sig.find(|c: char| c == '(' || c == '{' || c == '<' || c == ':')
                        .unwrap_or(sig.len());
                    items.push(sig[..end].trim().to_string());
                }
            }
        }
        "python" => {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("def ") || trimmed.starts_with("class ") {
                    if !trimmed.contains("_") || !trimmed.starts_with("def _") {
                        let end = trimmed.find('(').unwrap_or(trimmed.len());
                        items.push(trimmed[..end].trim().to_string());
                    }
                }
            }
        }
        "typescript" | "javascript" => {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("export function ")
                    || trimmed.starts_with("export class ")
                    || trimmed.starts_with("export interface ")
                    || trimmed.starts_with("export type ")
                    || trimmed.starts_with("export const ")
                {
                    let sig = trimmed.trim_start_matches("export ");
                    let end = sig.find(|c: char| c == '(' || c == '{' || c == '<' || c == '=' || c == ':')
                        .unwrap_or(sig.len());
                    items.push(sig[..end].trim().to_string());
                }
            }
        }
        _ => {}
    }

    items
}

/// Generate an LLM prompt for ARCHITECTURE.md creation
pub fn architecture_prompt(structure: &ProjectStructure, rag_context: &str) -> String {
    let mut prompt = String::from(
        "Generate an ARCHITECTURE.md for this project. Use Mermaid diagrams for:\n\
         - Component dependency graph\n\
         - Data flow for the main pipeline\n\
         - Any async/event-driven flows\n\n\
         Be concise and technical. Use ```mermaid blocks.\n\n"
    );

    prompt.push_str("## Project Summary\n\n");
    prompt.push_str(&format!("- {} files, {} total lines\n", structure.modules.len(), structure.total_lines));
    prompt.push_str("- Languages: ");
    for (lang, lines) in &structure.languages {
        prompt.push_str(&format!("{lang}({lines}) "));
    }
    prompt.push_str("\n\n## Key Modules\n\n");

    for module in structure.modules.iter().take(30) {
        prompt.push_str(&format!("### {} ({} lines, {})\n", module.name, module.line_count, module.language));
        if !module.public_items.is_empty() {
            for item in module.public_items.iter().take(10) {
                prompt.push_str(&format!("- `{item}`\n"));
            }
        }
        prompt.push('\n');
    }

    if !rag_context.is_empty() {
        prompt.push_str("\n## Additional Context from Index\n\n");
        prompt.push_str(rag_context);
    }

    prompt
}

/// Generate a per-module documentation prompt
pub fn module_doc_prompt(module: &ModuleInfo, source: &str) -> String {
    format!(
        "Generate technical documentation for this module.\n\
         Include: purpose, public API reference, usage examples, and a Mermaid sequence diagram \
         if the module has async flows or complex interactions.\n\n\
         Module: {name} ({lang}, {lines} lines)\n\n\
         Source:\n```{lang}\n{source}\n```",
        name = module.name,
        lang = module.language,
        lines = module.line_count,
        source = if source.len() > 8000 { &source[..8000] } else { source },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_rust_public_items() {
        let src = r#"
pub fn main() {}
pub struct App { }
pub enum Color { Red, Blue }
fn private() {}
pub trait Render { }
pub async fn serve() {}
"#;
        let items = extract_public_items(src, "rust");
        assert!(items.contains(&"fn main".to_string()));
        assert!(items.contains(&"struct App".to_string()));
        assert!(items.contains(&"enum Color".to_string()));
        assert!(items.contains(&"trait Render".to_string()));
        assert!(items.contains(&"fn serve".to_string()));
        assert!(!items.iter().any(|i| i.contains("private")));
    }

    #[test]
    fn test_extract_python_items() {
        let src = "def process():\n    pass\nclass Handler:\n    pass\ndef _private():\n    pass";
        let items = extract_public_items(src, "python");
        assert!(items.iter().any(|i| i.contains("process")));
        assert!(items.iter().any(|i| i.contains("Handler")));
    }

    #[test]
    fn test_extract_typescript_items() {
        let src = "export function fetchData() {}\nexport class ApiClient {}\nfunction internal() {}";
        let items = extract_public_items(src, "typescript");
        assert!(items.iter().any(|i| i.contains("fetchData")));
        assert!(items.iter().any(|i| i.contains("ApiClient")));
        assert!(!items.iter().any(|i| i.contains("internal")));
    }

    #[test]
    fn test_architecture_prompt_has_mermaid() {
        let structure = ProjectStructure {
            root: PathBuf::from("/test"),
            modules: vec![ModuleInfo {
                path: PathBuf::from("src/main.rs"),
                name: "src/main.rs".into(),
                language: "rust".into(),
                public_items: vec!["fn main".into()],
                line_count: 50,
            }],
            total_files: 1,
            total_lines: 50,
            languages: vec![("rust".into(), 50)],
        };
        let prompt = architecture_prompt(&structure, "");
        assert!(prompt.contains("Mermaid"));
        assert!(prompt.contains("src/main.rs"));
    }
}
