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

use sovereign_core::{tui_refresh_ms, PerformanceTier};
use sovereign_query::Coordinator;
use sovereign_tools::SecurityScanner;

use crate::buddy::Buddy;

/// Run the full TUI application
pub async fn run_tui() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut coordinator = Coordinator::new();
    let scanner = SecurityScanner::new();
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut buddy = Buddy::load_or_create(&project_root);

    let mut messages: Vec<(String, String)> = vec![
        ("system".into(), format!(
            "Sovereign SDLC v{} — {} — {}",
            env!("CARGO_PKG_VERSION"),
            coordinator.hw.platform,
            coordinator.hw.tier,
        )),
        ("system".into(), format!(
            "Buddy: {} the {} [{}] joined!",
            buddy.data.name,
            buddy.data.species.display_name(),
            buddy.data.rarity.label(),
        )),
    ];
    let mut input = String::new();
    let mut cursor_pos: usize = 0;
    let mut running = true;
    let mut last_refresh = Instant::now();
    let mut last_anim = Instant::now();

    // Security dashboard state
    let mut findings: (usize, usize, usize, usize) = (0, 0, 0, 0);

    // Hardware-adaptive refresh rate
    let refresh_ms = tui_refresh_ms(coordinator.hw.tier);
    let anim_interval = match coordinator.hw.tier {
        PerformanceTier::HighEnd => Duration::from_millis(100),  // 10 FPS
        PerformanceTier::Medium  => Duration::from_millis(250),  // 4 FPS
        PerformanceTier::Small   => Duration::from_millis(500),  // 2 FPS
        PerformanceTier::ExtraSmall => Duration::from_millis(1000), // 1 FPS
    };

    while running {
        // Refresh hardware every 2 seconds
        if last_refresh.elapsed() >= Duration::from_secs(2) {
            coordinator.refresh_hardware();
            last_refresh = Instant::now();
        }

        // Advance buddy animation
        if last_anim.elapsed() >= anim_interval {
            buddy.tick();
            last_anim = Instant::now();
        }

        terminal.draw(|frame| {
            let size = frame.area();

            // Main: Left 68% | Right 32%
            let main_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
                .split(size);

            // Left: Chat + Input
            let left = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(3)])
                .split(main_split[0]);

            // Right: Hardware 25% | Security 35% | Buddy 40%
            let right = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(25),
                    Constraint::Percentage(35),
                    Constraint::Percentage(40),
                ])
                .split(main_split[1]);

            // ── Chat Panel ──
            render_chat(frame, &messages, left[0]);

            // ── Input ──
            render_input(frame, &input, cursor_pos, left[1]);

            // ── Hardware Monitor ──
            coordinator.hw.refresh();
            let hw = &coordinator.hw;
            let ram_pct = ((hw.total_ram_gb - hw.available_ram_gb) / hw.total_ram_gb * 100.0) as u16;
            let cpu_pct = hw.cpu_usage() as u16;
            render_hardware(frame, hw, cpu_pct, ram_pct, right[0]);

            // ── Security Dashboard ──
            render_security(frame, &scanner, findings, right[1]);

            // ── Buddy Panel ──
            let (c, _, _, _) = findings;
            buddy.update_mood(cpu_pct, ram_pct, c);
            let ram_free_pct = 100u16.saturating_sub(ram_pct);
            buddy.render(frame, right[2], ram_free_pct);
        })?;

        if event::poll(Duration::from_millis(refresh_ms))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press { continue; }

                // Tick buddy on user input (ExtraSmall mode animates on input)
                if coordinator.hw.tier == PerformanceTier::ExtraSmall {
                    buddy.tick();
                }

                match key.code {
                    KeyCode::Enter if !input.is_empty() => {
                        let user_input = input.clone();
                        input.clear();
                        cursor_pos = 0;

                        messages.push(("you".into(), user_input.clone()));

                        match user_input.trim() {
                            "/quit" | "/q" => {
                                buddy.save();
                                running = false;
                                continue;
                            }
                            "/status" | "/s" => {
                                messages.push(("system".into(), coordinator.status()));
                            }
                            "/help" | "/h" => {
                                messages.push(("system".into(), HELP_TEXT.to_string()));
                            }
                            "/buddy" => {
                                messages.push(("system".into(), format!(
                                    "── {} ──\n\
                                     Species: {} [{}]\n\
                                     Level: {} (XP: {}/{})\n\
                                     Lines audited: {}\n\
                                     Vulns caught: {}\n\
                                     Born: {}",
                                    buddy.data.name,
                                    buddy.data.species.display_name(),
                                    buddy.data.rarity.label(),
                                    buddy.data.level,
                                    buddy.data.xp,
                                    buddy.data.xp_for_next_level(),
                                    buddy.data.lines_audited,
                                    buddy.data.vulns_caught,
                                    buddy.data.created_at,
                                )));
                            }
                            cmd if cmd.starts_with("/index") => {
                                let path = cmd.strip_prefix("/index").unwrap().trim();
                                let target = if path.is_empty() {
                                    project_root.clone()
                                } else {
                                    PathBuf::from(path)
                                };
                                messages.push(("system".into(), format!(
                                    "Indexing {}... ({})", target.display(), coordinator.hw.tier
                                )));

                                match coordinator.index_project(&target).await {
                                    Ok(result) => {
                                        // Buddy earns XP for indexing
                                        buddy.on_code_audited(result.chunks_indexed as u64 * 20);
                                        buddy.save();
                                        messages.push(("system".into(), format!(
                                            "{result}\nRAG active. {} gained XP!",
                                            buddy.data.name,
                                        )));
                                    }
                                    Err(e) => messages.push(("system".into(), format!("Error: {e}"))),
                                }
                            }
                            cmd if cmd.starts_with("/model ") => {
                                let model = cmd.strip_prefix("/model ").unwrap().trim();
                                let result = coordinator.set_model(model);
                                messages.push(("system".into(), format!("{result}")));
                            }
                            prompt => {
                                match coordinator.route_prompt(prompt).await {
                                    Ok((cat, model)) => {
                                        let rag = if coordinator.rag_enabled { " +RAG" } else { "" };
                                        messages.push(("system".into(), format!("[{cat}{rag}] -> {model}")));
                                        match coordinator.generate(&model, prompt).await {
                                            Ok(resp) => {
                                                // Buddy earns XP for work
                                                buddy.on_code_audited(resp.lines().count() as u64);
                                                messages.push(("sovereign".into(), resp));
                                            }
                                            Err(e) => messages.push(("system".into(), format!("Error: {e}"))),
                                        }
                                    }
                                    Err(e) => messages.push(("system".into(), format!("Router error: {e}"))),
                                }
                            }
                        }
                    }
                    KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'c' => {
                        buddy.save();
                        running = false;
                    }
                    KeyCode::Char(c) => {
                        input.insert(cursor_pos, c);
                        cursor_pos += 1;
                    }
                    KeyCode::Backspace if cursor_pos > 0 => {
                        cursor_pos -= 1;
                        input.remove(cursor_pos);
                    }
                    KeyCode::Left => cursor_pos = cursor_pos.saturating_sub(1),
                    KeyCode::Right if cursor_pos < input.len() => cursor_pos += 1,
                    KeyCode::Esc => {
                        buddy.save();
                        running = false;
                    }
                    _ => {}
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

// ────────────────────────────────────────────────────────
// Render helpers
// ────────────────────────────────────────────────────────

fn render_chat(frame: &mut Frame, messages: &[(String, String)], area: Rect) {
    let chat_lines: Vec<Line> = messages.iter().flat_map(|(role, content)| {
        let color = match role.as_str() {
            "you"       => Color::Cyan,
            "sovereign" => Color::Green,
            "system"    => Color::Yellow,
            "security"  => Color::Red,
            _           => Color::White,
        };
        let mut lines = vec![Line::from(Span::styled(
            format!("  {role} > "),
            Style::default().fg(color).bold(),
        ))];
        for l in content.lines() {
            lines.push(Line::from(format!("    {l}")));
        }
        lines.push(Line::from(""));
        lines
    }).collect();

    frame.render_widget(
        Paragraph::new(chat_lines)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Sovereign SDLC "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_input(frame: &mut Frame, input: &str, cursor_pos: usize, area: Rect) {
    frame.render_widget(
        Paragraph::new(input)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(" Input ")),
        area,
    );
    frame.set_cursor_position((area.x + cursor_pos as u16 + 1, area.y + 1));
}

fn render_hardware(
    frame: &mut Frame,
    hw: &sovereign_core::HardwareEnv,
    cpu_pct: u16,
    ram_pct: u16,
    area: Rect,
) {
    let rec = hw.tier.recommended_models();
    let ram_color = if ram_pct > 85 { Color::Red }
        else if ram_pct > 65 { Color::Yellow }
        else { Color::Green };

    let lines = vec![
        Line::from(vec![
            Span::styled(format!("  CPU {cpu_pct:>3}% "), Style::default().fg(Color::Cyan)),
            Span::styled(bar(cpu_pct, 12), Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled(format!("  RAM {ram_pct:>3}% "), Style::default().fg(ram_color)),
            Span::styled(bar(ram_pct, 12), Style::default().fg(ram_color)),
        ]),
        Line::from(Span::styled(
            format!("  {} | {}", rec.dev_model, rec.audit_model),
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(format!(" {} ", hw.tier))),
        area,
    );
}

fn render_security(
    frame: &mut Frame,
    scanner: &SecurityScanner,
    findings: (usize, usize, usize, usize),
    area: Rect,
) {
    let (c, e, w, i) = findings;
    let lines = vec![
        Line::from(Span::styled(
            format!("  Tools: {}", scanner.available_tools().join(", ")),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {c} "), Style::default().fg(
                if c > 0 { Color::Red } else { Color::DarkGray }).bold()),
            Span::styled("CRIT ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{e} "), Style::default().fg(
                if e > 0 { Color::LightRed } else { Color::DarkGray })),
            Span::styled("ERR ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{w} "), Style::default().fg(
                if w > 0 { Color::Yellow } else { Color::DarkGray })),
            Span::styled("WARN ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{i} "), Style::default().fg(Color::DarkGray)),
            Span::styled("INFO", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!("  Total: {}", c + e + w + i),
            Style::default().fg(Color::White),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red))
                .title(" Security ")),
        area,
    );
}

fn bar(pct: u16, w: u16) -> String {
    let filled = (pct as f32 / 100.0 * w as f32) as u16;
    format!("[{}{}]", "|".repeat(filled as usize), ".".repeat((w.saturating_sub(filled)) as usize))
}

const HELP_TEXT: &str = "\
── Sovereign SDLC Commands ──

  /model <name>    Switch model (SafeLoad)
  /index [path]    Index project for RAG
  /status          Hardware + model + memory
  /buddy           Show buddy stats
  /scan [path]     Security scan (SAST/SCA)
  /audit           Toggle OWASP audit mode
  /help            This help
  /quit            Exit";
