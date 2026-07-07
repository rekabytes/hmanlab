//! `render_inline_popup` — the floating autocomplete that appears above
//! the input box while the user is typing `/<command>` or `@<path>`.
//!
//! Different from the modal popups in this directory: it's anchored to
//! the input row instead of centred, doesn't dim the rest of the screen,
//! and is sized to its content (capped at 8 rows + 80 cols).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem},
    Frame,
};

use crate::app::{App, InlinePopup, SLASH_COMMANDS};

use super::super::theme;

pub(in crate::ui) fn render_inline_popup(f: &mut Frame, input_area: Rect, app: &App) {
    // Build the rows + title from whichever popup is active.
    let (title, rows, hint) = match &app.inline_popup {
        InlinePopup::None => return,
        InlinePopup::Slash(p) => {
            if p.matches.is_empty() {
                return;
            }
            let rows: Vec<(String, String, bool)> = p
                .matches
                .iter()
                .enumerate()
                .map(|(row_idx, &cmd_idx)| {
                    let cmd = &SLASH_COMMANDS[cmd_idx];
                    (
                        format!("/{}", cmd.name),
                        cmd.desc.to_string(),
                        row_idx == p.index,
                    )
                })
                .collect();
            (
                "slash commands",
                rows,
                "↑↓ pick  •  Tab/Enter complete  •  Esc cancel",
            )
        }
        InlinePopup::File(p) => {
            if p.matches.is_empty() {
                return;
            }
            let rows: Vec<(String, String, bool)> = p
                .matches
                .iter()
                .enumerate()
                .map(|(row_idx, &file_idx)| {
                    let path = p.workspace_files[file_idx].to_string_lossy().to_string();
                    (format!("@{path}"), String::new(), row_idx == p.index)
                })
                .collect();
            (
                "workspace files",
                rows,
                "↑↓ pick  •  Tab/Enter insert  •  Esc cancel",
            )
        }
    };

    // Popup dimensions: width = input width (or capped), height = up to 8 rows + 2 chrome.
    let max_rows: u16 = 8;
    let row_count = rows.len().min(max_rows as usize) as u16;
    let popup_h = row_count + 2; // borders
    let popup_w = input_area.width.min(80);

    // Anchor above the input box. If there's no room above (rare on small
    // terms), fall back to anchoring inside the top of the input.
    let popup_y = input_area.y.saturating_sub(popup_h);
    let popup_x = input_area.x;
    let area = Rect {
        x: popup_x,
        y: popup_y,
        width: popup_w,
        height: popup_h,
    };
    f.render_widget(Clear, area);

    let footer_title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            hint,
            Style::default()
                .fg(theme::color::FG_DIM)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::raw(" "),
    ]);
    // Custom block — light grey border (instead of the mauve
    // `popup_block` accent) so the inline autocomplete doesn't fight
    // with the input box. Title color stays peach so the
    // "slash commands" / "workspace files" label still pops.
    // Solid `BG_BASE` fill so the panel blends with the chat column
    // behind it (without it, the `Clear` widget would leave the
    // panel area in the terminal's default bg, showing as a visible
    // rectangle against the chat's `BG_BASE` fill).
    let block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme::color::FG_DIMMER))
        .style(Style::default().bg(theme::color::BG_BASE))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme::color::ACCENT)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(footer_title);

    let max_name_w: usize = rows
        .iter()
        .map(|(n, _, _)| n.chars().count())
        .max()
        .unwrap_or(0)
        + 2;
    let items: Vec<ListItem> = rows
        .into_iter()
        .take(max_rows as usize)
        .map(|(name, desc, selected)| {
            let style = if selected {
                // Selected row keeps the high-contrast black-on-peach
                // pill so the cursor lands on the active choice at a
                // glance.
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::color::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                // Slash command names are rendered in the peach accent
                // (same orange the brand badge + selected pill use) so
                // the eye catches them as "this is a command" without
                // needing a hover effect.
                Style::default()
                    .fg(theme::color::ACCENT)
                    .add_modifier(Modifier::BOLD)
            };
            let mut spans = vec![Span::styled(
                format!(" {:<w$}", name, w = max_name_w),
                style,
            )];
            if !desc.is_empty() {
                spans.push(Span::styled(
                    format!("{desc} "),
                    if selected {
                        Style::default()
                            .fg(ratatui::style::Color::Black)
                            .bg(theme::color::ACCENT)
                    } else {
                        Style::default().fg(theme::color::FG_DIM)
                    },
                ));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}
