//! System-level stream handlers: host change (new model list), `/update`
//! result, `/settings` edit-in-place.

use crate::config::{ExtraModel, OPENROUTER_PROVIDER};

use super::super::App;

impl App {
    /// Live OpenRouter catalog arrived — replace this provider's entries
    /// in `extra_models` with the fresh list and persist. Anything else
    /// (other providers, the active selection, etc.) is untouched.
    pub(super) fn on_openrouter_models_refreshed(&mut self, models: Vec<String>) {
        if models.is_empty() {
            // Treat empty as "fetch failed silently" — keep whatever
            // seed we have rather than blowing it away.
            return;
        }
        self.extra_models
            .retain(|m| m.provider != OPENROUTER_PROVIDER);
        for name in &models {
            self.extra_models.push(ExtraModel {
                provider: OPENROUTER_PROVIDER.to_string(),
                name: name.clone(),
            });
        }
        self.persist_config();
        // Status nudge so the user knows the refresh landed — not pushed
        // as a chat message because it's chrome, not conversation.
        self.status = format!("OpenRouter: {} models available", models.len());
    }

    pub(super) fn on_models(&mut self, models: Vec<String>, base: String) {
        let n = models.len();
        self.models = models;
        if !self.models.iter().any(|m| m == &self.model) {
            if let Some(first) = self.models.first() {
                self.model = first.clone();
            }
        }
        self.status = format!("Connected to {base} — {n} model(s)");
        self.push_info(format!(
            "Connected to {base}\nModels available: {n}\nCurrent: {}",
            self.model
        ));
        // /host succeeded — remember the URL across restarts so the
        // user doesn't have to re-add Ollama every session.
        let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
        cfg.ollama_host = Some(base);
        let _ = crate::config::save(&cfg);
    }

    pub(super) fn on_update_result(&mut self, ok: bool, text: String) {
        self.push_info(text);
        self.status = if ok {
            "Update finished — restart Hibiscus to use the new version".into()
        } else {
            "Update failed".into()
        };
        if ok {
            // Clear the header notice: whatever version was advertised
            // has now been installed, even if we can't detect the new
            // version until the user restarts.
            self.update_available = None;
        }
    }

    pub(super) fn on_settings(&mut self, text: String) {
        // Edit-in-place: if the placeholder card is still where we
        // left it (an `info` message at the stashed index), overwrite
        // its content instead of appending. Falls back to push_info
        // if the index drifted (e.g. /clear or another /settings
        // raced ours).
        match self.pending_settings_msg_idx.take() {
            Some(idx)
                if self
                    .messages
                    .get(idx)
                    .map(|m| m.role == "info")
                    .unwrap_or(false) =>
            {
                self.messages[idx].content = text;
                self.follow = true;
            }
            _ => self.push_info(text),
        }
        self.status = "Settings loaded".into();
    }
}
