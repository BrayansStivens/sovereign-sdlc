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
use crate::approval::{self, ApprovalState};
use sovereign_core::diff::{self, FileDiff, ProposedAction, classify_command_risk};

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

/// Messages from background tasks → TUI
enum GenResult {
    RouteInfo(String),
    Response { text: String, summary: String },
    Error(String),
    /// Agent is thinking (show in chat)
    AgentThink(String),
    /// Agent wants to execute a command — needs approval
    AgentNeedApproval { command: String, reason: String },
    /// Agent read a file (informational)
    AgentReadFile(String),
    /// Agent finished its loop
    AgentDone(String),
}

/// Approval response from TUI → Agent
enum ApprovalResponse {
    Approved,
    Denied,
}

/// Restore terminal to normal state (called on exit and panic)
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(
        std::io::stdout(),
        LeaveAlternateScreen,
        crossterm::event::DisableBracketedPaste,
        crossterm::cursor::Show,
    );
}

/// Run the TUI with async generation
pub async fn run_tui() -> Result<()> {
    // Install panic hook — ensures terminal is restored even on crash
    let original_hook = std::panic::take_hook();
    let main_thread = std::thread::current().id();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == main_thread {
            restore_terminal();
        }
        original_hook(info);
    }));

    enable_raw_mode()?;
    crossterm::execute!(
        stdout(),
        EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut coordinator = Coordinator::new();
    let onboarding = coordinator.auto_detect_models().await;
    let scanner = SecurityScanner::new();
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut buddy = Buddy::load_or_create(&project_root);

    // Splash
    let splash_art = crate::splash::SPLASH.join("\n");
    let mut messages: Vec<ChatMsg> = vec![
        ChatMsg::route(splash_art),
        ChatMsg::system(format!(
            "{} | {} | Model: {}",
            coordinator.hw.platform, coordinator.hw.tier, coordinator.active_model(),
        )),
    ];

    // Show onboarding if models are missing
    if let Some(onboard) = onboarding {
        messages.push(ChatMsg::system(onboard));
    }

    messages.push(ChatMsg::system(format!(
        "{} the {} [{}] joined!",
        buddy.data.name, buddy.data.species.display_name(), buddy.data.rarity.label(),
    )));

    let mut input = String::new();
    let mut cursor_pos: usize = 0;
    let mut scroll: u16 = 0;
    let mut running = true;
    let mut paste_range: Option<(usize, usize)> = None;
    let mut paste_counter: u32 = 0;
    let mut paste_contents: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut approval_state = ApprovalState::None;
    let mut last_hw = Instant::now();
    let mut loading = LoadingAnimation::new();
    let mut gen_start: Option<Instant> = None;
    let mut findings: (usize, usize, usize, usize) = (0, 0, 0, 0);

    let (mut gen_tx, mut gen_rx) = mpsc::channel::<GenResult>(16);
    let (mut approval_tx, mut approval_rx) = mpsc::channel::<ApprovalResponse>(1);
    let mut waiting_agent_approval = false; // true when agent is paused waiting for y/n

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
                GenResult::Response { text, summary } => {
                    buddy.on_code_audited(text.lines().count() as u64);
                    messages.push(ChatMsg::route(summary));

                    // Detect if response contains a shell command to execute
                    if let Some(action) = detect_proposed_action(&text) {
                        messages.push(ChatMsg::assistant(text));
                        approval_state = ApprovalState::Pending { action, scroll: 0 };
                    } else {
                        messages.push(ChatMsg::assistant(text));
                    }

                    loading.set(LoadingState::Idle);
                    gen_start = None;
                    scroll = 0;
                }
                GenResult::Error(e) => {
                    messages.push(ChatMsg::error(e));
                    loading.set(LoadingState::Idle);
                    gen_start = None;
                }
                GenResult::AgentThink(thought) => {
                    messages.push(ChatMsg::route(format!("[think] {thought}")));
                }
                GenResult::AgentReadFile(path) => {
                    messages.push(ChatMsg::route(format!("[read] {path}")));
                }
                GenResult::AgentNeedApproval { command, reason } => {
                    // Show approval overlay for the command
                    let (is_dangerous, danger_reason) = classify_command_risk(&command);
                    let working_dir = std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .to_string_lossy().to_string();
                    approval_state = ApprovalState::Pending {
                        action: ProposedAction::RunCommand {
                            command,
                            working_dir,
                            is_dangerous,
                            danger_reason: danger_reason.or(Some(reason)),
                        },
                        scroll: 0,
                    };
                    waiting_agent_approval = true;
                    loading.set(LoadingState::Idle); // pause spinner while waiting
                }
                GenResult::AgentDone(answer) => {
                    messages.push(ChatMsg::assistant(answer));
                    loading.set(LoadingState::Idle);
                    gen_start = None;
                    scroll = 0;
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

            // Sidebar: [Hardware 6] [Buddy 12] [Activity rest]
            let sidebar = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(6),
                    Constraint::Length(12),
                    Constraint::Min(4),
                ])
                .split(h_split[1]);

            // ── Chat ──
            render_chat(frame, &messages, scroll, h_split[0]);

            // ── Hardware ──
            render_hw(frame, cpu_pct, ram_pct, &tier, &active_model, sidebar[0]);

            // ── Buddy ──
            buddy.render(frame, sidebar[1], ram_free);

            // ── Activity panel (robot animation or project stats) ──
            render_activity(frame, &loading, &coordinator, sidebar[2]);

            // ── Status bar ──
            if loading.is_active() {
                render_status(frame, &loading, main_v[1]);
            }

            // ── Input ──
            render_input(frame, &input, cursor_pos, loading.is_active(), paste_range, main_v[2]);

            // ── Approval overlay (on top of everything) ──
            if approval_state.is_pending() {
                approval::render_approval(frame, &approval_state, main_v[0]);
            }
        })?;

        // Events
        if event::poll(Duration::from_millis(refresh_ms.min(80)))? {
            let ev = event::read()?;

            // ── Handle Paste events (native crossterm bracketed paste) ──
            if let Event::Paste(data) = &ev {
                if !loading.is_active() && !approval_state.is_pending() {
                    let line_count = data.lines().count();
                    if data.len() > 80 || line_count > 1 {
                        // Store full content, show placeholder
                        paste_counter += 1;
                        let placeholder = if line_count > 1 {
                            format!("[Pasted text #{} +{} lines]", paste_counter, line_count)
                        } else {
                            format!("[Pasted text #{}]", paste_counter)
                        };
                        paste_contents.insert(paste_counter, data.clone());
                        let start = cursor_pos;
                        for c in placeholder.chars() {
                            input.insert(cursor_pos, c);
                            cursor_pos += 1;
                        }
                        paste_range = Some((start, cursor_pos));
                    } else {
                        // Short paste — insert directly
                        for c in data.chars() {
                            input.insert(cursor_pos, c);
                            cursor_pos += 1;
                        }
                    }
                }
                continue;
            }

            if let Event::Key(key) = ev {
                if key.kind != KeyEventKind::Press { continue; }

                // ── Approval overlay key handling ──
                if approval_state.is_pending() {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') => {
                            if waiting_agent_approval {
                                // Send approval to agent — it will execute and continue
                                let _ = approval_tx.try_send(ApprovalResponse::Approved);
                                // Also execute the command here so the agent gets the observation
                                if let ApprovalState::Pending { action, .. } = &approval_state {
                                    if let ProposedAction::RunCommand { command, working_dir, .. } = action {
                                        match diff::execute_command(command, working_dir) {
                                            Ok(result) => {
                                                messages.push(ChatMsg::route(result.summary()));
                                                if !result.stdout.is_empty() {
                                                    messages.push(ChatMsg::system(result.stdout));
                                                }
                                            }
                                            Err(e) => messages.push(ChatMsg::error(e)),
                                        }
                                    }
                                }
                                waiting_agent_approval = false;
                                loading.set(LoadingState::Thinking); // agent continues
                            } else {
                                // Direct action (from LLM diff detection)
                                if let ApprovalState::Pending { action, .. } = &approval_state {
                                    match action {
                                        ProposedAction::EditFile { path, new_content, .. } => {
                                            match diff::apply_edit(path, new_content) {
                                                Ok(()) => messages.push(ChatMsg::route(format!("[+] Applied edit to {path}"))),
                                                Err(e) => messages.push(ChatMsg::error(format!("Failed: {e}"))),
                                            }
                                            buddy.on_auto_fix();
                                        }
                                        ProposedAction::RunCommand { command, working_dir, .. } => {
                                            match diff::execute_command(command, working_dir) {
                                                Ok(result) => {
                                                    messages.push(ChatMsg::route(result.summary()));
                                                    if !result.stdout.is_empty() {
                                                        messages.push(ChatMsg::system(result.stdout));
                                                    }
                                                }
                                                Err(e) => messages.push(ChatMsg::error(e)),
                                            }
                                        }
                                        ProposedAction::CreateFile { path, content } => {
                                            match diff::apply_edit(path, content) {
                                                Ok(()) => messages.push(ChatMsg::route(format!("[+] Created {path}"))),
                                                Err(e) => messages.push(ChatMsg::error(format!("Failed: {e}"))),
                                            }
                                        }
                                    }
                                }
                            }
                            approval_state = ApprovalState::None;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            if waiting_agent_approval {
                                let _ = approval_tx.try_send(ApprovalResponse::Denied);
                                waiting_agent_approval = false;
                                loading.set(LoadingState::Thinking);
                            }
                            messages.push(ChatMsg::system("Action declined.".into()));
                            approval_state = ApprovalState::None;
                        }
                        KeyCode::Char('e') | KeyCode::Char('E') => {
                            // TODO: ask LLM to explain the change
                            messages.push(ChatMsg::system("Explain not yet implemented.".into()));
                        }
                        KeyCode::Up => approval_state.scroll_up(),
                        KeyCode::Down => approval_state.scroll_down(),
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Enter if !input.is_empty() && !loading.is_active() => {
                        // Expand paste placeholders with real content
                        let mut user_input = input.clone();
                        for (id, content) in &paste_contents {
                            let placeholder_multi = format!("[Pasted text #{id} +");
                            let placeholder_single = format!("[Pasted text #{id}]");
                            if let Some(pos) = user_input.find(&placeholder_multi) {
                                // Find the closing ]
                                if let Some(end) = user_input[pos..].find(']') {
                                    user_input.replace_range(pos..pos+end+1, content);
                                }
                            } else if let Some(pos) = user_input.find(&placeholder_single) {
                                user_input.replace_range(pos..pos+placeholder_single.len(), content);
                            }
                        }
                        paste_contents.clear();
                        paste_range = None;
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
                                let is_agentic = needs_agent(&prompt);
                                let mode = if is_agentic { " Agent" } else { "" };
                                let _ = tx.send(GenResult::RouteInfo(
                                    format!("[{cat}{rag}{mode}] via {model}")
                                )).await;

                                // Build context
                                let mut full = sovereign_core::system_prompt_for_tier(coordinator.hw.tier).to_string();
                                if coordinator.rag_enabled {
                                    if let Ok(emb) = coordinator.client.embed(sovereign_core::EMBEDDING_MODEL, &prompt).await {
                                        let results = coordinator.memory.search(&emb, 5);
                                        if !results.is_empty() {
                                            full.push_str("[Context]:\n");
                                            for r in &results {
                                                let c = &r.chunk.content;
                                                let safe_end = c.len().min(600);
                                                let safe_end = (0..=safe_end).rev().find(|&i| c.is_char_boundary(i)).unwrap_or(0);
                                                full.push_str(&format!("-- {} --\n{}\n",
                                                    r.chunk.file_path.display(), &c[..safe_end]));
                                            }
                                            full.push('\n');
                                        }
                                    }
                                }

                                if is_agentic {
                                    // ── Agent mode: ReAct loop with approval ──
                                    let m = model.clone();
                                    let approval_rx_for_agent = approval_rx;
                                    // Create new approval channel for future use
                                    let (new_atx, new_arx) = mpsc::channel::<ApprovalResponse>(1);
                                    approval_tx = new_atx;
                                    approval_rx = new_arx;

                                    tokio::spawn(async move {
                                        run_agent_loop(tx, approval_rx_for_agent, m, prompt, full).await;
                                    });
                                } else {
                                    // ── Simple generation ──
                                    full.push_str(&format!("User: {prompt}"));
                                    let m = model.clone();
                                    let client = sovereign_api::OllamaClient::new();
                                    tokio::spawn(async move {
                                        match client.generate_with_metrics(&m, &full).await {
                                            Ok(metrics) => {
                                                let summary = metrics.summary();
                                                let _ = tx.send(GenResult::Response {
                                                    text: metrics.response,
                                                    summary,
                                                }).await;
                                            }
                                            Err(e) => { let _ = tx.send(GenResult::Error(format!("{e}"))).await; }
                                        }
                                    });
                                }
                            }
                        }
                    }
                    KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => {
                        buddy.save(); running = false;
                    }
                    KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'z' => {
                        // Suspend to background
                        buddy.save();
                        restore_terminal();
                        #[cfg(unix)]
                        {
                            unsafe { libc::raise(libc::SIGTSTP); }
                        }
                        // Resume
                        enable_raw_mode()?;
                        crossterm::execute!(stdout(), EnterAlternateScreen, crossterm::event::EnableBracketedPaste)?;
                        terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
                    }
                    KeyCode::Char(c) if !loading.is_active() => {
                        input.insert(cursor_pos, c);
                        cursor_pos += 1;
                    }
                    KeyCode::Backspace if cursor_pos > 0 && !loading.is_active() => {
                        // If cursor is at end of a paste placeholder, delete the whole block
                        if let Some((start, end)) = paste_range {
                            if cursor_pos == end && start < end {
                                input.drain(start..end);
                                cursor_pos = start;
                                paste_range = None;
                                continue;
                            }
                        }
                        paste_range = None;
                        cursor_pos -= 1;
                        input.remove(cursor_pos);
                    }
                    KeyCode::Left => cursor_pos = cursor_pos.saturating_sub(1),
                    KeyCode::Right if cursor_pos < input.len() => cursor_pos += 1,
                    KeyCode::Up => scroll = scroll.saturating_add(3),
                    KeyCode::Down => scroll = scroll.saturating_sub(3),
                    KeyCode::Esc => {
                        if loading.is_active() {
                            // Cancel generation — drop the channel, stop waiting
                            loading.set(LoadingState::Idle);
                            gen_start = None;
                            messages.push(ChatMsg::system("Generation cancelled.".into()));
                            // Create fresh channel (old spawned task will fail to send)
                            let (new_tx, new_rx) = mpsc::channel::<GenResult>(8);
                            gen_tx = new_tx;
                            gen_rx = new_rx;
                        }
                        // Esc when idle does nothing — use Ctrl+C or /quit to exit
                    }
                    _ => {}
                }
            }
        }
    }

    restore_terminal();
    Ok(())
}

