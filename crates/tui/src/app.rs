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
use sovereign_query::{AgentCommand, AgentEvent, Coordinator};
use sovereign_tools::SecurityScanner;
use sovereign_api::GenMetrics;

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

// AgentEvent and AgentCommand imported from sovereign_query

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
    let mut auto_scroll = true; // true = stick to bottom, false = user scrolled up
    let mut scroll_offset: u16 = 0; // only used when auto_scroll=false
    let mut running = true;
    let mut paste_range: Option<(usize, usize)> = None;
    let mut paste_counter: u32 = 0;
    let mut paste_contents: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    let mut approval_state = ApprovalState::None;
    let mut last_hw = Instant::now();
    let mut loading = LoadingAnimation::new();
    let mut gen_start: Option<Instant> = None;
    let mut findings: (usize, usize, usize, usize) = (0, 0, 0, 0);

    // Agent channels — created per-session via coordinator.start_agent_session()
    let mut agent_rx: Option<mpsc::UnboundedReceiver<AgentEvent>> = None;
    let mut agent_cmd_tx: Option<mpsc::UnboundedSender<AgentCommand>> = None;
    let mut streaming_buffer: Option<String> = None;
    let mut streaming_tokens: usize = 0;

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
            if streaming_buffer.is_some() {
                loading.set(LoadingState::Streaming { tokens: streaming_tokens });
            } else if let Some(start) = gen_start {
                loading.set(LoadingState::Generating { elapsed_secs: start.elapsed().as_secs() });
            }
            last_anim = Instant::now();
        }

        // Check agent events
        {
            let mut agent_done = false;
            if let Some(ref mut rx) = agent_rx {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        AgentEvent::RouteInfo(info) => {
                            messages.push(ChatMsg::route(info));
                            loading.set(LoadingState::Streaming { tokens: 0 });
                        }
                        AgentEvent::TextResponse(text) => {
                            messages.push(ChatMsg::assistant(text));
                            auto_scroll = true;
                        }
                        AgentEvent::ToolStart { name, input_summary } => {
                            let summary = if input_summary.len() > 80 {
                                format!("{}...", &input_summary[..80])
                            } else {
                                input_summary
                            };
                            messages.push(ChatMsg::route(format!("[tool] {name}: {summary}")));
                            loading.set(LoadingState::Thinking);
                        }
                        AgentEvent::ToolEnd { name, output, is_error, duration_ms } => {
                            let icon = if is_error { "[-]" } else { "[+]" };
                            let truncated = if output.lines().count() > 20 {
                                let first: String = output.lines().take(20).collect::<Vec<_>>().join("\n");
                                format!("{first}\n... (truncated)")
                            } else {
                                output
                            };
                            messages.push(ChatMsg::system(format!(
                                "{icon} {name} ({duration_ms}ms)\n{truncated}"
                            )));
                            loading.set(LoadingState::Thinking);
                        }
                        AgentEvent::ToolApprovalNeeded { tool_name, tool_input, permission } => {
                            let is_dangerous = permission == sovereign_tools::PermissionLevel::Execute
                                || permission == sovereign_tools::PermissionLevel::Dangerous;
                            let working_dir = std::env::current_dir()
                                .unwrap_or_else(|_| PathBuf::from("."))
                                .to_string_lossy().to_string();
                            approval_state = ApprovalState::Pending {
                                action: ProposedAction::RunCommand {
                                    command: format!("[{tool_name}] {tool_input}"),
                                    working_dir,
                                    is_dangerous,
                                    danger_reason: Some(format!("Permission: {:?}", permission)),
                                },
                                scroll: 0,
                            };
                            loading.set(LoadingState::Idle);
                        }
                        AgentEvent::Done(metrics) => {
                            buddy.on_code_audited(metrics.eval_count);
                            messages.push(ChatMsg::route(metrics.summary()));
                            loading.set(LoadingState::Idle);
                            gen_start = None;
                            auto_scroll = true;
                            agent_done = true;
                        }
                        AgentEvent::Error(e) => {
                            messages.push(ChatMsg::error(e));
                            loading.set(LoadingState::Idle);
                            gen_start = None;
                            agent_done = true;
                        }
                    }
                }
            }
            if agent_done {
                agent_rx = None;
                agent_cmd_tx = None;
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
            render_chat(frame, &messages, streaming_buffer.as_deref(), auto_scroll, scroll_offset, h_split[0]);

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
                            // Send approval to agent loop — it will execute the tool
                            if let Some(ref tx) = agent_cmd_tx {
                                let _ = tx.send(AgentCommand::Approve);
                            }
                            loading.set(LoadingState::Streaming { tokens: streaming_tokens });
                            approval_state = ApprovalState::None;
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            // Send denial to agent loop
                            if let Some(ref tx) = agent_cmd_tx {
                                let _ = tx.send(AgentCommand::Deny);
                            }
                            loading.set(LoadingState::Streaming { tokens: streaming_tokens });
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
                        auto_scroll = true;
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
                                // ── Start agent session (streaming + tools) ──
                                loading.set(LoadingState::Streaming { tokens: 0 });
                                gen_start = Some(Instant::now());
                                streaming_buffer = None;
                                streaming_tokens = 0;

                                let (rx, tx) = coordinator.start_agent_session(prompt);
                                agent_rx = Some(rx);
                                agent_cmd_tx = Some(tx);
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
                    KeyCode::Up => {
                        auto_scroll = false;
                        scroll_offset = scroll_offset.saturating_add(3);
                    }
                    KeyCode::Down => {
                        if scroll_offset <= 3 {
                            scroll_offset = 0;
                            auto_scroll = true;
                        } else {
                            scroll_offset = scroll_offset.saturating_sub(3);
                        }
                    }
                    KeyCode::Esc => {
                        if loading.is_active() {
                            // Cancel agent — send Cancel command, drop channels
                            if let Some(ref tx) = agent_cmd_tx {
                                let _ = tx.send(AgentCommand::Cancel);
                            }
                            agent_rx = None;
                            agent_cmd_tx = None;
                            streaming_buffer = None;
                            streaming_tokens = 0;
                            loading.set(LoadingState::Idle);
                            gen_start = None;
                            messages.push(ChatMsg::system("Generation cancelled.".into()));
                        }
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

fn render_chat(frame: &mut Frame, messages: &[ChatMsg], streaming: Option<&str>, auto_scroll: bool, scroll_offset: u16, area: Rect) {
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

    // Show streaming buffer as partial assistant message (strip tool JSON)
    if let Some(buf) = streaming {
        let clean = strip_tool_block(buf);
        if !clean.trim().is_empty() {
            lines.push(Line::from(Span::styled("  sov ", Style::default().fg(GREEN_OK).bold())));
            for l in clean.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  \u{2502} {l}"),
                    Style::default().fg(TEXT),
                )));
            }
            // Blinking cursor indicator
            lines.push(Line::from(Span::styled("  \u{2502} \u{2588}", Style::default().fg(CYAN_ACCENT))));
            lines.push(Line::from(""));
        }
    }

    // Scroll calculation
    let visible_height = area.height.saturating_sub(2) as usize;
    let total_lines = lines.len();
    let max_scroll = total_lines.saturating_sub(visible_height) as u16;
    let scroll_pos = if auto_scroll {
        max_scroll // always show bottom
    } else {
        max_scroll.saturating_sub(scroll_offset)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(SURFACE_LIGHT))
        .title(Span::styled(" Sovereign ", Style::default().fg(INDIGO).bold()));
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
        LoadingState::Streaming { .. } => CYAN_ACCENT,
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
        LoadingState::Streaming { .. } => SentinelMood::Generating,
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

// needs_agent() removed — all prompts now go through the agent loop

// run_agent_loop() removed — replaced by agent_loop.rs in sovereign-query

/// Strip ```tool and ```json blocks from text before displaying to user.
/// The tool call JSON is shown as [tool] indicators instead.
fn strip_tool_block(text: &str) -> String {
    let mut result = text.to_string();
    // Remove ```tool ... ``` blocks
    for marker in &["```tool", "```json"] {
        while let Some(start) = result.find(marker) {
            let after = start + marker.len();
            if let Some(end) = result[after..].find("```") {
                result.replace_range(start..after + end + 3, "");
            } else {
                // No closing ``` — remove from marker to end
                result.truncate(start);
                break;
            }
        }
    }
    // Clean up leftover backticks and whitespace from partial fences
    let result = result.trim().trim_matches('`').trim();
    result.to_string()
}
