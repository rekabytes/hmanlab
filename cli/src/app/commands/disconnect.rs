//! `/disconnect` — drop a saved BYOK provider key and remove its
//! associated extra-model rows.
//!
//! Two entry paths:
//!   - `/disconnect <name>` parses a string alias (`zai`, `zai-usage`,
//!     `ollama-cloud`, `opencode`, plus a few extras like `go` and `oc`)
//!     and dispatches straight to `disconnect_by_id`.
//!   - `/disconnect` with no arg opens an arrow-key picker
//!     (`Mode::DisconnectPicker`) listing only currently-connected
//!     providers. `handle_disconnect_picker` translates the picker's
//!     Enter into the same `disconnect_by_id` call.
//!
//! All real work — clearing the key on `App`, sweeping the
//! `extra_models` list, falling back the active model if needed,
//! and persisting the new config — lives in `disconnect_by_id` so both
//! paths stay in sync.

use crossterm::event::{KeyCode, KeyEvent};

use crate::config::ExtraModel;

use super::super::{App, AppAction, DisconnectEntry, Mode};

impl App {
    /// `/disconnect [provider]` — drop a saved BYOK key and the matching
    /// extra-model rows. With no arg, lists what's currently connected so
    /// the user can pick. With an arg, accepts a few common aliases
    /// (`zai`, `zai-usage`, `ollama-cloud`, `opencode`, plus `go` for
    /// OpenCode). If the currently-active model belonged to the
    /// disconnected provider, the active model falls back to the first
    /// Ollama-discovered model, then the first remaining extra, else empty.
    pub(in crate::app) fn handle_disconnect(&mut self, name: &str) {
        let trimmed = name.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            // No arg → open the arrow-key picker. Mirrors how `/model`
            // with no arg opens the model picker.
            self.open_disconnect_picker();
            return;
        }
        let provider_id: &'static str = match trimmed.as_str() {
            "zai" | "zai-subscription" | "subscription" | "z.ai" => {
                crate::config::ZAI_SUBSCRIPTION_PROVIDER
            }
            "zai-usage" | "usage" | "z.ai-usage" => crate::config::ZAI_USAGE_PROVIDER,
            "ollama-cloud" | "cloud" | "ollama_cloud" => crate::config::OLLAMA_CLOUD_PROVIDER,
            "opencode" | "opencode-go" | "go" | "oc" => crate::config::OPENCODE_PROVIDER,
            "openrouter" | "or" => crate::config::OPENROUTER_PROVIDER,
            other => {
                self.push_info(format!(
                    "Unknown provider '{other}'. Try: zai, zai-usage, ollama-cloud, opencode, openrouter.\nOr /disconnect (no args) to open the picker."
                ));
                return;
            }
        };
        self.disconnect_by_id(provider_id);
    }

    /// Drop a BYOK provider by its canonical id. Shared between the
    /// string-arg fast-path (`/disconnect zai`) and the picker
    /// (`/disconnect` → Enter on a row). All key-clearing, model-removal,
    /// active-model fallback, and config persistence live here.
    pub(in crate::app) fn disconnect_by_id(&mut self, provider_id: &str) {
        // No-op guard so the user gets a clean message rather than a
        // silent "removed 0 things".
        if !self.has_byok_key(provider_id) {
            self.push_info(format!(
                "'{provider_id}' isn't connected — nothing to disconnect."
            ));
            return;
        }

        // Clear the key on App; persist_config later flushes it to disk.
        self.remove_byok_key(provider_id);

        // Sweep the matching extra-model rows out of the picker source.
        let removed = self
            .extra_models
            .iter()
            .filter(|m| m.provider == provider_id)
            .count();
        self.extra_models.retain(|m| m.provider != provider_id);

        // Fall back the active model if it belonged to this provider.
        let active_was_provider =
            self.selected_extra.as_ref().map(|e| e.provider.as_str()) == Some(provider_id);
        if active_was_provider {
            self.selected_extra = None;
            if let Some(first) = self.models.first().cloned() {
                self.model = first;
            } else if let Some(em) = self.extra_models.first().cloned() {
                self.model = em.name.clone();
                self.selected_extra = Some(em);
            } else {
                self.model.clear();
            }
        }

        self.persist_config();
        self.push_info(format!(
            "Disconnected {provider_id} — removed {removed} model(s). Use /model to add another provider or pick a different model."
        ));
        self.status = format!("Disconnected {provider_id}");
    }

    /// Build `disconnect_entries` from currently-connected BYOK providers
    /// and switch to `Mode::DisconnectPicker`. If nothing is connected,
    /// push an info row and stay in chat — there's no point opening an
    /// empty popup.
    pub(in crate::app) fn open_disconnect_picker(&mut self) {
        let mut entries: Vec<DisconnectEntry> = Vec::new();
        let preview = |provider: &str, extras: &[ExtraModel]| -> String {
            let names: Vec<&str> = extras
                .iter()
                .filter(|m| m.provider == provider)
                .map(|m| m.name.as_str())
                .collect();
            if names.is_empty() {
                "(no models seeded)".to_string()
            } else if names.len() <= 3 {
                names.join(", ")
            } else {
                format!("{}, +{} more", names[..3].join(", "), names.len() - 3)
            }
        };
        for provider in crate::config::BYOK_PROVIDERS {
            if self.has_byok_key(provider) {
                entries.push(DisconnectEntry {
                    provider: (*provider).to_string(),
                    label: crate::config::provider_label(provider).to_string(),
                    preview: preview(provider, &self.extra_models),
                });
            }
        }
        if entries.is_empty() {
            self.push_info(
                "No BYOK providers connected. Use /model and the + Add … rows to add one.".into(),
            );
            return;
        }
        self.disconnect_picker.set_items(entries);
        self.mode = Mode::DisconnectPicker;
        self.status = "↑↓ to navigate · Enter to disconnect · Esc to cancel".into();
    }

    pub(in crate::app) fn handle_disconnect_picker(&mut self, key: KeyEvent) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Chat;
                self.status = "Disconnect cancelled".into();
            }
            KeyCode::Up | KeyCode::Char('k') => self.disconnect_picker.select_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.disconnect_picker.select_next(),
            KeyCode::Home => self.disconnect_picker.select_first(),
            KeyCode::End => self.disconnect_picker.select_last(),
            KeyCode::Enter => {
                if let Some(entry) = self.disconnect_picker.selected().cloned() {
                    self.mode = Mode::Chat;
                    self.disconnect_by_id(&entry.provider);
                }
            }
            _ => {}
        }
        AppAction::Continue
    }
}
