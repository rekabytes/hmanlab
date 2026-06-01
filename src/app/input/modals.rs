//! Key handlers for modes that briefly take over the screen: the model
//! picker (Ctrl+M), the confirm dialog (Y/N + diff scroll), the session
//! picker (/sessions), and the in-chat file viewer overlay.
//!
//! Each handler is registered in `event.rs::handle_event` and called
//! while its corresponding `Mode::*` is active. They share no state
//! beyond what's on `App`; placing them together keeps the keymap for
//! "what does Enter mean right now" co-located.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;

use super::super::commands::model::persist_last_model;
use super::super::{App, AppAction, Mode, ModelPickerLevel, ProviderRow, StreamMsg};

impl App {
    /// Key routing while the file viewer is open. Esc dismisses; arrow /
    /// page / home / end keys move through the file. Everything else is
    /// swallowed so the chat input doesn't pick up stray characters and the
    /// user can't accidentally fire a command (e.g. Ctrl+N) while reading.
    pub(in crate::app) fn handle_viewer_key(&mut self, key: KeyEvent) -> AppAction {
        let Some(file) = self.open_file.as_mut() else {
            return AppAction::Continue;
        };
        match key.code {
            KeyCode::Esc => {
                self.open_file = None;
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                file.scroll = file.scroll.saturating_add(10);
            }
            KeyCode::PageUp => {
                file.scroll = file.scroll.saturating_sub(10);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                file.scroll = file.scroll.saturating_add(1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                file.scroll = file.scroll.saturating_sub(1);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                file.scroll = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                file.scroll = u16::MAX;
            }
            // Ctrl+C remains an escape hatch so the user can always close.
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_file = None;
            }
            _ => {}
        }
        AppAction::Continue
    }

