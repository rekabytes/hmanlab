//! `render_disconnect_picker` — overlay listing currently-connected BYOK
//! providers (z.ai sub/usage, Ollama Cloud, OpenCode) so the user can
//! pick one to drop. Painted red because removing a key is destructive.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Clear, List, ListItem, ListState, Padding},
    Frame,
};

use crate::app::App;

use super::super::theme;

pub(in crate::ui) fn render_disconnect_picker(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = app
        .disconnect_picker
        .items
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let label = format!(" {} — {} ", e.label, e.preview);
            let style = if i == app.disconnect_picker.index {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::color::ERROR)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::color::ERROR)
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let title = format!(
        "disconnect provider ({}) — ↑↓ Enter Esc",
        app.disconnect_picker.len()
    );
    let list =
        List::new(items).block(theme::popup_block(&title, true).padding(Padding::horizontal(1)));
    let mut state = ListState::default();
    state.select(Some(app.disconnect_picker.index));
    f.render_stateful_widget(list, area, &mut state);
}
