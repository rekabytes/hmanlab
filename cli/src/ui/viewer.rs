//! Inline file viewer — replaces the chat panel while `app.open_file` is
//! `Some`. Read-only, line-numbered, scrollable; closed with Esc (handled in
//! `app::event`).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::App;

use super::theme;

pub(super) fn render_viewer(f: &mut Frame, area: Rect, app: &mut App) {
    let Some(file) = app.open_file.as_mut() else {
        return;
    };

    let title = format!("{} — Esc to close", file.display);
    let block = theme::panel_block(&title, true).padding(Padding::horizontal(1));
    let inner = block.inner(area);

    let mut lines: Vec<Line> = Vec::new();

    if file.loading {
        lines.push(Line::from(Span::styled(
            "loading…",
            Style::default()
                .fg(theme::color::FG_DIM)
                .add_modifier(Modifier::ITALIC),
        )));
    } else if let Some(err) = &file.error {
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default()
                .fg(theme::color::ERROR)
                .add_modifier(Modifier::BOLD),
        )));
    } else {
        // Compute line-number gutter width based on the file's line count so
        // long files don't shove their first lines off-center. Min width 3
        // keeps short files looking tidy.
        let total = file.content.lines().count().max(1);
        let gutter_w = total.to_string().len().max(3);
        for (i, raw) in file.content.lines().enumerate() {
            let n = format!("{:>w$}", i + 1, w = gutter_w);
            // Keep the raw line as a single span — no markdown parsing, no
            // wrap interpretation. The Paragraph wrap below handles overflow.
            lines.push(Line::from(vec![
                Span::styled(n, Style::default().fg(theme::color::FG_DIM)),
                Span::raw("  "),
                Span::styled(raw.to_string(), Style::default().fg(theme::color::FG)),
            ]));
        }
    }

    // Clamp scroll to a valid range so PgDn at EOF stops cleanly.
    let total = lines.len() as u16;
    let visible = inner.height;
    let max_scroll = total.saturating_sub(visible);
    if file.scroll > max_scroll {
        file.scroll = max_scroll;
    }

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((file.scroll, 0));
    f.render_widget(para, area);
}
