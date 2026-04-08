//! Action Approval — Visual diff review + confirmation UI
//!
//! Renders file diffs with +/- coloring and a confirmation menu.

use ratatui::prelude::*;
use ratatui::widgets::*;
use sovereign_core::diff::{FileDiff, LineTag, ProposedAction};

// Colors (match app.rs Astro palette)
const SURFACE_LIGHT: Color = Color::Rgb(45, 45, 65);
const TEXT: Color = Color::Rgb(205, 214, 244);
const TEXT_DIM: Color = Color::Rgb(108, 112, 134);
const RED_DEL: Color = Color::Rgb(243, 139, 168);
const GREEN_ADD: Color = Color::Rgb(166, 227, 161);
const YELLOW_WARN: Color = Color::Rgb(249, 226, 175);
const INDIGO: Color = Color::Rgb(79, 70, 229);
const RED_BG: Color = Color::Rgb(60, 30, 40);
const GREEN_BG: Color = Color::Rgb(30, 55, 40);
const HEADER_FG: Color = Color::Rgb(137, 180, 250);

/// Approval state
pub enum ApprovalState {
    /// No pending approval
    None,
    /// Showing a proposed action, waiting for user input
    Pending {
        action: ProposedAction,
        scroll: u16,
    },
}

impl ApprovalState {
    pub fn is_pending(&self) -> bool {
        matches!(self, ApprovalState::Pending { .. })
    }

    pub fn scroll_up(&mut self) {
        if let ApprovalState::Pending { scroll, .. } = self {
            *scroll = scroll.saturating_add(2);
        }
    }

    pub fn scroll_down(&mut self) {
        if let ApprovalState::Pending { scroll, .. } = self {
            *scroll = scroll.saturating_sub(2);
        }
    }
}

/// Render the approval overlay on top of the chat area
pub fn render_approval(frame: &mut Frame, state: &ApprovalState, area: Rect) {
    let ApprovalState::Pending { action, scroll } = state else { return };

    // Use 80% of the area, centered
    let popup_area = centered_rect(85, 80, area);

    // Clear background
    frame.render_widget(Clear, popup_area);

    match action {
        ProposedAction::EditFile { path, diff, .. } => {
            render_diff_view(frame, path, diff, *scroll, popup_area);
        }
        ProposedAction::RunCommand { command, is_dangerous, danger_reason, .. } => {
            render_command_view(frame, command, *is_dangerous, danger_reason.as_deref(), popup_area);
        }
        ProposedAction::CreateFile { path, content } => {
            render_create_view(frame, path, content, *scroll, popup_area);
        }
    }
}

fn render_diff_view(frame: &mut Frame, path: &str, diff: &FileDiff, scroll: u16, area: Rect) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(area);

    // Diff content
    let mut lines: Vec<Line> = Vec::new();

    for dl in &diff.lines {
        let (prefix, fg, bg) = match dl.tag {
            LineTag::Header  => ("", HEADER_FG, SURFACE_LIGHT),
            LineTag::Insert  => ("+ ", GREEN_ADD, GREEN_BG),
            LineTag::Delete  => ("- ", RED_DEL, RED_BG),
            LineTag::Context => ("  ", TEXT_DIM, SURFACE_LIGHT),
        };

        let line_num = dl.line_num
            .map(|n| format!("{:>4} ", n))
            .unwrap_or_else(|| "     ".to_string());

        lines.push(Line::from(vec![
            Span::styled(line_num, Style::default().fg(TEXT_DIM)),
            Span::styled(format!("{prefix}{}", dl.content), Style::default().fg(fg).bg(bg)),
        ]));
    }

    let title = format!(" Edit: {} ({}) ", path, diff.summary());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(INDIGO))
        .title(Span::styled(title, Style::default().fg(INDIGO).bold()))
        .style(Style::default().bg(SURFACE_LIGHT));

    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((scroll, 0)),
        inner[0],
    );

    // Controls
    render_controls(frame, false, inner[1]);
}

