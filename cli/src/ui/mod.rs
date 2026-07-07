//! UI render entry + always-on chrome (header + status bar).
//!
//! Each mode-specific surface lives in its own submodule:
//!   - chat.rs      — message history + input box
//!   - popups.rs    — model picker, session picker, add-model, confirm
//!   - markdown.rs  — inline markdown parser + word-wrap (shared)
//!   - layout.rs    — 3-pane region builder (sidebar | chat | inspector)
//!   - inspector.rs — right-column agent dashboard (tool / shell / plan / stats)

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::{App, Mode};

mod chat;
mod inspector;
mod layout;
mod markdown;
mod popups;
mod sidebar;
pub(crate) mod theme;
mod viewer;
mod wrap_cache;

pub(crate) use sidebar::initial_expanded;

pub fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // **Critical frame-level wipe.** `Clear` resets every cell's
    // character AND style (`cell.reset()` → symbol=" ", style=default).
    // Without this, stale characters from the PREVIOUS frame persist
    // in the buffer — ratatui's `Block::set_style` only patches style
    // (bg/fg/modifier) but does NOT clear `cell.symbol`. So overflow
    // text from a prior render (e.g. expanded thinking that was
    // collapsed, or a chat line that scrolled away) would remain
    // visible as ghost characters on the new frame's bg.
    //
    // The old code used `Paragraph::new("")` which is a NO-OP — an
    // empty paragraph has zero lines and paints nothing. This was
    // the root cause of the "spam Ctrl+T while generating" leak:
    // stale characters were never wiped between frames.
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme::color::BG_BASE)),
        area,
    );

    let regions = layout::build_regions(f, app);

    render_header(f, regions.header, app);

    // Chat renders FIRST so that its potential overflow (long lines
    // that bleed past `chat_area`'s edges) lands in the sidebar /
    // inspector buffer cells BEFORE those panels render. The sidebar
    // and inspector then `Clear` their own areas (wiping the overflow)
    // before painting their content. If the chat rendered last, its
    // overflow would land on top of already-rendered sidebar /
    // inspector content with nothing to wipe it.
    if app.open_file.is_some() {
        viewer::render_viewer(f, regions.chat, app);
    } else {
        chat::render_chat(f, regions.chat, app);
    }

    if let Some(sb) = regions.sidebar {
        sidebar::render_sidebar(f, sb, app);
    } else {
        // Sidebar is hidden — wipe stale geometry from a previous wider
        // frame so a resize doesn't keep catching clicks on a column
        // that no longer exists.
        app.render.sidebar_x = 0;
        app.render.sidebar_y = 0;
        app.render.sidebar_w = 0;
        app.render.sidebar_h = 0;
        app.render.sidebar_targets.clear();
        app.render.sidebar_tab_rects.clear();
    }

    // Modal popups split the chat column 50/50 — chat shrinks to the
    // top half, popup occupies the bottom half. The inspector stays
    // full-height so the user still sees the agent state during a
    // picker. **ModelPicker is back to modal** — rendered into the
    // bottom half of the chat column (its `popup_area` slice), not
    // floating. Confirm is non-modal (inline card in chat). Inline
    // autocomplete is NOT modal (anchored above the input).
    let popup_active = matches!(
        app.mode,
        Mode::ModelPicker
            | Mode::AddModel
            | Mode::SessionPicker
            | Mode::DisconnectPicker
            | Mode::TelegramSetup
            | Mode::AgentsSetup
            | Mode::ShellMonitor
            | Mode::McpSetup
    );
    let (chat_area, popup_area) = if popup_active {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(regions.chat);
        (split[0], Some(split[1]))
    } else {
        (regions.chat, None)
    };

    // Re-render the chat into the (possibly split) chat_area so that
    // modal popups get the top half. The first chat render (above)
    // already painted into the full `regions.chat`; this second render
    // overwrites the top-half portion cleanly because `render_chat`
    // starts with its own `Clear`.
    if popup_active {
        if app.open_file.is_some() {
            viewer::render_viewer(f, chat_area, app);
        } else {
            chat::render_chat(f, chat_area, app);
        }
    }

    inspector::render_inspector(f, &regions, app);
    chat::render_input(f, regions.input, app);
    render_status(f, regions.status, app);

    if app.inline_popup.is_open() {
        popups::render_inline_popup(f, regions.input, app);
    }

    if let Some(p) = popup_area {
        match app.mode {
            Mode::ModelPicker => popups::render_picker(f, p, app),
            Mode::AddModel => popups::render_add_model(f, p, app),
            Mode::SessionPicker => popups::render_session_picker(f, p, app),
            Mode::DisconnectPicker => popups::render_disconnect_picker(f, p, app),
            Mode::TelegramSetup => popups::render_telegram_setup(f, p, app),
            Mode::AgentsSetup => popups::render_agents_setup(f, p, app),
            Mode::ShellMonitor => popups::render_shell_monitor(f, p, app),
            Mode::McpSetup => popups::render_mcp_setup(f, p, app),
            // Confirm renders inline in the chat (see render_chat).
            Mode::Confirm | Mode::Chat => {}
        }
    }
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    // Header is now empty — the product badge, model, host, and
    // token counts all live in the inspector pane's top section
    // (see `ui::inspector::ModelSection`). The header row is kept so
    // the vertical layout doesn't shift, but it's a no-op render.
    // If we ever need to put chrome back here, the update-notification
    // banner is the natural home for it.
    let _ = app;
    f.render_widget(
        Paragraph::new("").style(Style::default().bg(theme::color::BG_BASE)),
        area,
    );
}