// ── Render helpers ──

/// scroll_back = 0 means auto-scroll to bottom. >0 means user scrolled up N lines.
fn render_chat(frame: &mut Frame, messages: &[ChatMsg], scroll_back: u16, area: Rect) {
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

    // Auto-scroll: calculate scroll position from bottom
    let visible_height = area.height.saturating_sub(2) as usize; // minus borders
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height) as u16;
    let scroll_pos = if scroll_back == 0 {
        max_scroll // auto-scroll to bottom
    } else {
        max_scroll.saturating_sub(scroll_back) // user scrolled back
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE_LIGHT))
        .title(Span::styled(" Sovereign SDLC ", Style::default().fg(INDIGO).bold()));
    frame.render_widget(Paragraph::new(lines).block(block).wrap(Wrap { trim: false }).scroll((scroll_pos, 0)), area);
}

fn render_input(frame: &mut Frame, input: &str, cursor: usize, busy: bool, paste: Option<(usize, usize)>, area: Rect) {
    let sym = if busy { "  \u{2026} " } else { "  \u{03bb} " };
    let color = if busy { TEXT_DIM } else { INDIGO };

    // If paste is active, show summary instead of raw text
    let display = if let Some((start, end)) = paste {
        let pasted_len = end - start;
        let pasted_text = &input[start..end.min(input.len())];
        let line_count = pasted_text.chars().filter(|&c| c == '\n').count() + 1;
        if line_count > 1 || pasted_len > 60 {
            let prefix = &input[..start];
            let suffix = if end < input.len() { &input[end..] } else { "" };
            format!("{sym}{prefix}[Pasted +{line_count} lines]{suffix}")
        } else {
            format!("{sym}{input}")
        }
    } else {
        format!("{sym}{input}")
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color));
    frame.render_widget(
        Paragraph::new(Span::styled(&display, Style::default().fg(if busy { TEXT_DIM } else { TEXT })))
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

fn render_activity(frame: &mut Frame, loading: &LoadingAnimation, coord: &Coordinator, area: Rect) {
    use crate::splash::{self, SentinelMood};

    let mut lines: Vec<Line> = Vec::new();

    let mood = match &loading.state {
        LoadingState::Idle => SentinelMood::Idle,
        LoadingState::Routing => SentinelMood::Routing,
        LoadingState::Thinking => SentinelMood::Thinking,
        LoadingState::Generating { .. } => SentinelMood::Generating,
        LoadingState::Indexing { .. } => SentinelMood::Indexing,
        LoadingState::Scanning => SentinelMood::Thinking,
    };

    let face_color = match &mood {
        SentinelMood::Idle => TEXT_DIM,
        SentinelMood::Routing => INDIGO,
        SentinelMood::Thinking => INDIGO,
        SentinelMood::Generating => CYAN_ACCENT,
        SentinelMood::Error => RED_ALERT,
        SentinelMood::Done => GREEN_OK,
        SentinelMood::Indexing => YELLOW_WARN,
    };

    // Sentinel face in box (Houston style, 3 lines)
    let sentinel = splash::sentinel_lines(&mood, loading.tick);
    for sl in &sentinel {
        lines.push(Line::from(Span::styled(
            sl.clone(), Style::default().fg(face_color),
        )));
    }
    lines.push(Line::from(""));

    // Project stats
    let rag_status = if coord.rag_enabled {
        format!("{} chunks", coord.memory.chunk_count())
    } else {
        "run /index".into()
    };
    let grimoire_n = coord.grimoire.as_ref()
        .and_then(|g| g.count().ok()).unwrap_or(0);

    lines.push(Line::from(Span::styled(
        format!("  RAG  {rag_status}"), Style::default().fg(TEXT_DIM),
    )));
    if grimoire_n > 0 {
        lines.push(Line::from(Span::styled(
            format!("  Fix  {grimoire_n} patterns"), Style::default().fg(TEXT_DIM),
        )));
    }

    let border = if loading.is_active() { face_color } else { SURFACE_LIGHT };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Span::styled(" Sentinel ", Style::default().fg(face_color)));
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
Scroll: Up/Down
Approval: (y)es (n)o (e)xplain (Esc)cancel";

/// Detect if a prompt needs the full ReAct agent (vs simple generation)
fn needs_agent(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    let signals = [
        // File system questions
        "carpeta", "directorio", "folder", "directory", "pwd", "where am i",
        "en que carpeta", "que archivos", "what files", "list files", "ls",
        // Action requests
        "ejecuta", "execute", "run ", "corre ", "instala", "install",
        "crea ", "create ", "borra ", "delete ", "mueve", "move",
        // Investigation
        "analiza", "analyze", "revisa", "check ", "investiga", "investigate",
        "read the", "lee el", "mira el", "look at", "find the",
        "que hay en", "what's in", "show me",
        // Code actions
        "fix ", "arregla", "debug", "compile", "build",
    ];
    signals.iter().any(|s| lower.contains(s))
}

/// Run the ReAct agent loop in a background task
/// Run the tool-aware agent loop (claurst-style)
async fn run_agent_loop(
    tx: mpsc::Sender<GenResult>,
    mut approval_rx: mpsc::Receiver<ApprovalResponse>,
    model: String,
    prompt: String,
    system_context: String,
) {
    use sovereign_tools::{default_registry, parse_tool_call, ToolContext};

    let client = sovereign_api::OllamaClient::new();
    let registry = default_registry();
    let ctx = ToolContext::new();

    // Build system prompt with tool descriptions
    let tools_prompt = registry.system_prompt();
    let full_system = format!(
        "{system_context}\n{tools_prompt}\n\
         User: {prompt}"
    );

    let mut messages = full_system;
    let max_turns = 8;

    for turn in 0..max_turns {
        let _ = tx.send(GenResult::AgentThink(format!("Turn {}/{max_turns}", turn + 1))).await;

        // Ask LLM
        let response = match client.generate(&model, &messages).await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(GenResult::Error(format!("{e}"))).await;
                return;
            }
        };

        // Try to parse a tool call from the response
        match parse_tool_call(&response) {
            Some((call, thinking)) => {
                // Show thinking text if any
                if !thinking.is_empty() {
                    let short = if thinking.len() > 150 { format!("{}...", &thinking[..150]) } else { thinking };
                    let _ = tx.send(GenResult::AgentThink(short)).await;
                }

                let tool = registry.get(&call.name);
                let perm_level = tool.map(|t| t.permission_level())
                    .unwrap_or(sovereign_tools::PermissionLevel::Execute);

                // Auto-allow read-only tools, ask for everything else
                let approved = if perm_level == sovereign_tools::PermissionLevel::ReadOnly {
                    let _ = tx.send(GenResult::AgentReadFile(
                        call.input.get("path").and_then(|v| v.as_str())
                            .or_else(|| call.input.get("pattern").and_then(|v| v.as_str()))
                            .unwrap_or(&call.name).to_string()
                    )).await;
                    true
                } else {
                    // Need user approval
                    let desc = format!("{}: {}",
                        call.name,
                        call.input.get("command").and_then(|v| v.as_str())
                            .or_else(|| call.input.get("path").and_then(|v| v.as_str()))
                            .unwrap_or("(see details)")
                    );
                    let _ = tx.send(GenResult::AgentNeedApproval {
                        command: desc,
                        reason: format!("Tool: {}", call.name),
                    }).await;

                    matches!(approval_rx.recv().await, Some(ApprovalResponse::Approved))
                };

                if approved {
                    let result = registry.execute(&call, &ctx)
                        .unwrap_or_else(|e| sovereign_tools::ToolResult::error(format!("{e}")));

                    let status = if result.is_error { "Error" } else { "Result" };
                    let _ = tx.send(GenResult::AgentThink(
                        format!("[{status}] {} chars", result.output.len())
                    )).await;

                    // Feed result back to LLM
                    messages.push_str(&format!(
                        "\nAssistant: {response}\n\
                         Tool Result ({}):\n{}\n\n\
                         Based on this result, respond to the user. \
                         If you need another tool, use the ```tool format. \
                         Otherwise, just respond normally.",
                        call.name, result.output,
                    ));
                } else {
                    messages.push_str(&format!(
                        "\nAssistant: {response}\n\
                         Tool Result: Permission denied by user.\n\n\
                         The user denied this action. Respond with what you know."
                    ));
                }
            }
            None => {
                // No tool call — this is the final response
                let _ = tx.send(GenResult::AgentDone(response)).await;
                return;
            }
        }
    }

    let _ = tx.send(GenResult::AgentDone("Max turns reached.".into())).await;
}

