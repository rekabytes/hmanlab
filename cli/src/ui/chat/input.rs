//! `render_input` — the bottom textarea. No border in the opencode-
//! style flat layout; the input sits on BG_BASE with a peach underline
//! (via `tui-textarea`'s cursor-line style) as the focus affordance.

use ratatui::{layout::Rect, style::Style, widgets::Padding, Frame};

use crate::app::App;

use super::super::theme;

pub(in crate::ui) fn render_input(f: &mut Frame, area: Rect, app: &mut App) {
    let first_line = app.input.lines().first().cloned().unwrap_or_default();
    let is_cmd = first_line.trim_start().starts_with('/');

    // Title encodes input mode. Border colour used to echo it but with
    // the opencode-style flat layout there's no border — the title
    // itself is the only mode signal, so we render it as a small
    // dim prefix above the textarea (handled in the render block below).
    let mut title = if app.turn.is_generating() {
        "▎ generating · Ctrl+C to cancel".to_string()
    } else if is_cmd {
        "▎ command · Enter to run".to_string()
    } else if app.yn_pending {
        "▎ [Y] yes  ·  [N] no  ·  type to override".to_string()
    } else {
        "▎ message".to_string()
    };

    // Append a queued-attachments summary to the title so the user knows
    // an image will ride along on the next send. Names are truncated to
    // keep the title from blowing past the available width — full list
    // is always visible in the chat log from the /attach confirmation.
    if !app.pending_attachments.is_empty() {
        let n = app.pending_attachments.len();
        let names: Vec<String> = app
            .pending_attachments
            .iter()
            .take(3)
            .map(|a| a.filename.clone())
            .collect();
        let mut summary = names.join(", ");
        if n > 3 {
            summary.push_str(&format!(", +{} more", n - 3));
        }
        title.push_str(&format!(" · attached {n}: {summary}"));
    }

    // The input box is the ONLY elevated surface in the flat layout —
    // BG_CHAT (one step lighter than the global BG_BASE) — so it reads as
    // the focused affordance without needing a border. No border.
    // Horizontal padding stays so long inputs don't run to the edge;
    // 1 row of top padding gives the textarea breathing room from
    // the chat content above (no glued-to-chat feel).
    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::NONE)
        .style(Style::default().bg(theme::color::BG_CHAT))
        .padding(Padding {
            left: 1,
            right: 1,
            top: 1,
            bottom: 0,
        });
    app.input.set_block(block);
    // Stash the inner content width so the event handler can soft-wrap
    // typed characters before they push the cursor off the right edge.
    // -2 for the horizontal padding (no border subtract now).
    app.render.input_inner_w = area.width.saturating_sub(2);
    f.render_widget(&app.input, area);
}
