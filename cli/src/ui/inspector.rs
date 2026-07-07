//! Right-column inspector pane — the "agent dashboard" of the 3-pane UI.
//!
//! Sections (top → bottom):
//!   1. Model — model name, host, running token total, state badge.
//!      Used to live in the top header; moved here so the entire
//!      "what is the agent doing right now + who am I talking to + how
//!      much have I spent" view lives in one column.
//!   2. Active tool — name of the in-flight tool call + elapsed time.
//!      Hidden when idle.
//!   3. Shell — if a shell command is running, the command + elapsed
//!      time. Hidden when no shell is active.
//!   4. Pending — pending permission count + the prompt head, so the
//!      user knows there's a card waiting in the chat even if the chat
//!      is scrolled away from it.
//!   5. Plan — current todo checklist. Empty state hides the section.
//!   6. Session — running prompt/completion token totals. Mirrors the
//!      model section's token line; kept here for "what is my spend"
//!      glanceability when the model section is scrolled off.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, TurnState};

use super::layout::Regions;
use super::theme::color;
use ratatui::style::Color;

/// One section of the inspector. Each section knows its own height and
/// how to draw itself — the dispatcher in `render_inspector` only
/// allocates a `Rect` and calls `draw`.
trait Section {
    /// Preferred height in rows. The dispatcher uses this to split the
    /// column top-to-bottom.
    fn height(&self, avail_w: u16) -> u16;
    /// Draw the section into `area`.
    fn draw(&self, f: &mut Frame, area: Rect);
}

/// Render the inspector into the right column. No-op if the column
/// doesn't exist (terminal too narrow — see `layout::INSPECTOR_MIN_TOTAL_W`).
pub fn render_inspector(f: &mut Frame, regions: &Regions, app: &mut App) {
    let Some(area) = regions.inspector else {
        return;
    };

    // **Critical**: `Clear` the full inspector area before any other
    // paint. Without this, chat content that overflows past
    // `chat_area`'s right edge stays visible in the inspector.
    //
    // Why: ratatui's `Block::render` calls `buf.set_style(area, style)`
    // which PATCHES the style (bg/fg/modifier) but does NOT touch
    // `cell.symbol` — the character. So if the chat Paragraph wrote
    // characters past its area into the inspector's buffer cells,
    // the inspector's Block sets BG_BASE on those cells but the chat's
    // TEXT remains. `Clear` calls `cell.reset()` which sets
    // `symbol = " "` AND resets style, wiping the leak. The Block then
    // re-paints BG_BASE on top.
    //
    // This is the root cause of the "spam Ctrl+T while generating"
    // leak: the chat Paragraph doesn't hard-clip past its area
    // (ratatui writes to absolute buffer positions via `set_string`),
    // so any wide line — a single long word, or a line the wrap cache
    // wrapped at a stale width — bleeds into the adjacent column.
    // Clearing here is the defence.
    f.render_widget(ratatui::widgets::Clear, area);

    // No block title — the brand badge and state badge live in
    // `ModelSection` (top of the section list) so the entire
    // "hmanlab · ● IDLE · model · host · tokens" view sits in one
    // cohesive block at the top of the panel. Rendering a duplicate
    // state badge as the block title here would show it twice.
    let block = Block::default()
        .borders(Borders::NONE)
        .style(Style::default().bg(color::BG_BASE))
        .padding(Padding::horizontal(1));

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Build the section list — only the ones that have something to say.
    // Model section is always first so the user sees "who am I talking
    // to + how much have I spent" before the rest of the dashboard.
    let mut sections: Vec<Box<dyn Section>> = Vec::new();
    sections.push(Box::new(ModelSection::from_app(app)));
    if let Some(s) = ActiveToolSection::from_app(app) {
        sections.push(Box::new(s));
    }
    if let Some(s) = ShellSection::from_app(app) {
        sections.push(Box::new(s));
    }
    if let Some(s) = PendingSection::from_app(app) {
        sections.push(Box::new(s));
    }
    // Plan section removed — the inspector now focuses on
    // model/host/tokens (top) and session stats (bottom). The plan
    // used to live between them; with the recent move of
    // model/host/tokens up to the brand row, the plan felt
    // redundant against the chat column where the same checklist
    // already renders as part of the assistant message.
    sections.push(Box::new(SessionSection::from_app(app)));

    // Vertical split. One leading spacer for top-edge breathing room,
    // then a 2-row spacer between Model and Session so the session
    // block sits visibly lower than the connection-info block — the
    // eye reads "who am I talking to" first, then has to scan down
    // to reach "how much have I spent / how long is this session".
    //
    // `section_chunk_indices` tracks which chunk indices correspond
    // to actual sections (vs spacer rows) so the draw loop pairs each
    // section with its real chunk — see the prior zip-misalignment
    // bug where Model ended up drawing into the top spacer's chunk.
    let mut constraints: Vec<Constraint> = Vec::with_capacity(sections.len() + 2);
    let mut section_chunk_indices: Vec<usize> = Vec::with_capacity(sections.len());
    let mut next_idx = 0;
    for (i, s) in sections.iter().enumerate() {
        let h = s.height(inner.width);
        constraints.push(if h == 0 {
            Constraint::Length(0)
        } else {
            Constraint::Length(h)
        });
        section_chunk_indices.push(next_idx);
        next_idx += 1;
        // Mid-spacer between Model (i=0) and Session (i=1) — pushes
        // the session block visibly lower than the connection-info
        // block, with no horizontal divider. `next_idx += 1` here
        // (NOT 2!) because `Length(2)` is one chunk of height 2 — not
        // two chunks. Incrementing by 2 caused the previous panic
        // where Session tried to draw into chunks[4] of a 4-chunk
        // vec.
        if i == 0 && sections.len() > 1 {
            constraints.push(Constraint::Length(2));
            next_idx += 1;
        }
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (s, &chunk_idx) in sections.iter().zip(section_chunk_indices.iter()) {
        let chunk = chunks[chunk_idx];
        if chunk.height > 0 {
            s.draw(f, chunk);
        }
    }
}

/// Compute the badge text based on current turn + shell state.
fn state_badge(app: &App) -> &'static str {
    if app.active_shell.as_ref().is_some_and(|s| s.running) {
        "● SHELL"
    } else if app.active_tool_msg_idx.is_some() {
        "● TOOL"
    } else if matches!(app.turn, TurnState::Generating { .. }) {
        "● THINKING"
    } else if matches!(app.turn, TurnState::Compacting { .. }) {
        "● COMPACT"
    } else {
        "● IDLE"
    }
}

