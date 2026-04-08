use anyhow::Result;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("sovereign=info,sovereign_core=info,sovereign_query=info")
        .with_target(false)
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--repl" || a == "-r") {
        run_repl().await
    } else {
        sovereign_tui::run_tui().await
    }
}

async fn run_repl() -> Result<()> {
    use sovereign_query::Coordinator;
    use std::io::{self, Write};

    let mut coord = Coordinator::new();

    println!("╔══════════════════════════════════════════════╗");
    println!("║       Sovereign SDLC v{}              ║", env!("CARGO_PKG_VERSION"));
    println!("║     S-SDLC Security Agent (REPL mode)       ║");
    println!("╚══════════════════════════════════════════════╝");
    println!();
    println!("{}", coord.status());
    println!();
    println!("  Type /help for commands. Ctrl-C to quit.");
    println!("─────────────────────────────────────────────────");

    loop {
        print!("\n  sovereign > ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() { break; }
        let input = input.trim();
        if input.is_empty() { continue; }

        match input {
            "/quit" | "/q" | "/exit" => {
                println!("  Goodbye.");
                break;
            }
            "/status" | "/s" => {
                println!("\n{}", coord.status());
            }
            "/help" | "/h" => {
                println!("\n  /model <name>     Switch model (SafeLoad validated)");
                println!("  /index [path]     Index project for RAG memory");
                println!("  /status           Hardware + model + memory status");
                println!("  /scan [path]      Security scan (SAST/SCA)");
                println!("  /audit            Toggle OWASP audit mode");
                println!("  /quit             Exit");
            }
            cmd if cmd.starts_with("/model ") => {
                let model = cmd.strip_prefix("/model ").unwrap().trim();
                let result = coord.set_model(model);
                println!("  {result}");
            }
            cmd if cmd.starts_with("/index") => {
                let path = cmd.strip_prefix("/index").unwrap().trim();
                let target = if path.is_empty() {
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                } else {
                    PathBuf::from(path)
                };

                println!("  Indexing {}... (tier: {})", target.display(), coord.hw.tier);
                println!("  Embedding model: nomic-embed-text");
                println!("  This may take a moment on CPU-only systems.\n");

                match coord.index_project(&target).await {
                    Ok(result) => {
                        println!("  {result}");
                        println!("  RAG memory is now active.");
                    }
                    Err(e) => println!("  [INDEX ERROR] {e}"),
                }
            }
            prompt => {
                match coord.route_prompt(prompt).await {
                    Ok((cat, model)) => {
                        let rag_tag = if coord.rag_enabled { " +RAG" } else { "" };
                        println!("  [{cat}{rag_tag}] → {model}");
                        match coord.generate(&model, prompt).await {
                            Ok(resp) => println!("\n{resp}"),
                            Err(e) => println!("  [ERROR] {e}"),
                        }
                    }
                    Err(e) => println!("  [ERROR] {e}"),
                }
            }
        }
    }

    Ok(())
}
