//! `render_mcp_setup` — three-screen MCP web-search provider wizard.
//!
//! Screen 1 (ProviderList): navigable list of supported providers with
//!   a status badge showing whether each is configured / active.
//! Screen 2 (KeyInput):     API key entry for providers that require one.
//! Screen 3 (Confirmed):    success message after saving.
//!
//! Layout mirrors `agents_setup.rs`: four vertical chunks — body, input,
//! error, hint — so the visual language is consistent across wizards.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Padding, Paragraph, Wrap},
    Frame,
};

use super::super::theme;
use crate::app::mcp_providers::MCP_PROVIDERS;
use crate::app::{App, McpSetupScreen};

pub(in crate::ui) fn render_mcp_setup(f: &mut Frame, area: Rect, app: &App) {
    f.render_widget(Clear, area);

    match app.mcp_setup_screen {
        McpSetupScreen::ProviderList => render_provider_list(f, area, app),
        McpSetupScreen::KeyInput => render_key_input(f, area, app),
        McpSetupScreen::Confirmed => render_confirmed(f, area, app),
    }
}

// ── Screen 1: Provider List ───────────────────────────────────────────────────

fn render_provider_list(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = MCP_PROVIDERS
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let is_active = app.mcp_active_provider.as_deref() == Some(p.id);
            let is_selected = i == app.mcp_setup_index;
            let has_key = app.mcp_keys.contains_key(p.id);

            // Build the status badge on the right.
            let badge = if is_active {
                " [active ✓]"
            } else if has_key || !p.needs_key {
                if !p.needs_key {
                    " [no key needed]"
                } else {
                    " [configured]"
                }
            } else {
                " [not configured]"
            };

            let label = format!(" {}{}  —  {} ", p.label, badge, p.free_tier);

            let style = if is_selected {
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(theme::color::ACCENT_ALT)
                    .add_modifier(Modifier::BOLD)
            } else if is_active {
                Style::default()
                    .fg(theme::color::ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::color::FG)
            };

            ListItem::new(label).style(style)
        })
        .collect();

    let title = format!(
        "mcp — web search providers  ({}/{})",
        app.mcp_setup_index + 1,
        MCP_PROVIDERS.len()
    );
    let hint_line = Line::from(vec![Span::styled(
        " ↑↓ navigate  •  Enter configure  •  Esc close ",
        Style::default()
            .fg(theme::color::FG_DIM)
            .add_modifier(Modifier::ITALIC),
    )]);

    let block = theme::popup_block(&title, false)
        .title_bottom(hint_line)
        .padding(Padding::horizontal(1));

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

// ── Screen 2: API Key Input ───────────────────────────────────────────────────

fn render_key_input(f: &mut Frame, area: Rect, app: &App) {
    let provider = &MCP_PROVIDERS[app.mcp_setup_index];

    // Outer border spanning the whole area.
    let border_block = theme::popup_block(provider.label, false);
    f.render_widget(border_block, area);

    // Inner area (inside the border) with horizontal padding.
    let inner = Block::default().padding(Padding::horizontal(2)).inner(area);

    // Layout: [body | input_field | error | hint]
    let inner_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    // Body text: signup URL + free tier.
    let body = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Sign up at: ", Style::default().fg(theme::color::FG_DIM)),
            Span::styled(
                provider.signup_url,
                Style::default().fg(theme::color::ACCENT),
            ),
        ]),
        Line::from(vec![Span::styled(
            provider.free_tier,
            Style::default().fg(theme::color::FG_DIM),
        )]),
    ])
    .wrap(Wrap { trim: false });
    f.render_widget(body, inner_chunks[0]);

    // API key input field.
    let input_title = format!(" API key — {} ", provider.label);
    let input_block = Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme::color::ACCENT))
        .title(Span::styled(
            input_title,
            Style::default().fg(theme::color::FG),
        ));
    // The TextArea widget renders itself — we just hand it the block + chunk.
    let mut input_clone = app.mcp_setup_input.clone();
    input_clone.set_block(input_block);
    f.render_widget(&input_clone, inner_chunks[1]);

    // Error line.
    if let Some(err) = &app.mcp_setup_error {
        let err_line = Paragraph::new(err.as_str()).style(Style::default().fg(theme::color::ERROR));
        f.render_widget(err_line, inner_chunks[2]);
    }

    // Hint footer.
    let hint = Paragraph::new(Line::from(vec![Span::styled(
        " Enter save  •  Ctrl+D clear key  •  Esc back ",
        Style::default()
            .fg(theme::color::FG_DIM)
            .add_modifier(Modifier::ITALIC),
    )]));
    f.render_widget(hint, inner_chunks[3]);
}

// ── Screen 3: Confirmed ────────────────────────────────────────────────────────

fn render_confirmed(f: &mut Frame, area: Rect, app: &App) {
    let active_label = app
        .mcp_active_provider
        .as_deref()
        .and_then(crate::app::mcp_providers::provider_by_id)
        .map(|p| p.label)
        .unwrap_or("unknown");

    let body = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("  ✓  API key saved for {}.", active_label),
            Style::default()
                .fg(theme::color::SUCCESS)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  web_search and web_fetch tools are now active.",
            Style::default().fg(theme::color::FG),
        )]),
        Line::from(vec![Span::styled(
            "  The model can search the web during conversations.",
            Style::default().fg(theme::color::FG_DIM),
        )]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Use /mcp again to switch providers or clear your key.",
            Style::default().fg(theme::color::FG_DIM),
        )]),
    ])
    .block(theme::popup_block("mcp — configured", false));

    f.render_widget(body, area);

    // Hint footer — rendered directly on area so it sits on the border.
    let hint_y = area.y + area.height.saturating_sub(1);
    let hint_area = Rect {
        x: area.x + 1,
        y: hint_y,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    let hint = Paragraph::new(Span::styled(
        " Enter or Esc to close ",
        Style::default()
            .fg(theme::color::FG_DIM)
            .add_modifier(Modifier::ITALIC),
    ));
    f.render_widget(hint, hint_area);
}