/// Map a badge string ("● THINKING", "● IDLE", etc.) to its semantic
/// color. Used by `ModelSection`'s title row so the badge stays in
/// sync with the agent's actual state. `badge_color` (the App-driven
/// version) was used by the now-removed inspector block title.
fn badge_color_from_str(badge: &str) -> Color {
    if badge.contains("SHELL") || badge.contains("TOOL") {
        color::STATE_TOOLING
    } else if badge.contains("THINKING") {
        color::STATE_THINKING
    } else if badge.contains("COMPACT") {
        color::STATE_COMPACTING
    } else {
        color::STATE_IDLE
    }
}

/// Bottom-bordered section block with a small uppercase title.
fn section_block(title: &str, accent: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(color::FG_DIMMER))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .padding(Padding::horizontal(0))
}

// ── Model / host / tokens ─────────────────────────────────────────
// Always shown — pinned to the top of the inspector. Used to live in
// the top header bar; moved here so the entire "who am I talking to +
// how much have I spent" view lives in one column. The state badge
// (● THINKING / ● IDLE / etc.) is the title row of this section so
// the panel's whole identity comes from the first thing the eye sees.
struct ModelSection {
    model: String,
    host: String,
    tokens: String,
    badge: String,
}
impl ModelSection {
    fn from_app(app: &App) -> Self {
        let total = app.total_prompt_tokens + app.total_completion_tokens;
        Self {
            model: app.model.clone(),
            host: short_host(app.current_host()),
            tokens: format_tokens(total),
            badge: state_badge(app).to_string(),
        }
    }
}
impl Section for ModelSection {
    fn height(&self, _avail_w: u16) -> u16 {
        // 3 rows: brand+state badge / model / host. Host sits below
        // model (not beside it) so long hostnames don't push the
        // model name off the right edge on narrow inspectors.
        3
    }
    fn draw(&self, f: &mut Frame, area: Rect) {
        let label_style = Style::default().fg(color::FG_DIM);
        let val_style = Style::default().fg(color::FG).add_modifier(Modifier::BOLD);
        let host_style = Style::default().fg(color::FG);

        let lines = vec![
            // Row 1: brand badge + state badge. Brand is the peach
            // pill (same as the legacy top header). State badge
            // echoes the agent's current activity so the eye lands
            // on "what is it doing" first.
            Line::from(vec![
                Span::styled(
                    " Hibiscus ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(color::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    self.badge.clone(),
                    Style::default()
                        .fg(badge_color_from_str(&self.badge))
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            // Row 2: model — own line so the name has full width.
            Line::from(vec![
                Span::styled("model ", label_style),
                Span::styled(
                    truncate(&self.model, area.width.saturating_sub(6) as usize),
                    val_style,
                ),
            ]),
            // Row 3: host — below model, also full width.
            Line::from(vec![
                Span::styled("host ", label_style),
                Span::styled(
                    truncate(&self.host, area.width.saturating_sub(5) as usize),
                    host_style,
                ),
            ]),
        ];
        let p = Paragraph::new(lines)
            .style(Style::default().bg(color::BG_BASE))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Active tool ─────────────────────────────────────────────────────
struct ActiveToolSection {
    name: String,
    elapsed: String,
}
impl ActiveToolSection {
    fn from_app(app: &App) -> Option<Self> {
        let idx = app.active_tool_msg_idx?;
        let msg = app.messages.get(idx)?;
        let name = msg.name.as_deref()?.to_string();
        let elapsed = app
            .active_tool_started_at
            .map(format_elapsed)
            .unwrap_or_else(|| "0.0s".to_string());
        Some(Self { name, elapsed })
    }
}
impl Section for ActiveToolSection {
    fn height(&self, _avail_w: u16) -> u16 {
        3
    }
    fn draw(&self, f: &mut Frame, area: Rect) {
        let line = Line::from(vec![
            Span::styled("⚡ ", Style::default().fg(color::STATE_TOOLING)),
            Span::styled(
                self.name.clone(),
                Style::default().fg(color::FG).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  · {}", self.elapsed),
                Style::default().fg(color::FG_DIM),
            ),
        ]);
        let p = Paragraph::new(line)
            .block(section_block("active", color::STATE_TOOLING))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Shell ────────────────────────────────────────────────────────────
struct ShellSection {
    command: String,
    status: Line<'static>,
    elapsed_s: String,
}
impl ShellSection {
    fn from_app(app: &App) -> Option<Self> {
        let rt = app.active_shell.as_ref()?;
        let elapsed_s = format_elapsed_instant(rt.started_at.elapsed());
        let status: Line<'static> = if rt.running {
            Line::from(Span::styled(
                "running",
                Style::default().fg(color::STATE_TOOLING),
            ))
        } else {
            match rt.exit_code {
                Some(Some(0)) => {
                    Line::from(Span::styled("done", Style::default().fg(color::SUCCESS)))
                }
                Some(Some(c)) => Line::from(Span::styled(
                    format!("exit {c}"),
                    Style::default().fg(color::ERROR),
                )),
                Some(None) => {
                    Line::from(Span::styled("killed", Style::default().fg(color::WARNING)))
                }
                None => Line::from(Span::styled("…", Style::default().fg(color::FG_DIM))),
            }
        };
        Some(Self {
            command: truncate(&rt.command, 24),
            status,
            elapsed_s,
        })
    }
}
impl Section for ShellSection {
    fn height(&self, _avail_w: u16) -> u16 {
        3
    }
    fn draw(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![
            Span::styled("$ ", Style::default().fg(color::ACCENT_ALT)),
            Span::styled(self.command.clone(), Style::default().fg(color::FG)),
        ];
        spans.push(Span::raw(" "));
        // `Line` (ratatui 0.29) exposes `spans: Vec<Span>` publicly —
        // splice them into the parent line.
        spans.extend(self.status.spans.iter().cloned());
        spans.push(Span::styled(
            format!(" · {}", self.elapsed_s),
            Style::default().fg(color::FG_DIM),
        ));
        let p = Paragraph::new(Line::from(spans))
            .block(section_block("shell", color::ACCENT_ALT))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Pending permission ──────────────────────────────────────────────
struct PendingSection {
    head: String,
}
impl PendingSection {
    fn from_app(app: &App) -> Option<Self> {
        let req = app.pending_confirm.as_ref()?;
        Some(Self {
            head: first_word(&req.prompt),
        })
    }
}
impl Section for PendingSection {
    fn height(&self, _avail_w: u16) -> u16 {
        3
    }
    fn draw(&self, f: &mut Frame, area: Rect) {
        let line = Line::from(vec![
            Span::styled("🔐 ", Style::default().fg(color::PERMISSION_INFO)),
            Span::styled(
                format!("{}…", self.head),
                Style::default()
                    .fg(color::PERMISSION_INFO)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  awaiting you", Style::default().fg(color::FG_DIM)),
        ]);
        let p = Paragraph::new(line)
            .block(section_block("pending", color::PERMISSION_INFO))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

// ── Session stats ────────────────────────────────────────────────────
struct SessionSection {
    /// Short session id (first 8 chars of the UUID) so the user
    /// can tell at a glance which conversation this is. Falls back
    /// to "untitled" before the first user message creates a session.
    session_label: String,
    total_tokens: u64,
    msg_count: usize,
}
impl SessionSection {
    fn from_app(app: &App) -> Self {
        let session_label = match app.local_session_id.as_deref() {
            Some(sid) if sid.len() >= 8 => sid[..8].to_string(),
            Some(sid) => sid.to_string(),
            None => "untitled".to_string(),
        };
        Self {
            session_label,
            total_tokens: app.total_prompt_tokens + app.total_completion_tokens,
            msg_count: app.messages.len(),
        }
    }
}
impl Section for SessionSection {
    fn height(&self, _avail_w: u16) -> u16 {
        // 3 rows: sessions label + token + message count. Pushed
        // visually lower than the model section by the mid-spacer
        // the dispatcher inserts between sections.
        3
    }
    fn draw(&self, f: &mut Frame, area: Rect) {
        let label_style = Style::default().fg(color::FG_DIM);
        let val_style = Style::default().fg(color::FG);
        let val_bold = Style::default().fg(color::FG).add_modifier(Modifier::BOLD);

        let lines = vec![
            // Row 1: sessions - {id}
            Line::from(vec![
                Span::styled("sessions - ", label_style),
                Span::styled(self.session_label.clone(), val_bold),
            ]),
            // Row 2: token {count}
            Line::from(vec![
                Span::styled("token    ", label_style),
                Span::styled(format_tokens(self.total_tokens), val_style),
            ]),
            // Row 3: message {count}
            Line::from(vec![
                Span::styled("message  ", label_style),
                Span::styled(self.msg_count.to_string(), val_style),
            ]),
        ];
        let p = Paragraph::new(lines)
            .style(Style::default().bg(color::BG_BASE))
            .wrap(Wrap { trim: false });
        f.render_widget(p, area);
    }
}

fn format_elapsed(t0: std::time::Instant) -> String {
    format_elapsed_instant(t0.elapsed())
}

fn format_elapsed_instant(d: std::time::Duration) -> String {
    let s = d.as_secs_f32();
    if s < 60.0 {
        format!("{:.1}s", s)
    } else if s < 3600.0 {
        format!("{}m{:02}s", (s as u32) / 60, (s as u32) % 60)
    } else {
        format!("{}h{:02}m", (s as u32) / 3600, ((s as u32) % 3600) / 60)
    }
}

fn format_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// Strip scheme and port from a base URL — `http://192.168.3.3:11434`
/// becomes `192.168.3.3`. Mirrors `mask_host` in `ui/mod.rs` so the
/// inspector shows the same compact host the legacy header did.
fn short_host(base: &str) -> String {
    let no_scheme = base
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let no_path = no_scheme.split('/').next().unwrap_or(no_scheme);
    if no_path.starts_with('[') {
        if let Some(close) = no_path.find(']') {
            return no_path[..=close].to_string();
        }
    }
    match no_path.rfind(':') {
        Some(i) => no_path[..i].to_string(),
        None => no_path.to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn first_word(s: &str) -> String {
    s.split_whitespace().next().unwrap_or("").to_string()
}
