//! Sidebar — left column with Sessions / Agents tabs.
//!
//! OpenCode-style: no border, no panel title. The sidebar sits on the
//! global BG_BASE fill and is visually distinguished from the chat
//! surface only by the slightly lighter BG_CHAT that the chat panel
//! paints on top of itself. Tabs at the top are clickable; tab
//! selection switches which list is rendered below.
//!
//! History: an earlier iteration showed a workspace file tree in this
//! column. That was removed for the flat opencode layout — file
//! browsing is now strictly via the file viewer (the chat emits
//! file paths that the user clicks).

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};
use std::path::Path;

use crate::app::App;
use crate::ui::theme;

/// Seed the initial `expanded_dirs` set for a workspace: workspace root
/// itself, plus its immediate (visible) child directories. The set is
/// still used by the inline `@file` autocomplete (`src/app/inline.rs`)
/// even though the sidebar no longer renders a tree — keeping the
/// expansion semantics consistent between sidebar and file mention.
///
/// `show_hidden` controls whether dotfile directories are pre-expanded.
/// Trusted workspaces pass `true` so `.hmanlab`, `.cargo/`, etc. open with
/// their contents visible; untrusted ones keep dotfiles hidden entirely.
pub(crate) fn initial_expanded(
    workspace: &Path,
    show_hidden: bool,
) -> std::collections::HashSet<std::path::PathBuf> {
    use std::collections::HashSet;
    use std::path::PathBuf;

    let mut out = HashSet::new();
    // The root is always implicitly expanded — keep it in the set so
    // the file-mention autocomplete uses a single membership check.
    out.insert(workspace.to_path_buf());
    let Ok(read) = std::fs::read_dir(workspace) else {
        return out;
    };
    // Same skip list the inline @file autocomplete uses — keeps the
    // two views consistent so a path the user can `@`-mention isn't
    // hidden from the sidebar (and vice versa).
    const SKIP_DIRS: &[&str] = &[
        ".git",
        ".svn",
        ".hg",
        ".jj",
        "target",
        "node_modules",
        "dist",
        "build",
        "out",
        "__pycache__",
        ".next",
        ".nuxt",
        ".turbo",
        ".venv",
        "venv",
        ".cache",
        ".pytest_cache",
        ".mypy_cache",
        ".ruff_cache",
        ".gradle",
        ".idea",
        ".vscode",
    ];
    for e in read.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !show_hidden {
            continue;
        }
        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if !is_dir || SKIP_DIRS.iter().any(|s| *s == name) {
            continue;
        }
        out.insert(PathBuf::from(e.path()));
    }
    out
}

pub(super) fn render_sidebar(f: &mut Frame, area: Rect, app: &mut App) {
    // **Critical**: Clear the full sidebar area before any paint.
    // Same defence as the inspector — chat overflow past
    // `chat_area`'s LEFT edge would otherwise leave stale characters
    // in the sidebar's buffer cells. `Block::set_style` only patches
    // style, doesn't clear `cell.symbol`. See the long comment in
    // `ui::inspector::render_inspector` for the full explanation.
    f.render_widget(ratatui::widgets::Clear, area);

    // OpenCode-style: no border, no panel title. The sidebar is just a
    // vertical column on the BG_BASE fill — visually distinguished from
    // the chat by the bg elevation (sidebar = BG_BASE, chat = BG_CHAT).
    app.render.sidebar_tab_rects.clear();
    app.render.sidebar_targets.clear();

    // Split into [tab strip | content]. The tab strip is exactly 1 row
    // tall; tabs evenly divided across `area.width`.
    let tab_chunks = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(1),
            ratatui::layout::Constraint::Min(1),
        ])
        .split(area);
    let tab_strip = tab_chunks[0];
    let content_area = tab_chunks[1];

    // Render the tab strip. Each tab is a clickable cell that sets
    // `app.active_sidebar_tab` when clicked. Active tab fills the cell
    // with the peach accent (inverse fg); inactive tabs are dim text.
    let tabs: &[(&str, crate::app::state::SidebarTab)] = &[
        ("Sessions", crate::app::state::SidebarTab::Sessions),
        ("Agents", crate::app::state::SidebarTab::Agents),
    ];
    let n = tabs.len() as u16;
    let cell_w = (tab_strip.width / n).max(1);
    let mut spans: Vec<Span> = Vec::new();
    for (i, (label, kind)) in tabs.iter().enumerate() {
        let is_active = *kind == app.active_sidebar_tab;
        let style = if is_active {
            Style::default()
                .fg(theme::color::BG_INSPECTOR)
                .bg(theme::color::TAB_ACTIVE)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::color::TAB_INACTIVE)
        };
        let padded = format!("{label:^cell_w$}", cell_w = cell_w as usize);
        spans.push(Span::styled(padded, style));
        let x = tab_strip
            .x
            .saturating_add((i as u16).saturating_mul(cell_w));
        let w = if i as u16 == n - 1 {
            tab_strip
                .width
                .saturating_sub((i as u16).saturating_mul(cell_w))
        } else {
            cell_w
        };
        app.render.sidebar_tab_rects.push((x, tab_strip.y, w, 1));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::color::BG_BASE)),
        tab_strip,
    );

    match app.active_sidebar_tab {
        crate::app::state::SidebarTab::Sessions => render_sessions_tab(f, content_area, app),
        crate::app::state::SidebarTab::Agents => render_agents_tab(f, content_area, app),
    }
}

