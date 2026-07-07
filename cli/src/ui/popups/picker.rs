//! `render_picker` — two-level model picker (Ctrl+M or `/model`).
//!
//! Modal: renders into the bottom half of the chat column (the
//! `popup_area` slice from `ui::mod::render`). The chat shrinks to
//! the top half so the conversation context is still visible while
//! the user picks. Opencode-style internal styling — light grey
//! border, peach title, BG_BASE fill — but the panel position is
//! the classic bottom-half modal layout.
//!
//! Two levels share the same panel:
//!   - Level 1 (`Provider`): provider list — Ollama, z.ai, OpenRouter,
//!     …, plus `+ Connect` rows for unconfigured providers.
//!   - Level 2 (`Model`): models within the selected provider.
//!
//! Both levels use ratatui `StatefulWidget` + `ListState` so the
//! cursor always scrolls into view automatically — no manual offset
//! arithmetic.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState},
    Frame,
};

use crate::app::{App, ModelPickerLevel, ModelRow, ProviderRow};

use super::super::theme;

pub(in crate::ui) fn render_picker(f: &mut Frame, area: Rect, app: &mut App) {
    // Fill the bottom half (the `popup_area` slice passed in by the
    // dispatcher). `Clear` wipes any chat content the bottom-half
    // slice inherited; the Block's BG_BASE fill then paints over
    // with our solid panel bg.
    f.render_widget(Clear, area);

    match app.model_picker_level {
        ModelPickerLevel::Provider => render_provider_level(f, area, app),
        ModelPickerLevel::Model => render_model_level(f, area, app),
    }
}

fn render_provider_level(f: &mut Frame, area: Rect, app: &mut App) {
    let visible = area.height.saturating_sub(2) as usize;
    app.model_picker.clamp_scroll(visible);

    let items: Vec<ListItem> = app
        .model_picker
        .items
        .iter()
        .enumerate()
        .map(|(i, row)| {
            // Match the slash-popup selected style: black-on-peach
            // pill for the active row, otherwise role-coloured text
            // (cyan for configured providers, green for + Connect).
            let style = if i == app.model_picker.index {
                Style::default()
                    .fg(Color::Black)
                    .bg(theme::color::ACCENT)
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
        "select provider · ↑↓ Enter Esc  ({}/{})",
        app.model_picker.index + 1,
        total
    );
    let block = panel_block(&title);

    let list = List::new(items).block(block);

    let mut state = ListState::default();
    state.select(Some(app.model_picker.index));
    *state.offset_mut() = app.model_picker.scroll_offset;

    f.render_stateful_widget(list, area, &mut state);
    app.model_picker.scroll_offset = state.offset();
}

fn render_model_level(f: &mut Frame, area: Rect, app: &mut App) {
    let visible = area.height.saturating_sub(2) as usize;
    app.model_picker_models.clamp_scroll(visible);

    let provider_label = app
        .model_picker
        .selected()
        .and_then(|row| match row {
            ProviderRow::Active { label, .. } => Some(label.as_str()),
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
                    .fg(Color::Black)
                    .bg(theme::color::ACCENT)
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
        "← {} · ↑↓ Enter Esc  ({}/{})",
        provider_label,
        app.model_picker_models.index + 1,
        total
    );
    let block = panel_block(&title);

    let list = List::new(items).block(block);

    let mut state = ListState::default();
    state.select(Some(app.model_picker_models.index));
    *state.offset_mut() = app.model_picker_models.scroll_offset;

    f.render_stateful_widget(list, area, &mut state);
    app.model_picker_models.scroll_offset = state.offset();
}

/// Build the panel `Block` — matches the slash-commands popup
/// (`ui::popups::inline`) exactly so the two pickers feel like one
/// design language:
///   - Rounded border in `FG_DIMMER` (light grey)
///   - Title in `ACCENT` (peach) bold
///   - Solid `BG_BASE` fill so the panel blends seamlessly with the
///     chat column behind it. Without this the `Clear` widget leaves
///     the panel area in the terminal's default bg, which reads as
///     a visible rectangle against the chat's `BG_BASE` fill.
///
/// Kept as a private helper here (rather than in `theme`) because
/// the styling is specific to floating picker panels — the modal
/// popups (AddModel, etc.) still use `theme::popup_block` with the
/// mauve accent until/unless we migrate them too.
fn panel_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme::color::FG_DIMMER))
        .style(Style::default().bg(theme::color::BG_BASE))
        .title(Span::styled(
            format!(" {title} "),
            Style::default()
                .fg(theme::color::ACCENT)
                .add_modifier(Modifier::BOLD),
        ))
}