    pub(in crate::app) fn handle_session_picker(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Chat;
                self.status = "Cancelled".into();
            }
            KeyCode::Up | KeyCode::Char('k') => self.session_picker.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.session_picker.select_next(),
            KeyCode::Enter => {
                if let Some(s) = self.session_picker.selected().cloned() {
                    self.mode = Mode::Chat;
                    self.load_session(s.id, tx);
                }
            }
            _ => {}
        }
        AppAction::Continue
    }

    pub(in crate::app) fn handle_picker(&mut self, key: KeyEvent) -> AppAction {
        match self.model_picker_level.clone() {
            // ── Level 1: provider list ────────────────────────────────────
            ModelPickerLevel::Provider => match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Chat;
                    self.status = "Cancelled".into();
                }
                KeyCode::Up | KeyCode::Char('k') => self.model_picker.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.model_picker.select_next(),
                KeyCode::Enter => {
                    if let Some(row) = self.model_picker.selected().cloned() {
                        match row {
                            ProviderRow::Active { provider, .. } => {
                                // Drill into this provider's model list.
                                self.rebuild_model_rows(provider.as_deref());
                                self.model_picker_level = ModelPickerLevel::Model;
                                let label = provider
                                    .as_deref()
                                    .map(crate::config::provider_label)
                                    .unwrap_or("Ollama (local)");
                                self.status = format!(
                                    "↑↓ select model  ·  Enter confirm  ·  Esc back  ·  {label}"
                                );
                            }
                            ProviderRow::Add(provider) => {
                                self.begin_add_model(&provider);
                            }
                        }
                    }
                }
                _ => {}
            },
            // ── Level 2: model list ───────────────────────────────────────
            ModelPickerLevel::Model => match key.code {
                KeyCode::Esc => {
                    // Go back to level 1.
                    self.model_picker_level = ModelPickerLevel::Provider;
                    self.status = "↑↓ select provider  ·  Enter open  ·  Esc cancel".into();
                }
                KeyCode::Up | KeyCode::Char('k') => self.model_picker_models.select_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.model_picker_models.select_next(),
                KeyCode::Enter => {
                    if let Some(row) = self.model_picker_models.selected().cloned() {
                        let name = row.name().to_string();
                        let provider = row.provider().map(|s| s.to_string());
                        match row {
                            crate::app::ModelRow::Ollama(_) => {
                                self.model = name.clone();
                                self.selected_extra = None;
                                self.status = format!("Switched to {name}");
                                let _ = persist_last_model(&self.model, None);
                            }
                            crate::app::ModelRow::Extra(m) => {
                                self.model = m.name.clone();
                                self.status = format!("Switched to [{}] {}", m.provider, m.name);
                                let prov = m.provider.clone();
                                self.selected_extra = Some(m);
                                let _ = persist_last_model(&self.model, Some(&prov));
                            }
                        }
                        let _ = provider;
                        self.mode = Mode::Chat;
                    } else {
                        self.mode = Mode::Chat;
                    }
                }
                _ => {}
            },
        }
        AppAction::Continue
    }

    pub(in crate::app) fn handle_confirm(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            // Scroll the diff body. ↑↓ for fine-grained, PgUp/PgDn for
            // a page at a time. The renderer clamps to a valid max so
            // saturating_add never runs past the end visibly.
            KeyCode::Up => {
                self.confirm_scroll = self.confirm_scroll.saturating_sub(1);
                return AppAction::Continue;
            }
            KeyCode::Down => {
                self.confirm_scroll = self.confirm_scroll.saturating_add(1);
                return AppAction::Continue;
            }
            KeyCode::PageUp => {
                self.confirm_scroll = self.confirm_scroll.saturating_sub(10);
                return AppAction::Continue;
            }
            KeyCode::PageDown => {
                self.confirm_scroll = self.confirm_scroll.saturating_add(10);
                return AppAction::Continue;
            }
            KeyCode::Home => {
                self.confirm_scroll = 0;
                return AppAction::Continue;
            }
            KeyCode::End => {
                self.confirm_scroll = u16::MAX;
                return AppAction::Continue;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                if let Some(req) = self.pending_confirm.take() {
                    // Attach the authorised diff DIRECTLY to the running
                    // tool placeholder right now — `active_tool_msg_idx`
                    // already points at it. Attaching here means the diff
                    // is in place before the file even gets written, so
                    // click-to-expand on the finished tool row always
                    // shows the diff. Empty-diff tools (run_command etc.)
                    // skip the attach.
                    if !req.diff.is_empty() {
                        if let Some(idx) = self.active_tool_msg_idx {
                            if let Some(msg) = self.messages.get_mut(idx) {
                                msg.diff = Some(req.diff.clone());
                            }
                        }
                    }
                    let _ = req.responder.send(true);
                    // Memory ops already render a tight `memory · save …`
                    // tile + a result row — adding a third `✓ Allowed:`
                    // info line just for them is noise. Other tools
                    // (write_file / edit_file / run_command) still get
                    // the line so the user can see what they greenlit.
                    let is_memory_op = req.prompt.starts_with("SAVE memory")
                        || req.prompt.starts_with("UPDATE memory")
                        || req.prompt.starts_with("FORGET memory");
                    if !is_memory_op {
                        self.push_info(format!("✓ Allowed: {}", req.prompt));
                    }
                    // If we'd also DM'd this prompt to Telegram, overwrite
                    // the original message there so the buttons no longer
                    // look actionable.
                    if let Some(ctx) = self.pending_telegram_confirm.take() {
                        let body = format!("✓ Allowed locally by the hmanlab user: {}", req.prompt);
                        if let Some(message_id) = ctx.message_id {
                            self.edit_telegram_message(ctx.chat_id, message_id, body);
                        } else {
                            self.send_telegram_dm(ctx.chat_id, body);
                        }
                    }
                }
                self.mode = Mode::Chat;
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                if let Some(req) = self.pending_confirm.take() {
                    let _ = req.responder.send(false);
                    self.push_info(format!("✗ Denied: {}", req.prompt));
                    if let Some(ctx) = self.pending_telegram_confirm.take() {
                        let body = format!("✗ Denied locally by the hmanlab user: {}", req.prompt);
                        if let Some(message_id) = ctx.message_id {
                            self.edit_telegram_message(ctx.chat_id, message_id, body);
                        } else {
                            self.send_telegram_dm(ctx.chat_id, body);
                        }
                    }
                }
                self.mode = Mode::Chat;
            }
            _ => {}
        }
        AppAction::Continue
    }

    /// Key routing for the shell monitor overlay. Esc hides the overlay
    /// (the child keeps running and the runtime stays in `active_shell`,
    /// so the user can re-open via the footer indicator). Ctrl+C fires
    /// the kill channel so the shell tool's `tokio::select!` reaps the
    /// child and reports `[killed by user]`. PgUp/PgDn scroll the
    /// buffer; we drop `follow_tail` on PgUp so new output doesn't
    /// snap the user back to the bottom while they're reading history.
    pub(in crate::app) fn handle_shell_monitor_key(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                // If the shell has already exited, dismiss the runtime
                // entirely — there's nothing left to monitor and the
                // archived output lives on in the tool tile.
                let finished = self
                    .active_shell
                    .as_ref()
                    .map(|rt| !rt.running)
                    .unwrap_or(true);
                if finished {
                    self.active_shell = None;
                }
                self.mode = Mode::Chat;
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(rt) = self.active_shell.as_mut() {
                    if rt.running {
                        if let Some(tx) = rt.kill_tx.take() {
                            let _ = tx.send(());
                        }
                    }
                }
            }
            KeyCode::PageUp => {
                if let Some(rt) = self.active_shell.as_mut() {
                    rt.follow_tail = false;
                    rt.scroll = rt.scroll.saturating_sub(5);
                }
            }
            KeyCode::PageDown => {
                if let Some(rt) = self.active_shell.as_mut() {
                    let next = rt.scroll.saturating_add(5);
                    rt.scroll = next;
                    // Re-arm follow if user scrolled past everything —
                    // they're back at the live tail.
                    let total = rt.output.len() as u16;
                    if next >= total {
                        rt.follow_tail = true;
                    }
                }
            }
            KeyCode::End => {
                if let Some(rt) = self.active_shell.as_mut() {
                    rt.follow_tail = true;
                }
            }
            KeyCode::Home => {
                if let Some(rt) = self.active_shell.as_mut() {
                    rt.follow_tail = false;
                    rt.scroll = 0;
                }
            }
            _ => {}
        }
        AppAction::Continue
    }
}