fn render_command_view(
    frame: &mut Frame,
    command: &str,
    is_dangerous: bool,
    danger_reason: Option<&str>,
    area: Rect,
) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(area);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled("  Command:", Style::default().fg(TEXT_DIM))),
        Line::from(""),
        Line::from(Span::styled(
            format!("    $ {command}"),
            Style::default().fg(if is_dangerous { RED_DEL } else { TEXT }).bold(),
        )),
        Line::from(""),
    ];

    if let Some(reason) = danger_reason {
        lines.push(Line::from(vec![
            Span::styled("  [!] ", Style::default().fg(YELLOW_WARN).bold()),
            Span::styled(reason, Style::default().fg(YELLOW_WARN)),
        ]));
        lines.push(Line::from(""));
    }

    let border_color = if is_dangerous { RED_DEL } else { INDIGO };
    let title = if is_dangerous { " [!] Dangerous Command " } else { " Execute Command " };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, Style::default().fg(border_color).bold()))
        .style(Style::default().bg(SURFACE_LIGHT));

    frame.render_widget(Paragraph::new(lines).block(block), inner[0]);
    render_controls(frame, is_dangerous, inner[1]);
}

fn render_create_view(frame: &mut Frame, path: &str, content: &str, scroll: u16, area: Rect) {
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(area);

    let lines: Vec<Line> = content.lines()
        .enumerate()
        .map(|(i, line)| {
            Line::from(vec![
                Span::styled(format!("{:>4} ", i + 1), Style::default().fg(TEXT_DIM)),
                Span::styled(format!("+ {line}"), Style::default().fg(GREEN_ADD).bg(GREEN_BG)),
            ])
        })
        .collect();

    let title = format!(" Create: {} ({} bytes) ", path, content.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(INDIGO))
        .title(Span::styled(title, Style::default().fg(INDIGO).bold()))
        .style(Style::default().bg(SURFACE_LIGHT));

    frame.render_widget(
        Paragraph::new(lines).block(block).scroll((scroll, 0)),
        inner[0],
    );
    render_controls(frame, false, inner[1]);
}

fn render_controls(frame: &mut Frame, is_dangerous: bool, area: Rect) {
    let yes_color = if is_dangerous { YELLOW_WARN } else { GREEN_ADD };
    let controls = Line::from(vec![
        Span::styled("  (", Style::default().fg(TEXT_DIM)),
        Span::styled("y", Style::default().fg(yes_color).bold()),
        Span::styled(") Yes  (", Style::default().fg(TEXT_DIM)),
        Span::styled("n", Style::default().fg(RED_DEL).bold()),
        Span::styled(") No  (", Style::default().fg(TEXT_DIM)),
        Span::styled("e", Style::default().fg(INDIGO).bold()),
        Span::styled(") Explain  (", Style::default().fg(TEXT_DIM)),
        Span::styled("Esc", Style::default().fg(TEXT_DIM).bold()),
        Span::styled(") Cancel", Style::default().fg(TEXT_DIM)),
    ]);

    frame.render_widget(
        Paragraph::new(controls).style(Style::default().bg(SURFACE_LIGHT)),
        area,
    );
}

/// Create a centered rect within a parent area
fn centered_rect(pct_w: u16, pct_h: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_h) / 2),
            Constraint::Percentage(pct_h),
            Constraint::Percentage((100 - pct_h) / 2),
        ])
        .split(area);
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_w) / 2),
            Constraint::Percentage(pct_w),
            Constraint::Percentage((100 - pct_w) / 2),
        ])
        .split(v[1]);
    h[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_approval_state_none() {
        let state = ApprovalState::None;
        assert!(!state.is_pending());
    }

    #[test]
    fn test_approval_state_pending() {
        let diff = FileDiff::compute("test.rs", "old\n", "new\n");
        let state = ApprovalState::Pending {
            action: ProposedAction::EditFile {
                path: "test.rs".into(),
                diff,
                new_content: "new\n".into(),
            },
            scroll: 0,
        };
        assert!(state.is_pending());
    }

    #[test]
    fn test_scroll() {
        let diff = FileDiff::compute("t.rs", "a\n", "b\n");
        let mut state = ApprovalState::Pending {
            action: ProposedAction::EditFile {
                path: "t.rs".into(), diff, new_content: "b\n".into(),
            },
            scroll: 0,
        };
        state.scroll_up();
        if let ApprovalState::Pending { scroll, .. } = &state {
            assert_eq!(*scroll, 2);
        }
        state.scroll_down();
        if let ApprovalState::Pending { scroll, .. } = &state {
            assert_eq!(*scroll, 0);
        }
    }

    #[test]
    fn test_centered_rect() {
        let area = Rect::new(0, 0, 100, 50);
        let centered = centered_rect(80, 60, area);
        assert!(centered.width > 0);
        assert!(centered.height > 0);
        assert!(centered.x > 0);
        assert!(centered.y > 0);
    }
}
