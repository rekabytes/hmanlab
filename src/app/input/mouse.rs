//! Mouse event handling: scroll routing, sidebar clicks, tool-row
//! toggling, and drag-to-select with OSC-52 clipboard copy.
//!
//! `handle_mouse` is the single entry point — `event.rs::handle_event`
//! calls it for every `Event::Mouse`. Modal popups short-circuit at the
//! top: Confirm mode steals the wheel for diff scrolling; every other
//! non-chat mode is a no-op so clicks/scrolls don't leak into hidden
//! state behind a picker.

use std::io::Write;

use base64::Engine;
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use tokio::sync::mpsc;

use super::super::{App, Mode, StreamMsg};

impl App {
    pub(in crate::app) fn handle_mouse(
        &mut self,
        m: MouseEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        // Confirm mode runs its own minimal mouse handler — wheel only,
        // scrolls the diff body the same way the keyboard arrows do.
        // The rest of the chat-mouse logic (selection, sidebar clicks,
        // tool toggle) would be meaningless behind a modal popup, so
        // we early-return here without going through the chat path.
        if self.mode == Mode::Confirm {
            match m.kind {
                MouseEventKind::ScrollUp => {
                    self.confirm_scroll = self.confirm_scroll.saturating_sub(3);
                }
                MouseEventKind::ScrollDown => {
                    self.confirm_scroll = self.confirm_scroll.saturating_add(3);
                }
                _ => {}
            }
            return;
        }
        if self.mode != Mode::Chat {
            return;
        }
        // Status-bar shell indicator. Bounded rect is populated by
        // `render_status` only while a shell is in flight, so a click
        // here implies the indicator is currently visible. Open the
        // monitor and short-circuit before the chat-area hit-tests so
        // the click doesn't also try to toggle a tool tile.
        if let MouseEventKind::Down(MouseButton::Left) = m.kind {
            let in_indicator = self.render.shell_indicator_w > 0
                && m.row == self.render.shell_indicator_y
                && m.column >= self.render.shell_indicator_x
                && m.column
                    < self
                        .render
                        .shell_indicator_x
                        .saturating_add(self.render.shell_indicator_w);
            if in_indicator {
                self.mode = Mode::ShellMonitor;
                // Cancel any in-progress selection state so this click
                // doesn't leave the chat-area drag tracker armed.
                self.sel_start = None;
                self.sel_end = None;
                self.selecting = false;
                return;
            }
        }
        // Track cursor position on every mouse event so the hover overlay
        // can highlight the row under the pointer — including mid-drag,
        // mid-scroll, anything. Cheaper than a Moved-only branch and never
        // gets stale.
        self.render.hover_x = m.column;
        self.render.hover_y = m.row;
        match m.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.sel_start = Some((m.column, m.row));
                self.sel_end = Some((m.column, m.row));
                self.selecting = true;
            }
            MouseEventKind::Drag(MouseButton::Left) if self.selecting => {
                self.sel_end = Some((m.column, m.row));
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if !self.selecting {
                    return;
                }
                self.selecting = false;
                // No drag → treat as a click. Sidebar clicks open files;
                // chat clicks toggle the tool block under the cursor.
                if self.sel_start == self.sel_end {
                    if let Some((col, row)) = self.sel_start {
                        if self.point_in_sidebar(col, row) {
                            self.try_open_sidebar_at(row, tx);
                        } else {
                            let in_chat_col = col >= self.render.chat_x
                                && col < self.render.chat_x.saturating_add(self.render.chat_w);
                            if in_chat_col && self.open_file.is_none() {
                                self.try_toggle_tool_at(row);
                            }
                        }
                    }
                    self.sel_start = None;
                    self.sel_end = None;
                } else {
                    self.copy_selection_to_clipboard();
                }
            }
            MouseEventKind::ScrollUp => {
                // Route wheel events to whichever panel the cursor is over.
                if self.point_in_sidebar(m.column, m.row) {
                    self.sidebar_scroll = self.sidebar_scroll.saturating_sub(3);
                } else if let Some(file) = self.open_file.as_mut() {
                    file.scroll = file.scroll.saturating_sub(3);
                } else {
                    self.follow = false;
                    self.scroll = self.scroll.saturating_sub(3);
                }
            }
            MouseEventKind::ScrollDown => {
                if self.point_in_sidebar(m.column, m.row) {
                    // The renderer clamps to a valid max each frame, so a
                    // saturating add here is enough — wheel-past-end stops
                    // cleanly without us computing max_scroll up-front.
                    self.sidebar_scroll = self.sidebar_scroll.saturating_add(3);
                } else if let Some(file) = self.open_file.as_mut() {
                    file.scroll = file.scroll.saturating_add(3);
                } else {
                    let prev = self.scroll;
                    self.scroll = self.scroll.saturating_add(3);
                    if self.scroll == prev {
                        self.follow = true;
                    }
                }
            }
            _ => {}
        }
    }

    fn point_in_sidebar(&self, col: u16, row: u16) -> bool {
        self.render.sidebar_w != 0
            && col >= self.render.sidebar_x
            && col < self.render.sidebar_x.saturating_add(self.render.sidebar_w)
            && row >= self.render.sidebar_y
            && row < self.render.sidebar_y.saturating_add(self.render.sidebar_h)
    }

    /// Resolve a sidebar click: directories toggle expanded state, files
    /// open in the viewer. `screen_y` is the raw row from the mouse event;
    /// we convert it to a logical line via `(screen_y - sidebar_y) +
    /// sidebar_scroll` before looking up the entry, so scrolled-out rows
    /// resolve correctly once they're brought back into view.
    fn try_open_sidebar_at(&mut self, screen_y: u16, tx: &mpsc::UnboundedSender<StreamMsg>) {
        if screen_y < self.render.sidebar_y {
            return;
        }
        let logical_y = (screen_y - self.render.sidebar_y).saturating_add(self.sidebar_scroll);
        let hit = self
            .render
            .sidebar_targets
            .iter()
            .find(|(row, _, _)| *row == logical_y)
            .map(|(_, path, is_dir)| (path.clone(), *is_dir));
        match hit {
            // Toggle expand/collapse. The next render uses the updated
            // set; the walker re-builds the tree from scratch each frame.
            // Collapsing the inner `if` into a match guard would push the
            // side-effecting `remove` into the guard expression AND force
            // an extra catch-all arm to keep exhaustiveness, so allow the
            // lint here.
            #[allow(clippy::collapsible_match)]
            Some((path, true)) => {
                if !self.expanded_dirs.remove(&path) {
                    self.expanded_dirs.insert(path);
                }
            }
            Some((path, false)) => {
                self.open_workspace_file(path, tx);
            }
            None => {}
        }
    }

    fn try_toggle_tool_at(&mut self, screen_y: u16) {
        let top = self.render.chat_y;
        let bottom = top.saturating_add(self.render.chat_h);
        if screen_y < top || screen_y >= bottom {
            return;
        }
        let logical_y = self.scroll.saturating_add(screen_y - top);
        for &(idx, ls, le) in &self.render.message_line_ranges {
            if logical_y >= ls && logical_y < le {
                if let Some(msg) = self.messages.get(idx) {
                    if msg.role == "tool" && !self.expanded_tools.remove(&idx) {
                        self.expanded_tools.insert(idx);
                    }
                }
                return;
            }
        }
    }

    fn copy_selection_to_clipboard(&mut self) {
        let (Some(start), Some(end)) = (self.sel_start, self.sel_end) else {
            return;
        };
        // Normalize so start is the top-left, end is the bottom-right.
        let ((sx, sy), (ex, ey)) = if (start.1, start.0) <= (end.1, end.0) {
            (start, end)
        } else {
            (end, start)
        };
        let cx = self.render.chat_x;
        let cy = self.render.chat_y;
        let cw = self.render.chat_w;
        let ch = self.render.chat_h;
        if cw == 0 || ch == 0 {
            self.sel_start = None;
            self.sel_end = None;
            return;
        }
        let row_lo = sy.max(cy);
        let row_hi = ey.min(cy.saturating_add(ch).saturating_sub(1));
        if row_lo > row_hi {
            self.sel_start = None;
            self.sel_end = None;
            return;
        }

        let mut lines_out: Vec<String> = Vec::new();
        for screen_y in row_lo..=row_hi {
            let logical_idx = (self.scroll as usize) + ((screen_y - cy) as usize);
            let line = self
                .render
                .rendered_text_lines
                .get(logical_idx)
                .map(|s| s.as_str())
                .unwrap_or("");
            let row_start_col = if screen_y == sy {
                (sx.saturating_sub(cx)) as usize
            } else {
                0
            };
            let row_end_col = if screen_y == ey {
                (ex.saturating_sub(cx)) as usize + 1
            } else {
                usize::MAX
            };
            let chars: Vec<char> = line.chars().collect();
            let s = row_start_col.min(chars.len());
            let e = row_end_col.min(chars.len());
            let piece: String = if s < e {
                chars[s..e].iter().collect()
            } else {
                String::new()
            };
            lines_out.push(piece.trim_end().to_string());
        }
        while lines_out.last().is_some_and(|l| l.is_empty()) {
            lines_out.pop();
        }
        let buf: String = lines_out.join("\n");
        self.sel_start = None;
        self.sel_end = None;
        if buf.is_empty() {
            return;
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(buf.as_bytes());
        let mut out = std::io::stdout();
        let _ = write!(out, "\x1b]52;c;{}\x07", encoded);
        let _ = out.flush();
        self.status = format!("Copied {} chars to clipboard", buf.len());
    }
}