/// Sessions tab — shows the saved JSONL sessions for the current
/// workspace, newest first. Each row shows the title (first user
/// message, truncated) and a date stamp. Read-only for now — click
/// to load wires up in a follow-up.
fn render_sessions_tab(f: &mut Frame, area: Rect, app: &mut App) {
    app.render.sidebar_x = area.x;
    app.render.sidebar_y = area.y;
    app.render.sidebar_w = area.width;
    app.render.sidebar_h = area.height;

    let mut lines: Vec<Line> = Vec::new();
    if app.local_session_summaries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No saved sessions yet.",
            Style::default().fg(theme::color::FG_DIM),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  /sessions to refresh from disk",
            Style::default()
                .fg(theme::color::FG_DIMMER)
                .add_modifier(Modifier::ITALIC),
        )));
    } else {
        for s in app.local_session_summaries.iter().take(40) {
            let title = if s.title.is_empty() {
                "(untitled)".to_string()
            } else {
                s.title.clone()
            };
            let truncated = truncate(&title, area.width.saturating_sub(2) as usize);
            lines.push(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(
                    truncated,
                    Style::default()
                        .fg(theme::color::FG)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                format!("    {}", s.updated_at),
                Style::default().fg(theme::color::FG_DIMMER),
            )));
        }
        if app.local_session_summaries.len() > 40 {
            lines.push(Line::from(Span::styled(
                format!("  …(+{} more)", app.local_session_summaries.len() - 40),
                Style::default().fg(theme::color::FG_DIMMER),
            )));
        }
    }

    let para = Paragraph::new(lines)
        .style(Style::default().bg(theme::color::BG_BASE))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(para, area);
}

/// Agents tab — lists configured specialist agents from
/// `Config::agents`. Each row shows the agent name + the model it
/// uses. The `/ask` flow stays in the chat (it requires a message);
/// this tab is a glance view of who's available.
fn render_agents_tab(f: &mut Frame, area: Rect, app: &mut App) {
    app.render.sidebar_x = area.x;
    app.render.sidebar_y = area.y;
    app.render.sidebar_w = area.width;
    app.render.sidebar_h = area.height;

    let mut lines: Vec<Line> = Vec::new();
    if app.agents.specialists.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No specialist agents.",
            Style::default().fg(theme::color::FG_DIM),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  /agents add to configure",
            Style::default()
                .fg(theme::color::FG_DIMMER)
                .add_modifier(Modifier::ITALIC),
        )));
        lines.push(Line::from(Span::styled(
            "  up to 5 specialists.",
            Style::default()
                .fg(theme::color::FG_DIMMER)
                .add_modifier(Modifier::ITALIC),
        )));
    } else {
        for a in &app.agents.specialists {
            let display = truncate(&a.name, area.width.saturating_sub(4) as usize);
            lines.push(Line::from(vec![
                Span::styled("  ⚙ ", Style::default().fg(theme::color::ACCENT_ALT)),
                Span::styled(
                    display,
                    Style::default()
                        .fg(theme::color::FG)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            let model_disp = truncate(&a.model, area.width.saturating_sub(8) as usize);
            lines.push(Line::from(Span::styled(
                format!("    {model_disp}"),
                Style::default().fg(theme::color::FG_DIMMER),
            )));
        }
    }

    let para = Paragraph::new(lines)
        .style(Style::default().bg(theme::color::BG_BASE))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(para, area);
}

/// Truncate a label to `max` display columns, appending `…` if cut. Uses
/// char count as a proxy for display width — fine for ASCII source paths.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
