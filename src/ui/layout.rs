//! Layout constants + region builder for the 3-pane agentic TUI.
//!
//! The terminal area is sliced vertically into Header / Body / Status.
//! The Body row is then sliced horizontally into Sidebar / Chat /
//! Inspector. Widths are responsive: at narrow widths the inspector is
//! collapsed (the chat keeps the space) and the sidebar is dropped at
//! extreme narrowness (so the input box stays usable).
//!
//! All render code that needs a region pulls it from here so the layout
//! stays in one place — no renderer hand-rolls its own `Rect`.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

use crate::app::App;

/// Width of the left sidebar (incl. its border). Matches the legacy
/// sidebar width so files-fans don't see a shift during the redesign.
pub const SIDEBAR_W: u16 = 26;

/// Width of the right inspector pane (incl. border). Wide enough to fit
/// a tool name + elapsed time on one line, plus a 3-line todo row.
pub const INSPECTOR_W: u16 = 32;

/// Below this total width the inspector is hidden — chat needs the
/// space. The header tokens, model name, and host fit comfortably in
/// the remaining ~50 cols.
pub const INSPECTOR_MIN_TOTAL_W: u16 = 110;

/// Below this total width even the sidebar is dropped (matches the
/// legacy threshold so resize behaviour stays predictable).
pub const SIDEBAR_MIN_TOTAL_W: u16 = 80;

/// One row for the top header line (model · host · tokens).
pub const HEADER_H: u16 = 1;

/// Bottom status / keybind hint row.
pub const STATUS_H: u16 = 1;

/// Input box height. Kept generous so multi-line drafts and pasted
/// snippets don't fight the chat column for vertical space.
pub const INPUT_H: u16 = 6;

/// Computed body regions for the current frame. Renderers consume
/// these directly — no renderer hand-rolls its own split.
pub struct Regions {
    pub header: Rect,
    pub input: Rect,
    pub status: Rect,
    pub sidebar: Option<Rect>,
    pub chat: Rect,
    pub inspector: Option<Rect>,
}

/// Build the 3-pane layout for the current frame. Decides whether to
/// show sidebar / inspector based on terminal width so the TUI stays
/// usable from 80-col terminals up to 4K monitors.
pub fn build_regions(f: &mut Frame, _app: &mut App) -> Regions {
    let area = f.area();

    // Vertical split first: header / body / input / status.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_H),
            Constraint::Min(3),
            Constraint::Length(INPUT_H),
            Constraint::Length(STATUS_H),
        ])
        .split(area);

    let total_w = rows[1].width;

    // Horizontal split of the body row. Inspector takes priority over
    // sidebar visibility — when both can't fit, hide the sidebar
    // before shrinking the inspector. Chat always gets the remainder.
    let show_inspector = total_w >= INSPECTOR_MIN_TOTAL_W;
    let show_sidebar = total_w >= SIDEBAR_MIN_TOTAL_W;

    let (sidebar, chat_and_inspector) = if show_sidebar {
        let pair = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(SIDEBAR_W), Constraint::Min(40)])
            .split(rows[1]);
        (Some(pair[0]), pair[1])
    } else {
        (None, rows[1])
    };

    let (chat, inspector) = if show_inspector {
        let pair = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(INSPECTOR_W)])
            .split(chat_and_inspector);
        (pair[0], Some(pair[1]))
    } else {
        (chat_and_inspector, None)
    };

    Regions {
        header: rows[0],
        input: rows[2],
        status: rows[3],
        sidebar,
        chat,
        inspector,
    }
}