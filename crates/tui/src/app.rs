use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::*;
use std::io::stdout;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use sovereign_core::{tui_refresh_ms, PerformanceTier};
use sovereign_query::Coordinator;
use sovereign_tools::SecurityScanner;

use crate::buddy::Buddy;
use crate::loading::{LoadingAnimation, LoadingState};

// ── Astro-inspired palette ──
const INDIGO: Color = Color::Rgb(79, 70, 229);
const CYAN_ACCENT: Color = Color::Rgb(34, 211, 238);
const SURFACE_LIGHT: Color = Color::Rgb(45, 45, 65);
const TEXT: Color = Color::Rgb(205, 214, 244);
const TEXT_DIM: Color = Color::Rgb(108, 112, 134);
const RED_ALERT: Color = Color::Rgb(243, 139, 168);
const GREEN_OK: Color = Color::Rgb(166, 227, 161);
const YELLOW_WARN: Color = Color::Rgb(249, 226, 175);
const MAUVE: Color = Color::Rgb(203, 166, 247);

/// Async generation results
enum GenResult {
    RouteInfo(String),
    Response(String),
    Error(String),
}

/// Run the TUI with async generation
pub async fn run_tui() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut coordinator = Coordinator::new();
    let scanner = SecurityScanner::new();
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut buddy = Buddy::load_or_create(&project_root);

    let mut messages: Vec<ChatMsg> = vec![
        ChatMsg::system(format!(
            "Sovereign SDLC v{} | {} | {}",
            env!("CARGO_PKG_VERSION"), coordinator.hw.platform, coordinator.hw.tier,
        )),
        ChatMsg::system(format!(
            "{} the {} [{}] joined!",
            buddy.data.name, buddy.data.species.display_name(), buddy.data.rarity.label(),
        )),
    ];
    let mut input = String::new();
    let mut cursor_pos: usize = 0;
    let mut scroll: u16 = 0;
    let mut running = true;
    let mut last_hw = Instant::now();
    let mut loading = LoadingAnimation::new();
    let mut gen_start: Option<Instant> = None;
    let mut findings: (usize, usize, usize, usize) = (0, 0, 0, 0);

    let (gen_tx, mut gen_rx) = mpsc::channel::<GenResult>(8);

    let refresh_ms = tui_refresh_ms(coordinator.hw.tier);
    let anim_ms = match coordinator.hw.tier {
        PerformanceTier::HighEnd => 80,
        PerformanceTier::Medium => 150,
        PerformanceTier::Small => 300,
        PerformanceTier::ExtraSmall => 600,
    };
    let mut last_anim = Instant::now();

    while running {
        if last_hw.elapsed() >= Duration::from_secs(3) {
            coordinator.refresh_hardware();
            last_hw = Instant::now();
        }

        if last_anim.elapsed() >= Duration::from_millis(anim_ms) {
            loading.tick();
            buddy.tick();
            if let Some(start) = gen_start {
                loading.set(LoadingState::Generating { elapsed_secs: start.elapsed().as_secs() });
            }
            last_anim = Instant::now();
        }

        // Check async results
        while let Ok(result) = gen_rx.try_recv() {
            match result {
                GenResult::RouteInfo(info) => {
                    messages.push(ChatMsg::route(info));
                    loading.set(LoadingState::Thinking);
                }
                GenResult::Response(resp) => {
                    buddy.on_code_audited(resp.lines().count() as u64);
                    // Telemetry summary
                    let elapsed = gen_start.map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
                    loading.finish_generation(elapsed, resp.len());
                    if let Some(summary) = loading.last_summary() {
                        messages.push(ChatMsg::route(summary));
                    }
                    messages.push(ChatMsg::assistant(resp));
                    loading.set(LoadingState::Idle);
                    gen_start = None;
                    scroll = 0;
                }
                GenResult::Error(e) => {
                    messages.push(ChatMsg::error(e));
                    loading.set(LoadingState::Idle);
                    gen_start = None;
                }
            }
        }

        // Render
        let hw = &coordinator.hw;
        let ram_pct = ((hw.total_ram_gb - hw.available_ram_gb) / hw.total_ram_gb * 100.0) as u16;
        let cpu_pct = hw.cpu_usage() as u16;
        buddy.update_mood(cpu_pct, ram_pct, findings.0);
        let ram_free = 100u16.saturating_sub(ram_pct);
        let active_model = coordinator.force_model.as_deref()
            .unwrap_or(coordinator.recommendation.dev_model).to_string();
        let tier = coordinator.hw.tier;
        let tools_str = scanner.available_tools().join(", ");

        terminal.draw(|frame| {
            let size = frame.area();
            let status_h = if loading.is_active() { 1 } else { 0 };

            // [Chat+Sidebar] [Status?] [Input]
            let main_v = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(6),
                    Constraint::Length(status_h),
                    Constraint::Length(3),
                ])
                .split(size);

            // [Chat 70%] [Sidebar 30%]
            let h_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(70), Constraint::Percentage(30)])
                .split(main_v[0]);

            // Sidebar: [Hardware 7] [Buddy rest]
            let sidebar = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(7), Constraint::Min(8)])
                .split(h_split[1]);

            // ── Chat ──
            render_chat(frame, &messages, scroll, h_split[0]);

            // ── Hardware ──
            render_hw(frame, cpu_pct, ram_pct, &tier, &active_model, sidebar[0]);

            // ── Buddy ──
            buddy.render(frame, sidebar[1], ram_free);

            // ── Status bar ──
            if loading.is_active() {
                render_status(frame, &loading, main_v[1]);
            }

            // ── Input ──
            render_input(frame, &input, cursor_pos, loading.is_active(), main_v[2]);
        })?;

        // Events
        if event::poll(Duration::from_millis(refresh_ms.min(80)))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }

                match key.code {
                    KeyCode::Enter if !input.is_empty() && !loading.is_active() => {
                        let user_input = input.clone();
                        input.clear();
                        cursor_pos = 0;
                        scroll = 0;
                        messages.push(ChatMsg::user(user_input.clone()));

                        match user_input.trim() {
                            "/quit" | "/q" => { buddy.save(); running = false; }
                            "/status" | "/s" => { messages.push(ChatMsg::system(coordinator.status())); }
                            "/help" | "/h" => { messages.push(ChatMsg::system(HELP.to_string())); }
                            "/buddy" | "/b" => {
                                messages.push(ChatMsg::system(format!(
                                    "{} the {} [{}] Lv.{}\nXP {}/{} | {} audited | {} vulns",
                                    buddy.data.name, buddy.data.species.display_name(),
                                    buddy.data.rarity.label(), buddy.data.level,
                                    buddy.data.xp, buddy.data.xp_for_next_level(),
                                    buddy.data.lines_audited, buddy.data.vulns_caught,
                                )));
                            }
                            cmd if cmd.starts_with("/model ") => {
                                let m = cmd.strip_prefix("/model ").unwrap().trim();
                                messages.push(ChatMsg::system(format!("{}", coordinator.set_model(m))));
                            }
                            cmd if cmd.starts_with("/index") => {
                                let p = cmd.strip_prefix("/index").unwrap().trim();
                                let target = if p.is_empty() { project_root.clone() } else { PathBuf::from(p) };
                                loading.set(LoadingState::Indexing { files_done: 0, files_total: 0 });
                                gen_start = Some(Instant::now());
                                match coordinator.index_project(&target).await {
                                    Ok(r) => {
                                        buddy.on_code_audited(r.chunks_indexed as u64 * 20);
                                        buddy.save();
                                        messages.push(ChatMsg::system(format!("{r}")));
                                    }
                                    Err(e) => messages.push(ChatMsg::error(format!("{e}"))),
                                }
                                loading.set(LoadingState::Idle);
                                gen_start = None;
                            }
                            prompt => {
                                // Async generation
                                loading.set(LoadingState::Routing);
                                gen_start = Some(Instant::now());
                                let prompt = prompt.to_string();
                                let tx = gen_tx.clone();

                                let (cat, model) = match coordinator.route_prompt(&prompt).await {
                                    Ok(r) => r,
                                    Err(e) => {
                                        messages.push(ChatMsg::error(format!("{e}")));
                                        loading.set(LoadingState::Idle);
                                        gen_start = None;
                                        continue;
                                    }
                                };

                                let rag = if coordinator.rag_enabled { " +RAG" } else { "" };
                                let _ = tx.send(GenResult::RouteInfo(format!("[{cat}{rag}] via {model}"))).await;

                                // Build context
                                let mut full = sovereign_core::system_prompt_for_tier(coordinator.hw.tier).to_string();
                                if coordinator.rag_enabled {
                                    if let Ok(emb) = coordinator.client.embed(sovereign_core::EMBEDDING_MODEL, &prompt).await {
                                        let results = coordinator.memory.search(&emb, 5);
                                        if !results.is_empty() {
                                            full.push_str("[Context]:\n");
                                            for r in &results {
                                                let c = &r.chunk.content;
                                                full.push_str(&format!("-- {} --\n{}\n",
                                                    r.chunk.file_path.display(),
                                                    &c[..c.len().min(600)]));
                                            }
                                            full.push('\n');
                                        }
                                    }
                                }
                                full.push_str(&format!("User: {prompt}"));

                                let m = model.clone();
                                let client = sovereign_api::OllamaClient::new();
                                tokio::spawn(async move {
                                    match client.generate(&m, &full).await {
                                        Ok(r) => { let _ = tx.send(GenResult::Response(r)).await; }
                                        Err(e) => { let _ = tx.send(GenResult::Error(format!("{e}"))).await; }
                                    }
                                });
                            }
                        }
                    }
                    KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => {
                        buddy.save(); running = false;
                    }
                    KeyCode::Char(c) if !loading.is_active() => {
                        input.insert(cursor_pos, c);
                        cursor_pos += 1;
                    }
                    KeyCode::Backspace if cursor_pos > 0 && !loading.is_active() => {
                        cursor_pos -= 1;
                        input.remove(cursor_pos);
                    }
                    KeyCode::Left => cursor_pos = cursor_pos.saturating_sub(1),
                    KeyCode::Right if cursor_pos < input.len() => cursor_pos += 1,
                    KeyCode::Up => scroll = scroll.saturating_add(3),
                    KeyCode::Down => scroll = scroll.saturating_sub(3),
                    KeyCode::Esc => { buddy.save(); running = false; }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

// ── Render helpers ──

fn render_chat(frame: &mut Frame, messages: &[ChatMsg], scroll: u16, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    for msg in messages {
        let (icon, color) = match msg.role {
            Role::User      => ("  you ", CYAN_ACCENT),
            Role::Assistant => ("  sov ", GREEN_OK),
            Role::System    => ("  sys ", TEXT_DIM),
            Role::Route     => ("    ~ ", INDIGO),
            Role::Error     => ("  err ", RED_ALERT),
        };
        lines.push(Line::from(Span::styled(icon, Style::default().fg(color).bold())));
        for l in msg.content.lines() {
            let pfx = if msg.role == Role::Assistant { "  \u{2502} " } else { "    " };
            lines.push(Line::from(Span::styled(
                format!("{pfx}{l}"),
                Style::default().fg(if msg.role == Role::Assistant { TEXT } else { TEXT_DIM }),
            )));
        }
        lines.push(Line::from(""));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE_LIGHT))
        .title(Span::styled(" Sovereign SDLC ", Style::default().fg(INDIGO).bold()));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }).scroll((scroll, 0)), area);
}

