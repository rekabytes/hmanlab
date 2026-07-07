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
                self.resolve_confirm(true);
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.resolve_confirm(false);
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

    /// Key routing for the `/mcp` web-search provider setup modal.
    ///
    /// Screen 1 (ProviderList): ↑↓/j/k navigate, Enter advances, Esc exits.
    /// Screen 2 (KeyInput):     printable chars feed the textarea, Enter saves,
    ///                          Ctrl+D clears the stored key, Esc goes back.
    /// Screen 3 (Confirmed):    any Enter or Esc returns to Chat.
    pub(in crate::app) fn handle_mcp_setup_key(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        use crate::app::mcp_providers::MCP_PROVIDERS;
        use crate::app::state::McpSetupScreen;

        match self.mcp_setup_screen {
            // ── Screen 1: provider list ──────────────────────────────────
            McpSetupScreen::ProviderList => match key.code {
                KeyCode::Esc => {
                    self.mode = Mode::Chat;
                    self.status = "Cancelled".into();
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.mcp_setup_index > 0 {
                        self.mcp_setup_index -= 1;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.mcp_setup_index + 1 < MCP_PROVIDERS.len() {
                        self.mcp_setup_index += 1;
                    }
                }
                KeyCode::Enter => {
                    let provider = &MCP_PROVIDERS[self.mcp_setup_index];
                    if provider.needs_key {
                        // Pre-fill input with existing key if there is one.
                        let existing = self
                            .mcp_keys
                            .get(provider.id)
                            .cloned()
                            .unwrap_or_default();
                        self.mcp_setup_input = crate::app::fresh_textarea();
                        if !existing.is_empty() {
                            self.mcp_setup_input.insert_str(&existing);
                        }
                        self.mcp_setup_error = None;
                        self.mcp_setup_screen = McpSetupScreen::KeyInput;
                    } else {
                        // Key-free providers (Exa basic, Parallel) activate immediately.
                        self.mcp_active_provider = Some(provider.id.to_string());
                        self.persist_mcp_config(tx);
                        self.mcp_setup_screen = McpSetupScreen::Confirmed;
                        self.status = format!("web_search active: {}", provider.label);
                    }
                }
                _ => {}
            },

            // ── Screen 2: API key input ──────────────────────────────────
            McpSetupScreen::KeyInput => {
                // Clear inline error on any keypress.
                self.mcp_setup_error = None;

                match key.code {
                    KeyCode::Esc => {
                        self.mcp_setup_screen = McpSetupScreen::ProviderList;
                    }
                    // Ctrl+D: remove stored key for this provider.
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let id = MCP_PROVIDERS[self.mcp_setup_index].id;
                        self.mcp_keys.remove(id);
                        // If this was the active provider, deactivate.
                        if self.mcp_active_provider.as_deref() == Some(id) {
                            self.mcp_active_provider = None;
                        }
                        self.persist_mcp_config(tx);
                        self.mcp_setup_input = crate::app::fresh_textarea();
                        self.mcp_setup_screen = McpSetupScreen::ProviderList;
                        self.status = "API key cleared".into();
                    }
                    KeyCode::Enter => {
                        let key_text = self
                            .mcp_setup_input
                            .lines()
                            .first()
                            .cloned()
                            .unwrap_or_default()
                            .trim()
                            .to_string();
                        if key_text.is_empty() {
                            self.mcp_setup_error =
                                Some("API key cannot be empty.".into());
                        } else {
                            let id = MCP_PROVIDERS[self.mcp_setup_index].id;
                            let label = MCP_PROVIDERS[self.mcp_setup_index].label;
                            self.mcp_keys.insert(id.to_string(), key_text);
                            self.mcp_active_provider = Some(id.to_string());
                            self.persist_mcp_config(tx);
                            self.mcp_setup_screen = McpSetupScreen::Confirmed;
                            self.status = format!("web_search active: {label}");
                        }
                    }
                    _ => {
                        // All other keys are forwarded to the textarea.
                        self.mcp_setup_input.input(key);
                    }
                }
            }

            // ── Screen 3: confirmation ───────────────────────────────────
            McpSetupScreen::Confirmed => match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    self.mode = Mode::Chat;
                }
                _ => {}
            },
        }
        AppAction::Continue
    }
}