/// Detect if LLM response contains a proposed action (edit or command)
fn detect_proposed_action(response: &str) -> Option<ProposedAction> {
    // Detect ```diff blocks → file edit
    if let Some(diff_start) = response.find("```diff") {
        let content_start = diff_start + 7;
        if let Some(diff_end) = response[content_start..].find("```") {
            let diff_block = response[content_start..content_start + diff_end].trim();

            // Try to extract file path from --- a/path line
            let file_path = diff_block.lines()
                .find(|l| l.starts_with("--- a/") || l.starts_with("--- "))
                .and_then(|l| l.strip_prefix("--- a/").or_else(|| l.strip_prefix("--- ")))
                .unwrap_or("unknown_file")
                .trim()
                .to_string();

            // Read current file if it exists
            let old_content = std::fs::read_to_string(&file_path).unwrap_or_default();

            // Try to extract new content from +++ lines
            // For now, show the diff as-is and let user decide
            let new_content = apply_diff_lines(&old_content, diff_block);
            let diff = FileDiff::compute(&file_path, &old_content, &new_content);

            if diff.has_changes() {
                return Some(ProposedAction::EditFile {
                    path: file_path,
                    diff,
                    new_content,
                });
            }
        }
    }

    // Detect ```bash or ```sh blocks → command execution
    for marker in &["```bash", "```sh", "```shell"] {
        if let Some(cmd_start) = response.find(marker) {
            let content_start = cmd_start + marker.len();
            if let Some(cmd_end) = response[content_start..].find("```") {
                let command = response[content_start..content_start + cmd_end].trim().to_string();

                // Skip if it looks like an install instruction (ollama pull, apt, brew)
                let lower = command.to_lowercase();
                if lower.starts_with("ollama ") || lower.starts_with("brew ") || lower.starts_with("apt ") {
                    continue;
                }

                if !command.is_empty() && command.lines().count() <= 3 {
                    let (is_dangerous, danger_reason) = classify_command_risk(&command);
                    let working_dir = std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .to_string_lossy().to_string();

                    return Some(ProposedAction::RunCommand {
                        command,
                        working_dir,
                        is_dangerous,
                        danger_reason,
                    });
                }
            }
        }
    }

    None
}

/// Simple diff application: take + lines from a diff block as the new content
fn apply_diff_lines(old: &str, diff_block: &str) -> String {
    let mut result = String::new();
    let mut has_diff_lines = false;

    for line in diff_block.lines() {
        if line.starts_with('+') && !line.starts_with("+++") {
            has_diff_lines = true;
            result.push_str(line.strip_prefix('+').unwrap_or(line));
            result.push('\n');
        } else if line.starts_with(' ') {
            has_diff_lines = true;
            result.push_str(line.strip_prefix(' ').unwrap_or(line));
            result.push('\n');
        }
    }

    // If no diff-style lines found, return old content
    if has_diff_lines { result } else { old.to_string() }
}