fn render_input(frame: &mut Frame, input: &str, cursor: usize, busy: bool, area: Rect) {
    let sym = if busy { "  \u{2026} " } else { "  \u{03bb} " };
    let color = if busy { TEXT_DIM } else { INDIGO };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color));
    frame.render_widget(
        Paragraph::new(Span::styled(format!("{sym}{input}"), Style::default().fg(if busy { TEXT_DIM } else { TEXT })))
            .block(block),
        area,
    );
    if !busy {
        frame.set_cursor_position((area.x + 5 + cursor as u16, area.y + 1));
    }
}

fn render_status(frame: &mut Frame, loading: &LoadingAnimation, area: Rect) {
    let color = match loading.state {
        LoadingState::Thinking | LoadingState::Routing => INDIGO,
        LoadingState::Generating { .. } => RED_ALERT,
        LoadingState::Indexing { .. } => YELLOW_WARN,
        LoadingState::Scanning => MAUVE,
        LoadingState::Idle => TEXT_DIM,
    };
    frame.render_widget(
        Paragraph::new(Span::styled(format!("  {}", loading.status_text()), Style::default().fg(color).bold()))
            .style(Style::default().bg(SURFACE_LIGHT)),
        area,
    );
}

fn render_hw(frame: &mut Frame, cpu: u16, ram: u16, tier: &PerformanceTier, model: &str, area: Rect) {
    let rc = if ram > 85 { RED_ALERT } else if ram > 65 { YELLOW_WARN } else { GREEN_OK };
    let cc = if cpu > 85 { RED_ALERT } else { CYAN_ACCENT };
    let lines = vec![
        Line::from(vec![
            Span::styled(" CPU ", Style::default().fg(cc).bold()),
            Span::styled(mini_bar(cpu, 10), Style::default().fg(cc)),
            Span::styled(format!(" {cpu:>2}%"), Style::default().fg(TEXT_DIM)),
        ]),
        Line::from(vec![
            Span::styled(" RAM ", Style::default().fg(rc).bold()),
            Span::styled(mini_bar(ram, 10), Style::default().fg(rc)),
            Span::styled(format!(" {ram:>2}%"), Style::default().fg(TEXT_DIM)),
        ]),
        Line::from(Span::styled(format!(" {tier}"), Style::default().fg(INDIGO).bold())),
        Line::from(Span::styled(format!(" {model}"), Style::default().fg(TEXT_DIM))),
    ];
    let block = Block::default()
        .borders(Borders::ALL).border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE_LIGHT))
        .title(Span::styled(" HW ", Style::default().fg(CYAN_ACCENT)));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn mini_bar(pct: u16, w: u16) -> String {
    let f = (pct as f32 / 100.0 * w as f32) as u16;
    format!("{}{}", "\u{2588}".repeat(f as usize), "\u{2591}".repeat((w - f) as usize))
}

// ── Message types ──

#[derive(Clone, PartialEq)]
enum Role { User, Assistant, System, Route, Error }

#[derive(Clone)]
struct ChatMsg { role: Role, content: String }

impl ChatMsg {
    fn user(s: String) -> Self { Self { role: Role::User, content: s } }
    fn assistant(s: String) -> Self { Self { role: Role::Assistant, content: s } }
    fn system(s: String) -> Self { Self { role: Role::System, content: s } }
    fn route(s: String) -> Self { Self { role: Role::Route, content: s } }
    fn error(s: String) -> Self { Self { role: Role::Error, content: s } }
}

const HELP: &str = "Commands
  /model <name>    Switch model
  /index [path]    Index for RAG
  /status          System info
  /buddy           Companion stats
  /help            Commands
  /quit            Exit
Scroll: Up/Down";
