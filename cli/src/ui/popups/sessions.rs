//! `render_session_picker` — overlay listing recent saved sessions
//! (their short id, model, and title). Triggered by `/sessions`.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Clear, List, ListItem, ListState, Padding},
    Frame,
};

use crate::app::App;

use super::super::theme;

pub(in crate::ui) fn render_session_picker(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);
    let items: Vec<ListItem> = app
        .session_picker
        .items
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let id_str = s.id.replace('-', "");
            let short = &id_str[..id_str.len().min(8)];
            let model = s.model.as_deref().unwrap_or("?");
            let title: String = s.title.chars().take(70).collect();
            let label = format!(" {short}  [{model}]  {title} ");
            let style = if i == app.session_picker.index {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::color::ACCENT_ALT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::color::FG)
            };
            ListItem::new(label).style(style)
        })
        .collect();
    let title = format!("sessions ({}) — ↑↓ Enter Esc", app.session_picker.len());
    let list =
        List::new(items).block(theme::popup_block(&title, false).padding(Padding::horizontal(1)));
    let mut state = ListState::default();
    state.select(Some(app.session_picker.index));
    f.render_stateful_widget(list, area, &mut state);
}
