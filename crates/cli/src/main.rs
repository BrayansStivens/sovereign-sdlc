use anyhow::Result;
use std::path::PathBuf;

// ANSI color codes
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const MAGENTA: &str = "\x1b[35m";
const BLUE: &str = "\x1b[34m";
const GRAY: &str = "\x1b[90m";
const WHITE: &str = "\x1b[97m";
const BG_DARK: &str = "\x1b[48;5;235m";

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
    use sovereign_tui::Buddy;
    use std::io::{self, Write};

    let mut coord = Coordinator::new();
    let onboarding = coord.auto_detect_models().await;
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut buddy = Buddy::load_or_create(&project_root);

    // ── Header ──
    println!();
    println!("  {CYAN}{BOLD}╔══════════════════════════════════════════════════╗{RESET}");
    println!("  {CYAN}{BOLD}║{RESET}  {WHITE}{BOLD}Sovereign{RESET} {DIM}v{}{RESET}                          {CYAN}{BOLD}║{RESET}", env!("CARGO_PKG_VERSION"));
    println!("  {CYAN}{BOLD}║{RESET}  {DIM}Local AI Agent for Secure Development{RESET}         {CYAN}{BOLD}║{RESET}");
    println!("  {CYAN}{BOLD}╚══════════════════════════════════════════════════╝{RESET}");
    println!();

    // ── Status ──
    print_status(&coord, &buddy);

    // ── Onboarding: show missing models + install commands ──
    if let Some(onboard_msg) = &onboarding {
        println!("  {YELLOW}{BOLD}Setup{RESET}");
        let sep2 = "-".repeat(40);
        println!("  {GRAY}{sep2}{RESET}");
        for line in onboard_msg.lines() {
            if line.starts_with("  ollama pull") {
                println!("  {GREEN}{BOLD}{line}{RESET}");
            } else if line.contains("No models") || line.contains("Could not connect") {
                println!("  {RED}{line}{RESET}");
            } else {
                println!("  {DIM}{line}{RESET}");
            }
        }
        println!("  {GRAY}{sep2}{RESET}");
        println!();
    }

    // ── Buddy greeting ──
    let (idle, _, _) = buddy.data.species.frames();
    let rarity_color = match buddy.data.rarity {
        sovereign_tui::buddy::Rarity::Common => WHITE,
        sovereign_tui::buddy::Rarity::Uncommon => GREEN,
        sovereign_tui::buddy::Rarity::Rare => BLUE,
        sovereign_tui::buddy::Rarity::Epic => MAGENTA,
        sovereign_tui::buddy::Rarity::Sovereign => YELLOW,
    };
    println!("  {rarity_color}{}{RESET}", idle[0]);
    println!("  {rarity_color}{BOLD}{}{RESET}", idle[1]);
    println!("  {rarity_color}{}{RESET}", idle[2]);
    println!("  {DIM}{} the {} [{}] Lv.{}{RESET}",
        buddy.data.name, buddy.data.species.display_name(),
        buddy.data.rarity.label(), buddy.data.level);
    println!();

    let active = coord.active_model();
    println!("  {GREEN}{BOLD}Model:{RESET} {WHITE}{active}{RESET}");
    println!("  {DIM}Type {WHITE}/help{RESET}{DIM} for commands. {WHITE}Ctrl+C{RESET}{DIM} to quit.{RESET}");
    let sep = "-".repeat(50);
    println!("  {GRAY}{sep}{RESET}");

    loop {
        // ── Prompt ──
        print!("\n  {CYAN}{BOLD}>{RESET} ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_err() { break; }
        let input = input.trim();
        if input.is_empty() { continue; }

        match input {
            "/quit" | "/q" | "/exit" => {
                buddy.save();
                println!("\n  {DIM}Goodbye. {} waves.{RESET}", buddy.data.name);
                break;
            }
            "/status" | "/s" => {
                println!();
                print_status(&coord, &buddy);
            }
            "/buddy" | "/b" => {
                print_buddy(&buddy);
            }
            "/help" | "/h" => {
                println!();
                println!("  {BOLD}{WHITE}Commands{RESET}");
                let sep = "-".repeat(40);
                println!("  {GRAY}{sep}{RESET}");
                println!("  {CYAN}/model{RESET} {DIM}<name>{RESET}     Switch LLM model");
                println!("  {CYAN}/index{RESET} {DIM}[path]{RESET}     Index project for RAG");
                println!("  {CYAN}/status{RESET}            Hardware + model info");
                println!("  {CYAN}/buddy{RESET}             Companion stats");
                println!("  {CYAN}/scan{RESET} {DIM}[path]{RESET}      Security scan");
                println!("  {CYAN}/help{RESET}              This help");
                println!("  {CYAN}/quit{RESET}              Exit");
                println!();
                println!("  {DIM}Or just type a question to chat with the AI.{RESET}");
            }
            cmd if cmd.starts_with("/model ") => {
                let model = cmd.strip_prefix("/model ").unwrap().trim();
                let result = coord.set_model(model);
                println!("  {YELLOW}{result}{RESET}");
            }
            "/model" => {
                println!("  {WHITE}Active:{RESET} {GREEN}{}{RESET}", coord.force_model.as_deref()
                    .unwrap_or(coord.recommendation.dev_model));
                println!("  {DIM}Usage: /model <name>{RESET}");
            }
            cmd if cmd.starts_with("/index") => {
                let path = cmd.strip_prefix("/index").unwrap().trim();
                let target = if path.is_empty() {
                    project_root.clone()
                } else {
                    PathBuf::from(path)
                };

                println!();
                println!("  {YELLOW}Indexing{RESET} {WHITE}{}{RESET}", target.display());
                println!("  {DIM}Tier: {} | Embedding: nomic-embed-text{RESET}", coord.hw.tier);
                println!();

                match coord.index_project(&target).await {
                    Ok(result) => {
                        buddy.on_code_audited(result.chunks_indexed as u64 * 20);
                        buddy.save();
                        println!("  {GREEN}{BOLD}Done.{RESET} {result}");
                        println!("  {DIM}RAG memory active. {} gained XP!{RESET}", buddy.data.name);
                    }
                    Err(e) => println!("  {RED}Error:{RESET} {e}"),
                }
            }
            cmd if cmd.starts_with("/scan") => {
                let path = cmd.strip_prefix("/scan").unwrap().trim();
                let target = if path.is_empty() {
                    project_root.clone()
                } else {
                    PathBuf::from(path)
                };
                println!("  {YELLOW}Scanning{RESET} {WHITE}{}{RESET}...", target.display());

                let scanner = sovereign_tools::SecurityScanner::new();
                let reports = scanner.scan_all(&target);
                if reports.is_empty() {
                    println!("  {DIM}No security tools available. Install: semgrep, cargo-audit{RESET}");
                } else {
                    let (c, e, w, i) = sovereign_tools::SecurityScanner::severity_counts(&reports);
                    let total = sovereign_tools::SecurityScanner::total_findings(&reports);

                    for report in &reports {
                        println!("  {BOLD}{}{RESET}: {}", report.tool, report.summary());
                    }

                    let risk_color = if c > 0 { RED } else if e > 0 { YELLOW } else { GREEN };
                    println!("\n  {risk_color}{BOLD}{total} findings{RESET} ({RED}{c} crit{RESET} {YELLOW}{e} err{RESET} {DIM}{w} warn {i} info{RESET})");

                    buddy.update_code_quality(w as u32, total);
                    if c > 0 { buddy.on_vuln_caught(); }
                    buddy.save();
                }
            }
            prompt => {
                // Start agent session with streaming + tools
                use sovereign_query::AgentEvent;

                let (mut event_rx, _cmd_tx) = coord.start_agent_session(prompt);
                let mut streaming = false;

                while let Some(event) = event_rx.recv().await {
                    match event {
                        AgentEvent::RouteInfo(info) => {
                            println!("  {DIM}{info}{RESET}");
                            println!();
                        }
                        AgentEvent::StreamDelta(text) => {
                            if !streaming {
                                print!("  {GRAY}│{RESET} ");
                                streaming = true;
                            }
                            // Handle newlines in delta
                            for (i, part) in text.split('\n').enumerate() {
                                if i > 0 {
                                    println!();
                                    print!("  {GRAY}│{RESET} ");
                                }
                                print!("{part}");
                            }
                            io::stdout().flush().ok();
                        }
                        AgentEvent::ToolStart { name, input_summary } => {
                            if streaming { println!(); streaming = false; }
                            let summary = if input_summary.len() > 60 {
                                format!("{}...", &input_summary[..60])
                            } else {
                                input_summary
                            };
                            println!("  {YELLOW}[tool]{RESET} {WHITE}{name}{RESET}: {DIM}{summary}{RESET}");
                        }
                        AgentEvent::ToolEnd { name, output, is_error, duration_ms } => {
                            let icon = if is_error { format!("{RED}[-]{RESET}") } else { format!("{GREEN}[+]{RESET}") };
                            let lines: Vec<&str> = output.lines().take(15).collect();
                            for line in &lines {
                                println!("  {DIM}  {line}{RESET}");
                            }
                            if output.lines().count() > 15 {
                                println!("  {DIM}  ... (truncated){RESET}");
                            }
                            println!("  {icon} {DIM}{name} ({duration_ms}ms){RESET}");
                            println!();
                        }
                        AgentEvent::ToolApprovalNeeded { tool_name, tool_input, permission } => {
                            if streaming { println!(); streaming = false; }
                            println!("  {YELLOW}{BOLD}Tool needs approval:{RESET}");
                            println!("  {WHITE}{tool_name}{RESET}: {DIM}{tool_input}{RESET}");
                            println!("  {DIM}Permission: {permission:?}{RESET}");
                            print!("  {CYAN}Approve? (y/n):{RESET} ");
                            io::stdout().flush().ok();
                            let mut answer = String::new();
                            io::stdin().read_line(&mut answer).ok();
                            if answer.trim().to_lowercase().starts_with('y') {
                                let _ = _cmd_tx.send(sovereign_query::AgentCommand::Approve);
                            } else {
                                let _ = _cmd_tx.send(sovereign_query::AgentCommand::Deny);
                            }
                        }
                        AgentEvent::Done(metrics) => {
                            if streaming { println!(); }
                            println!("  {GRAY}│{RESET}");
                            println!("  {DIM}{}{RESET}", metrics.summary());
                            buddy.on_code_audited(metrics.eval_count);
                            break;
                        }
                        AgentEvent::Error(e) => {
                            if streaming { println!(); }
                            println!("  {RED}Error:{RESET} {e}");
                            println!("  {DIM}Is Ollama running? Try: ollama serve{RESET}");
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn print_status(coord: &sovereign_query::Coordinator, buddy: &sovereign_tui::Buddy) {
    let hw = &coord.hw;
    let rec = hw.tier.recommended_models();
    let ram_pct = ((hw.total_ram_gb - hw.available_ram_gb) / hw.total_ram_gb * 100.0) as u16;
    let ram_color = if ram_pct > 85 { RED } else if ram_pct > 65 { YELLOW } else { GREEN };

    println!("  {BOLD}{WHITE}Hardware{RESET}");
    println!("  {DIM}Platform{RESET}  {WHITE}{}{RESET}", hw.platform);
    println!("  {DIM}Tier{RESET}      {CYAN}{BOLD}{}{RESET}", hw.tier);
    println!("  {DIM}RAM{RESET}       {ram_color}{:.1}/{:.1} GB{RESET} {DIM}({:.1} free){RESET}",
        hw.total_ram_gb - hw.available_ram_gb, hw.total_ram_gb, hw.available_ram_gb);
    println!();
    println!("  {BOLD}{WHITE}Models{RESET}");
    println!("  {DIM}Dev{RESET}       {GREEN}{}{RESET}", rec.dev_model);
    println!("  {DIM}Audit{RESET}     {YELLOW}{}{RESET}", rec.audit_model);
    println!("  {DIM}Active{RESET}    {CYAN}{}{RESET}",
        coord.force_model.as_deref().unwrap_or(rec.dev_model));
    println!();
    println!("  {BOLD}{WHITE}Knowledge{RESET}");
    let rag_status = if coord.rag_enabled {
        format!("{GREEN}{} chunks{RESET}", coord.memory.chunk_count())
    } else {
        format!("{DIM}none — run /index{RESET}")
    };
    let grimoire_count = coord.grimoire.as_ref()
        .and_then(|g| g.count().ok()).unwrap_or(0);
    println!("  {DIM}RAG{RESET}       {rag_status}");
    println!("  {DIM}Grimoire{RESET}  {DIM}{grimoire_count} patterns{RESET}");
    println!();
}

fn print_buddy(buddy: &sovereign_tui::Buddy) {
    let (idle, _, _) = buddy.data.species.frames();
    let rarity_color = match buddy.data.rarity {
        sovereign_tui::buddy::Rarity::Common => WHITE,
        sovereign_tui::buddy::Rarity::Uncommon => GREEN,
        sovereign_tui::buddy::Rarity::Rare => BLUE,
        sovereign_tui::buddy::Rarity::Epic => MAGENTA,
        sovereign_tui::buddy::Rarity::Sovereign => YELLOW,
    };

    println!();
    println!("  {rarity_color}{}{RESET}", idle[0]);
    println!("  {rarity_color}{BOLD}{}{RESET}", idle[1]);
    println!("  {rarity_color}{}{RESET}", idle[2]);
    println!();
    println!("  {rarity_color}{BOLD}{}{RESET} {DIM}the {}{RESET} {rarity_color}[{}]{RESET}",
        buddy.data.name, buddy.data.species.display_name(), buddy.data.rarity.label());
    println!("  {DIM}Level{RESET} {WHITE}{BOLD}{}{RESET}  {DIM}XP{RESET} {CYAN}{}/{}{RESET}",
        buddy.data.level, buddy.data.xp, buddy.data.xp_for_next_level());
    println!("  {DIM}Mood{RESET}  {}{}{RESET}", buddy.mood.color_ansi(), buddy.mood.label());
    println!();
    println!("  {DIM}Lines audited{RESET}  {WHITE}{}{RESET}", buddy.data.lines_audited);
    println!("  {DIM}Vulns caught{RESET}   {WHITE}{}{RESET}", buddy.data.vulns_caught);
    println!("  {DIM}Auto-fixes{RESET}     {WHITE}{}{RESET}", buddy.data.auto_fixes);
    println!("  {DIM}Born{RESET}           {DIM}{}{RESET}", buddy.data.created_at);
    println!();
}

fn hline(n: usize) -> String {
    "-".repeat(n)
}