fn render_status(f: &mut Frame, area: Rect, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    // Left side: the regular status text, with the shell-monitor
    // indicator appended when a `run_command` is in flight. The
    // indicator is mouse-clickable — record its column range into
    // `app.render` so the mouse handler can route the click.
    let mut left_spans = vec![
        Span::styled("▎ ", Style::default().fg(theme::color::ACCENT_ALT)),
        Span::styled(app.status.as_str(), Style::default().fg(theme::color::FG)),
    ];
    // Default to "no indicator this frame" so a stale rect from a prior
    // frame can't keep catching clicks after the shell exits.
    app.render.shell_indicator_x = 0;
    app.render.shell_indicator_y = 0;
    app.render.shell_indicator_w = 0;
    if let Some(rt) = app.active_shell.as_ref() {
        if rt.running {
            let dot = "●";
            // Pulse the dot via the same anim_tick the chat renderer
            // uses for in-flight tools — visually links "tool running"
            // (chat-side breath) and "shell running" (footer-side
            // breath) as the same kind of "something is happening".
            // Sine-interp between a dim and a saturated peach (the
            // same palette as `tool_breath` in chat/helpers.rs).
            let dot_color = {
                let period = 30u64;
                let phase = (app.anim_tick % period) as f32 / period as f32 * std::f32::consts::TAU;
                let t = (phase.sin() * 0.5) + 0.5;
                let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
                Color::Rgb(lerp(115, 250), lerp(80, 179), lerp(60, 135))
            };
            let label = "  1 shell running ";
            left_spans.push(Span::styled("   ", Style::default()));
            left_spans.push(Span::styled(
                dot.to_string(),
                Style::default().fg(dot_color),
            ));
            left_spans.push(Span::styled(
                label.to_string(),
                Style::default()
                    .fg(theme::color::FG)
                    .add_modifier(Modifier::BOLD),
            ));
            // Position the indicator hit-test. Span widths in cells:
            //   "▎ " = 2, status text = app.status width,
            //   "   " (gap) = 3, "●" = 1, label width follows.
            // The whole `● 1 shell running ` block is clickable —
            // record its left edge + total width so the mouse handler
            // can detect taps anywhere in that range.
            let prefix_w = 2 + app.status.chars().count() as u16 + 3;
            let indicator_w = 1 + label.chars().count() as u16;
            // chunks[0] is the left half of the status bar. The
            // indicator sits inside it at column offset `prefix_w`.
            app.render.shell_indicator_x = chunks[0].x.saturating_add(prefix_w);
            app.render.shell_indicator_y = chunks[0].y;
            app.render.shell_indicator_w = indicator_w;
        }
    }
    f.render_widget(
        Paragraph::new(Line::from(left_spans)).style(Style::default().bg(theme::color::BG_BASE)),
        chunks[0],
    );
    let help = Line::from(vec![
        Span::styled("/help", Style::default().fg(theme::color::FG)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("alt+enter", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" newline", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("drag", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" copy", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("wheel", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" scroll", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("  ·  ", Style::default().fg(theme::color::FG_DIMMER)),
        Span::styled("^T", Style::default().fg(theme::color::FG_DIM)),
        Span::styled(" fold ", Style::default().fg(theme::color::FG_DIMMER)),
    ]);
    f.render_widget(
        Paragraph::new(help)
            .alignment(Alignment::Right)
            .style(Style::default().bg(theme::color::BG_BASE)),
        chunks[1],
    );
}

// `mask_host` and `format_tokens` used to live here for the header.
// They moved to `ui::inspector::short_host` / `format_tokens` when the
// model/host/token block moved to the right panel.
