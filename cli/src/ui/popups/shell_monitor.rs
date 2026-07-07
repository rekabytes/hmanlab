//! `Mode::ShellMonitor` — live-output overlay for the in-flight (or
//! just-finished) `run_command`. Centered in the chat area, scrollable
//! with PgUp/PgDn, dismissed with Esc (shell keeps running), killed
//! with Ctrl+C (shell dies, overlay stays so the final output is
//! readable until Esc).

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::App;

use super::super::theme;

pub(in crate::ui) fn render_shell_monitor(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(rt) = app.active_shell.as_ref() else {
        // Mode is ShellMonitor but the runtime got cleared underneath
        // us — render nothing, the input handler will route back to
        // Chat on the next key.
        return;
    };

    // Modal popups now fill the bottom half of the chat column edge to
    // edge — `area` is already exactly that rect. No interior centering.
    let panel = area;

    // Status badge — colors carry the lifecycle: peach=running,
    // green=clean exit, red=non-zero or signal-killed.
    let (badge_text, badge_color): (String, Color) = if rt.running {
        ("● running".into(), theme::color::TOOL)
    } else {
        match rt.exit_code {
            Some(Some(0)) => ("✓ exit 0".into(), theme::color::SUCCESS),
            Some(Some(code)) => (format!("✗ exit {code}"), theme::color::ERROR),
            Some(None) | None => ("✗ killed".into(), theme::color::ERROR),
        }
    };

    let elapsed = rt.started_at.elapsed().as_secs();
    let title = Line::from(vec![
        Span::styled(" shell monitor ", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(
            badge_text,
            Style::default()
                .fg(badge_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {elapsed}s"),
            Style::default().fg(theme::color::FG_DIMMER),
        ),
        Span::styled(" ", Style::default()),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::color::ACCENT))
        .title(title)
        .padding(Padding::new(1, 1, 0, 0));

    let inner = block.inner(panel);
    f.render_widget(Clear, panel);
    f.render_widget(block, panel);

    // Split inner into [command row | output | footer hint].
    let inner_v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

    // Command row — `$ <cmd>` in dim grey, BOLD so it visually anchors
    // the panel even when the output below scrolls past.
    let cmd_line = Line::from(vec![
        Span::styled("$ ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled(
            rt.command.as_str(),
            Style::default()
                .fg(theme::color::FG)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(
        Paragraph::new(cmd_line).wrap(Wrap { trim: false }),
        inner_v[0],
    );

    // Build output lines. stderr in red, stdout in default fg. A
    // trailing blinking cursor sits on the last line while running, so
    // the panel reads as live even during quiet stretches.
    let mut lines: Vec<Line> = Vec::with_capacity(rt.output.len() + 1);
    for (text, is_stderr) in &rt.output {
        let style = if *is_stderr {
            Style::default().fg(theme::color::ERROR)
        } else {
            Style::default().fg(theme::color::FG)
        };
        lines.push(Line::from(Span::styled(text.clone(), style)));
    }
    if rt.running {
        let caret = if (app.anim_tick / 4) % 2 == 0 {
            "▌"
        } else {
            " "
        };
        if let Some(last) = lines.last_mut() {
            last.spans
                .push(Span::styled(caret, Style::default().fg(theme::color::FG)));
        } else {
            // No output yet — show the cursor on its own line so the
            // user has a visual cue that we're connected and waiting.
            lines.push(Line::from(Span::styled(
                caret,
                Style::default().fg(theme::color::FG),
            )));
        }
    } else if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "(no output)",
            Style::default()
                .fg(theme::color::FG_DIMMER)
                .add_modifier(Modifier::ITALIC),
        )));
    }

    // Auto-follow tail unless the user has scrolled up. ratatui's
    // Paragraph scroll is in "lines from top", so to follow the tail
    // we compute (total_lines - visible_lines).
    let visible_h = inner_v[1].height as usize;
    let total = lines.len();
    let scroll_y = if rt.follow_tail {
        total.saturating_sub(visible_h)
    } else {
        rt.scroll as usize
    } as u16;

    let body = Paragraph::new(lines)
        .scroll((scroll_y, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(body, inner_v[1]);

    // Footer hints — different copy depending on lifecycle stage so
    // Ctrl+C isn't advertised when there's nothing left to kill.
    let footer = if rt.running {
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(theme::color::FG_DIM)),
            Span::styled(" hide  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
            Span::styled("Ctrl+C", Style::default().fg(theme::color::FG_DIM)),
            Span::styled(
                " kill shell  ·  ",
                Style::default().fg(theme::color::FG_DIMMER),
            ),
            Span::styled("PgUp/PgDn", Style::default().fg(theme::color::FG_DIM)),
            Span::styled(" scroll", Style::default().fg(theme::color::FG_DIMMER)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(theme::color::FG_DIM)),
            Span::styled(
                " dismiss  ·  ",
                Style::default().fg(theme::color::FG_DIMMER),
            ),
            Span::styled("PgUp/PgDn", Style::default().fg(theme::color::FG_DIM)),
            Span::styled(" scroll", Style::default().fg(theme::color::FG_DIMMER)),
        ])
    };
    f.render_widget(
        Paragraph::new(footer).alignment(Alignment::Right),
        inner_v[2],
    );
}
