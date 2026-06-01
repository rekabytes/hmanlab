//! `render_picker` — two-level model picker overlay (Ctrl+M or `/model`).
//!
//! Level 1: provider list (Ollama, z.ai, OpenRouter, …, + Connect rows).
//! Level 2: models within the selected provider.
//!
//! Both levels use ratatui `StatefulWidget` + `ListState` so the cursor
//! always scrolls into view automatically — no manual offset arithmetic.

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Clear, List, ListItem, ListState, Padding},
    Frame,
};

use crate::app::{App, ModelPickerLevel, ModelRow, ProviderRow};

use super::super::theme;

pub(in crate::ui) fn render_picker(f: &mut Frame, area: Rect, app: &mut App) {
    f.render_widget(Clear, area);

    match app.model_picker_level {
        ModelPickerLevel::Provider => render_provider_level(f, area, app),
        ModelPickerLevel::Model => render_model_level(f, area, app),
    }
}

fn render_provider_level(f: &mut Frame, area: Rect, app: &mut App) {
    // How many rows fit inside the popup border.
    let visible = area.height.saturating_sub(2) as usize;
    app.model_picker.clamp_scroll(visible);

    let items: Vec<ListItem> = app
        .model_picker
        .items
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let style = if i == app.model_picker.index {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::color::ACCENT_ALT)
                    .add_modifier(Modifier::BOLD)
            } else {
                match row {
                    ProviderRow::Active { .. } => Style::default().fg(theme::color::ASSISTANT),
                    ProviderRow::Add(_) => Style::default().fg(theme::color::USER),
                }
            };
            ListItem::new(format!(" {} ", row.display())).style(style)
        })
        .collect();

    let total = app.model_picker.items.len();
    let title = format!(
        "select provider — ↑↓ Enter Esc  ({}/{})",
        app.model_picker.index + 1,
        total
    );
    let list =
        List::new(items).block(theme::popup_block(&title, false).padding(Padding::horizontal(1)));

    let mut state = ListState::default();
    state.select(Some(app.model_picker.index));
    *state.offset_mut() = app.model_picker.scroll_offset;

    f.render_stateful_widget(list, area, &mut state);
    // Write ratatui's computed offset back so our scroll state stays in sync.
    app.model_picker.scroll_offset = state.offset();
}

fn render_model_level(f: &mut Frame, area: Rect, app: &mut App) {
    let visible = area.height.saturating_sub(2) as usize;
    app.model_picker_models.clamp_scroll(visible);

    // Determine the current provider label for the title breadcrumb.
    let provider_label = app
        .model_picker
        .selected()
        .and_then(|row| match row {
            ProviderRow::Active {
                provider, label, ..
            } => {
                let _ = provider;
                Some(label.as_str())
            }
            ProviderRow::Add(_) => None,
        })
        .unwrap_or("models");

    let active_model = app.model.clone();
    let active_provider = app.selected_extra.as_ref().map(|e| e.provider.as_str());

    let items: Vec<ListItem> = app
        .model_picker_models
        .items
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let is_current = match row {
                ModelRow::Ollama(n) => active_provider.is_none() && n == &active_model,
                ModelRow::Extra(m) => {
                    active_provider.is_some_and(|p| p == m.provider) && m.name == active_model
                }
            };
            let label = if is_current {
                format!(" {} ✓", row.display())
            } else {
                format!(" {} ", row.display())
            };
            let style = if i == app.model_picker_models.index {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::color::ACCENT_ALT)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default()
                    .fg(theme::color::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::color::FG)
            };
            ListItem::new(label).style(style)
        })
        .collect();

    let total = app.model_picker_models.items.len();
    let title = format!(
        "← {}  ({}/{})",
        provider_label,
        app.model_picker_models.index + 1,
        total
    );
    let list =
        List::new(items).block(theme::popup_block(&title, false).padding(Padding::horizontal(1)));

    let mut state = ListState::default();
    state.select(Some(app.model_picker_models.index));
    *state.offset_mut() = app.model_picker_models.scroll_offset;

    f.render_stateful_widget(list, area, &mut state);
    app.model_picker_models.scroll_offset = state.offset();
}
}
