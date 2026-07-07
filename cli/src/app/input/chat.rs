//! The chat-mode keymap — the default keyboard handler when no modal is
//! active. Owns:
//!
//!   - Ctrl shortcuts (Ctrl+Q quit, Ctrl+C cancel/quit, Ctrl+M picker,
//!     Ctrl+L clear, Ctrl+N new session, Ctrl+T toggle folds, Ctrl+J
//!     newline).
//!   - The plain-Enter-submits-vs-modifier-Enter-inserts-newline split.
//!   - Inline `/` slash and `@` file-mention autocomplete: open/close,
//!     prev/next selection, completion-on-Tab-or-Enter.
//!   - Y/N quick-reply intercept when the assistant just asked a yes/no
//!     question and the input is empty.
//!   - Soft-wrap: synthesizing a newline when typing pushes the current
//!     row past the input box's visible width.
//!   - `submit` — parses slash commands, dispatches via `handle_command`,
//!     or forwards the text to the model via `send_to_llm`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tui_textarea::Input;

use super::super::commands::parse_command;
use super::super::inline::{self, FilePopup, InlinePopup, SlashPopup, SLASH_COMMANDS};
use super::super::{App, AppAction, StreamMsg};

impl App {
    pub(in crate::app) fn handle_chat(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        // File-viewer overlay takes over input while open. Esc closes it,
        // scroll keys page through the file, everything else is swallowed
        // so the chat input stays inert until the user dismisses the file.
        if self.open_file.is_some() {
            return self.handle_viewer_key(key);
        }

        // Inline autocomplete popup (slash / @ mention) intercept — must
        // come before the global Esc, Up/Down scroll, and Enter-submit
        // handlers so the popup gets first dibs on its own navigation keys.
        if self.inline_popup.is_open() && key.modifiers.is_empty() {
            match key.code {
                KeyCode::Esc => {
                    self.inline_popup = InlinePopup::None;
                    return AppAction::Continue;
                }
                KeyCode::Up => {
                    self.inline_popup_select_prev();
                    return AppAction::Continue;
                }
                KeyCode::Down => {
                    self.inline_popup_select_next();
                    return AppAction::Continue;
                }
                KeyCode::Tab => {
                    // Tab still inserts the text into the input box (for
                    // cases where the user wants to keep editing — e.g.
                    // `/model ` then typing a model name). Single-press
                    // autocomplete behaviour.
                    self.inline_popup_complete();
                    return AppAction::Continue;
                }
                KeyCode::Enter => {
                    // Slash popup: run the command immediately on first
                    // press. Matches opencode / claude-code / most chat
                    // clients' behaviour where selecting from the
                    // autocomplete popup triggers the command without an
                    // extra Enter. The user no longer has to press Enter
                    // twice to open `/model` etc.
                    //
                    // File popup: keep the legacy "insert text" behaviour
                    // — `@` mentions insert the path into the input box
                    // (the user might want to keep typing around it).
                    if matches!(self.inline_popup, InlinePopup::Slash(_)) {
                        return self.inline_popup_run(tx);
                    }
                    self.inline_popup_complete();
                    return AppAction::Continue;
                }
                _ => {}
            }
        }

        // Y/N quick-reply intercept — only when the AI just asked a yes/no
        // question and the input is empty. Plain key, no modifiers.
        if self.yn_pending
            && !self.turn.is_generating()
            && key.modifiers.is_empty()
            && self.input.lines().iter().all(|l| l.is_empty())
        {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.yn_pending = false;
                    self.awaiting_yn_followup = true;
                    self.inject_hidden_user("yes", tx);
                    return AppAction::Continue;
                }
                KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.yn_pending = false;
                    self.inject_hidden_user("no, that's enough", tx);
                    return AppAction::Continue;
                }
                KeyCode::Esc => {
                    self.yn_pending = false;
                    return AppAction::Continue;
                }
                // Any other plain key dismisses the hint and falls through
                // to the normal input pipeline below.
                _ => {
                    self.yn_pending = false;
                }
            }
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') => return AppAction::Quit,
                KeyCode::Char('c') => {
                    if self.turn.is_generating() {
                        self.cancel();
                    } else {
                        return AppAction::Quit;
                    }
                    return AppAction::Continue;
                }
                KeyCode::Char('m') => {
                    self.open_picker();
                    return AppAction::Continue;
                }
                KeyCode::Char('l') => {
                    self.clear_history();
                    return AppAction::Continue;
                }
                KeyCode::Char('n') => {
                    self.new_session();
                    return AppAction::Continue;
                }
                // Ctrl+T — was fold-toggle. Removed when tool /
                // thinking fold UI was stripped from the chat
                // renderer. Re-bind to a redesign when we add a new
                // fold surface.
                // Ctrl+J — classic terminal newline (raw 0x0A). Always
                // works regardless of terminal Shift+Enter support, so we
                // bind it explicitly as a universal "insert newline"
                // shortcut. Also matches readline / emacs convention.
                KeyCode::Char('j') => {
                    self.input.insert_newline();
                    self.update_inline_popup();
                    return AppAction::Continue;
                }
                // Ctrl+V — try clipboard image first (queue as
                // attachment), fall back to text paste into the
                // textarea. Matches the chat-app convention of "one key
                // for paste, app figures out what kind".
                KeyCode::Char('v') => {
                    self.paste_from_clipboard();
                    self.update_inline_popup();
                    self.soft_wrap_input();
                    return AppAction::Continue;
                }
                _ => {}
            }
        }

        match key.code {
            // Esc never quits — too easy to hit mid-conversation. Priority
            // order: interrupt an in-flight generation, then clear a draft,
            // then no-op. Quit stays available via /quit, /exit, Ctrl+Q,
            // and Ctrl+C (when nothing's running).
            KeyCode::Esc => {
                if self.turn.is_generating() {
                    self.cancel();
                } else {
                    let has_text = self.input.lines().iter().any(|l| !l.is_empty());
                    if has_text {
                        self.reset_input();
                        self.inline_popup = InlinePopup::None;
                    }
                }
                return AppAction::Continue;
            }
            KeyCode::PageUp => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(5);
                return AppAction::Continue;
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(5);
                return AppAction::Continue;
            }
            // Wheel scroll arrives as Up/Down arrow keys (DEC 1007). Route to chat scroll
            // when the input is a single line so multi-line composition still uses
            // arrows for cursor movement.
            KeyCode::Up if self.input.lines().len() <= 1 => {
                self.follow = false;
                self.scroll = self.scroll.saturating_sub(3);
                return AppAction::Continue;
            }
            KeyCode::Down if self.input.lines().len() <= 1 => {
                let prev = self.scroll;
                self.scroll = self.scroll.saturating_add(3);
                if self.scroll == prev {
                    self.follow = true;
                }
                return AppAction::Continue;
            }
            KeyCode::Home => {
                self.follow = false;
                self.scroll = 0;
                return AppAction::Continue;
            }
            KeyCode::End => {
                self.follow = true;
                return AppAction::Continue;
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                // Plain Enter (no modifiers) submits. Any modifier
                // combination — Shift, Alt, Ctrl — falls through to the
                // newline branch below. We do it this way (instead of
                // checking only Shift) because most terminals collapse
                // Shift+Enter to plain Enter, so without explicit Alt /
                // Ctrl backups there's no way to insert a newline at all.
                if !self.turn.is_generating() {
                    return self.submit(tx);
                }
                return AppAction::Continue;
            }
            KeyCode::Enter => {
                // Modifier+Enter → insert a newline. tui-textarea's
                // default keymap also does this for plain Enter, but
                // we've already stolen plain Enter for "submit", so call
                // insert_newline() directly here.
                self.input.insert_newline();
                self.update_inline_popup();
                return AppAction::Continue;
            }
            _ => {}
        }

        let input: Input = key.into();
        self.input.input(input);
        // After the textarea consumed the key, re-evaluate whether a `/`
        // slash command or `@` file mention is being typed and surface /
        // dismiss the popup accordingly.
        self.update_inline_popup();
        // Soft-wrap: if the typed character pushed the current line past
        // the input box's visible width, break it at the last whitespace
        // (or hard-wrap if none). Keeps the cursor in view.
        self.soft_wrap_input();
        AppAction::Continue
    }

    fn toggle_all_tools(&mut self) {
        // (Removed when tool / thinking fold UI was stripped from the
        // chat renderer. Re-introduce a single bulk toggle when we
        // redesign the fold surface.)
    }

    // toggle_all_tools / toggle_all_thoughts removed when the tool /
    // thinking fold UI was stripped from the chat renderer. Re-introduce
    // a single bulk toggle when we redesign the fold surface.

    /// Insert a synthetic newline into the current input line when typing
    /// has pushed the cursor row past `input_inner_w`. Tries a word
    /// boundary first (last whitespace at-or-before the limit), falls
    /// back to a hard break at the column limit. Skips when the input
    /// box is too narrow for wrap to make sense (<10 cols).
    fn soft_wrap_input(&mut self) {
        let max_w = self.render.input_inner_w as usize;
        if max_w < 10 {
            return;
        }
        let (row, _col) = self.input.cursor();
        let Some(line) = self.input.lines().get(row).cloned() else {
            return;
        };
        let char_count = line.chars().count();
        if char_count <= max_w {
            return;
        }
        // Prefer breaking at a whitespace at-or-before max_w so words
        // stay intact. The whitespace itself becomes the line break
        // (deleted, replaced with newline). If no whitespace exists in
        // that range, hard-wrap exactly at max_w.
        let chars: Vec<char> = line.chars().collect();
        let break_pos = chars[..max_w].iter().rposition(|c| c.is_whitespace());
        match break_pos {
            Some(pos) => {
                // Position cursor at the whitespace, delete it forward,
                // insert newline. End cursor on the new line at the same
                // logical word position the user was typing into.
                self.input
                    .move_cursor(tui_textarea::CursorMove::Jump(row as u16, pos as u16));
                self.input.delete_str(1);
                self.input.insert_newline();
                self.input.move_cursor(tui_textarea::CursorMove::End);
            }
            None => {
                // Long unbroken token (e.g. a pasted URL) — hard-wrap.
                self.input
                    .move_cursor(tui_textarea::CursorMove::Jump(row as u16, max_w as u16));
                self.input.insert_newline();
                self.input.move_cursor(tui_textarea::CursorMove::End);
            }
        }
    }

    /// Refresh `inline_popup` based on the current input state. Called
    /// after every key the textarea consumed.
    fn update_inline_popup(&mut self) {
        let first = self.input.lines().first().cloned().unwrap_or_default();
        let (row, col) = self.input.cursor();
        if row != 0 {
            self.inline_popup = InlinePopup::None;
            return;
        }
        match inline::detect_trigger(&first, col) {
            Some(('/', filter)) => match &mut self.inline_popup {
                InlinePopup::Slash(p) => p.set_filter(filter),
                _ => self.inline_popup = InlinePopup::Slash(SlashPopup::new(filter)),
            },
            Some(('@', filter)) => match &mut self.inline_popup {
                InlinePopup::File(p) => p.set_filter(filter),
                _ => {
                    self.inline_popup = InlinePopup::File(FilePopup::new(
                        filter,
                        &self.workspace,
                        self.workspace_trusted(),
                    ));
                }
            },
            _ => {
                self.inline_popup = InlinePopup::None;
            }
        }
    }

    fn inline_popup_select_prev(&mut self) {
        match &mut self.inline_popup {
            InlinePopup::Slash(p) => {
                if p.index > 0 {
                    p.index -= 1;
                }
            }
            InlinePopup::File(p) => {
                if p.index > 0 {
                    p.index -= 1;
                }
            }
            InlinePopup::None => {}
        }
    }

    fn inline_popup_select_next(&mut self) {
        match &mut self.inline_popup {
            InlinePopup::Slash(p) => {
                if p.index + 1 < p.matches.len() {
                    p.index += 1;
                }
            }
            InlinePopup::File(p) => {
                if p.index + 1 < p.matches.len() {
                    p.index += 1;
                }
            }
            InlinePopup::None => {}
        }
    }

    /// Slash popup only — Enter-key path. Runs the highlighted command
    /// immediately instead of just inserting its text into the input
    /// box (the legacy `inline_popup_complete` behaviour, kept for
    /// Tab). Closes the popup, resets the input, and dispatches the
    /// command through the normal `parse_command` → `handle_command`
    /// pipeline so the resulting `AppAction` (e.g. opening a modal)
    /// propagates back to the event loop.
    ///
    /// Commands that need positional args (`/ask <name> <query>`,
    /// `/model <name>`) won't work here — pressing Enter on those
    /// from the popup dispatches the bare command and the user gets
    /// the command's "missing args" feedback. That's the correct UX:
    /// picking from the popup is for "run the command" intent, not
    /// "insert and edit" intent (Tab still does the latter).
    fn inline_popup_run(
        &mut self,
        tx: &tokio::sync::mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        let cmd_text = match &self.inline_popup {
            InlinePopup::Slash(p) => p
                .matches
                .get(p.index)
                .map(|&idx| format!("/{}", SLASH_COMMANDS[idx].name)),
            _ => None,
        };
        // Close popup + clear input regardless of whether we found a
        // command (an unexpected None case shouldn't leave the popup
        // stuck open).
        self.inline_popup = InlinePopup::None;
        self.reset_input();
        let Some(text) = cmd_text else {
            return AppAction::Continue;
        };
        if let Some(cmd) = parse_command(&text) {
            return self.handle_command(cmd, tx);
        }
        // Slash popup should never surface a non-command match, but if
        // it does (e.g. a future filter accepts arbitrary text), send
        // it through as a normal message so we never silently drop.
        self.send_to_llm(text, tx);
        AppAction::Continue
    }

    /// Insert the currently-highlighted completion into the input at the
    /// trigger position, replacing the partial filter the user typed.
    /// Closes the popup either way.
    ///
    /// Subtlety: tui-textarea's `delete_str(n)` deletes n chars **after**
    /// the cursor, not before, so we jump the cursor to the start of the
    /// range we want to replace, delete forward, then insert.
    fn inline_popup_complete(&mut self) {
        let (row, col) = self.input.cursor();
        if row != 0 {
            self.inline_popup = InlinePopup::None;
            return;
        }
        match &self.inline_popup {
            InlinePopup::Slash(p) => {
                if let Some(&idx) = p.matches.get(p.index) {
                    let cmd = SLASH_COMMANDS[idx].name;
                    // Slash trigger always starts at column 0 — jump there
                    // and delete the `/<filter>` we're replacing.
                    self.input.move_cursor(tui_textarea::CursorMove::Jump(0, 0));
                    self.input.delete_str(col);
                    self.input.insert_str(format!("/{cmd} "));
                }
            }
            InlinePopup::File(p) => {
                if let Some(&idx) = p.matches.get(p.index) {
                    let path = p.workspace_files[idx].to_string_lossy().to_string();
                    let first = self.input.lines().first().cloned().unwrap_or_default();
                    let head_len = col.min(first.chars().count());
                    let head: String = first.chars().take(head_len).collect();
                    if let Some(at_byte) = head.rfind('@') {
                        let at_char_pos = head[..at_byte].chars().count();
                        let to_delete = head_len.saturating_sub(at_char_pos);
                        self.input
                            .move_cursor(tui_textarea::CursorMove::Jump(0, at_char_pos as u16));
                        self.input.delete_str(to_delete);
                        self.input.insert_str(format!("@{path} "));
                    }
                }
            }
            InlinePopup::None => {}
        }
        self.inline_popup = InlinePopup::None;
    }

    fn submit(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) -> AppAction {
        let text = self.input.lines().join("\n").trim().to_string();
        if text.is_empty() {
            return AppAction::Continue;
        }
        self.yn_pending = false;
        self.awaiting_yn_followup = false;
        self.reset_input();

        if let Some(cmd) = parse_command(&text) {
            return self.handle_command(cmd, tx);
        }

        self.send_to_llm(text, tx);
        AppAction::Continue
    }
}
