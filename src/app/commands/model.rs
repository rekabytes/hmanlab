//! Model selection + BYOK key-entry flows.
//!
//! Covers:
//!   - `/model <name>` — fuzzy-pick from the Ollama-discovered list.
//!   - `/models` — print the discovered list inline.
//!   - The Ctrl+M picker (rebuild entries + open in picker mode).
//!   - The "+ Add … key" flow for BYOK providers (z.ai, Ollama Cloud,
//!     OpenCode) and the modal that collects the key.
//!   - `persist_last_model` — best-effort write so the next launch boots
//!     into the user's last choice instead of the default.
//!
//! Picker entries are rebuilt every time the modal opens, not cached, so
//! "+ Add … key" rows disappear the moment the key is saved.
//!
//! For BYOK providers, saving a key also auto-switches the active model to
//! that provider's `*_DEFAULT_MODEL` so the user can start chatting
//! immediately — no need to re-open the picker.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tui_textarea::Input;

use super::super::{
    fresh_textarea, App, AppAction, Mode, ModelPickerLevel, ModelRow, ProviderRow, StreamMsg,
};

impl App {
    /// Build the level-1 provider list. Called each time the picker opens.
    pub(in crate::app) fn rebuild_picker_entries(&mut self) {
        let mut rows: Vec<ProviderRow> = Vec::new();

        // Ollama (local) — always shown when models are available or host is set.
        if !self.models.is_empty() {
            rows.push(ProviderRow::Active {
                provider: None,
                label: "Ollama (local)".to_string(),
                model_count: self.models.len(),
            });
        }

        // Configured BYOK providers — one row per provider that has models.
        for provider in crate::config::BYOK_PROVIDERS {
            if self.has_byok_key(provider) {
                let count = self
                    .extra_models
                    .iter()
                    .filter(|m| m.provider == *provider)
                    .count();
                rows.push(ProviderRow::Active {
                    provider: Some((*provider).to_string()),
                    label: crate::config::provider_label(provider).to_string(),
                    model_count: count,
                });
            }
        }

        // Unconnected providers — "+ Connect" rows at the bottom.
        for provider in crate::config::BYOK_PROVIDERS {
            if !self.has_byok_key(provider) {
                rows.push(ProviderRow::Add((*provider).to_string()));
            }
        }

        self.model_picker.set_items(rows);

        // Pre-select the row matching the current active provider.
        let active_provider = self.selected_extra.as_ref().map(|e| e.provider.as_str());
        self.model_picker.index = self
            .model_picker
            .items
            .iter()
            .position(|row| match row {
                ProviderRow::Active { provider, .. } => match (provider, active_provider) {
                    (None, None) => true,
                    (Some(p), Some(ap)) => p == ap,
                    _ => false,
                },
                ProviderRow::Add(_) => false,
            })
            .unwrap_or(0);
    }

    /// Build level-2 model list for the given provider tag.
    /// `None` = local Ollama, `Some(id)` = BYOK provider.
    pub(in crate::app) fn rebuild_model_rows(&mut self, provider: Option<&str>) {
        let rows: Vec<ModelRow> = match provider {
            None => self
                .models
                .iter()
                .map(|m| ModelRow::Ollama(m.clone()))
                .collect(),
            Some(prov) => self
                .extra_models
                .iter()
                .filter(|m| m.provider == prov)
                .map(|m| ModelRow::Extra(m.clone()))
                .collect(),
        };
        self.model_picker_models.set_items(rows);

        // Pre-select the active model if it's in this provider.
        let active_provider = self.selected_extra.as_ref().map(|e| e.provider.as_str());
        let is_active_provider = match (provider, active_provider) {
            (None, None) => true,
            (Some(p), Some(ap)) => p == ap,
            _ => false,
        };
        if is_active_provider {
            self.model_picker_models.index = self
                .model_picker_models
                .items
                .iter()
                .position(|r| r.name() == self.model)
                .unwrap_or(0);
        }
    }

    pub(in crate::app) fn open_picker(&mut self) {
        self.rebuild_picker_entries();
        self.model_picker_level = ModelPickerLevel::Provider;
        self.mode = Mode::ModelPicker;
        self.status = "↑↓ select provider  ·  Enter open  ·  Esc cancel".into();
    }

    /// Begin the "+ Connect" key-entry flow for the given provider.
    pub(in crate::app) fn begin_add_model(&mut self, provider: &str) {
        self.add_model_provider = provider.to_string();
        self.add_model_input = fresh_textarea();
        let (placeholder, label) = match provider {
            p if p == crate::config::ZAI_USAGE_PROVIDER => {
                ("Paste your z.ai usage-based API key", "z.ai usage-based")
            }
            p if p == crate::config::OLLAMA_CLOUD_PROVIDER => (
                "Paste your Ollama Cloud API key (from https://ollama.com/settings/keys)",
                "Ollama Cloud",
            ),
            p if p == crate::config::OPENCODE_PROVIDER => (
                "Paste your OpenCode API key (from https://opencode.ai/zen)",
                "OpenCode",
            ),
            p if p == crate::config::OPENROUTER_PROVIDER => (
                "Paste your OpenRouter API key (from https://openrouter.ai/settings/keys)",
                "OpenRouter",
            ),
            p if p == crate::config::HMANLAB_PROVIDER => (
                "Paste your hmanlab API key (sk-… from https://ai.hmanlab.pro)",
                "hmanlab",
            ),
            _ => ("Paste your z.ai coding-plan API key", "z.ai subscription"),
        };
        self.add_model_input.set_placeholder_text(placeholder);
        self.mode = Mode::AddModel;
        self.status = format!("Adding {label} key — Esc to cancel");
    }

    pub(in crate::app) fn handle_add_model(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Chat;
                self.status = "Add model cancelled".into();
                return AppAction::Continue;
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                let val = self.add_model_input.lines().join("").trim().to_string();
                if val.is_empty() {
                    return AppAction::Continue;
                }
                let provider = self.add_model_provider.clone();
                self.set_byok_key(&provider, val);
                let (label, default_model) = match provider.as_str() {
                    p if p == crate::config::ZAI_USAGE_PROVIDER => {
                        self.ensure_zai_models_for(&provider);
                        ("z.ai usage-based", crate::config::ZAI_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OLLAMA_CLOUD_PROVIDER => {
                        self.ensure_ollama_cloud_models();
                        ("Ollama Cloud", crate::config::OLLAMA_CLOUD_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OPENCODE_PROVIDER => {
                        self.ensure_opencode_models();
                        ("OpenCode", crate::config::OPENCODE_DEFAULT_MODEL)
                    }
                    p if p == crate::config::OPENROUTER_PROVIDER => {
                        self.ensure_openrouter_models();
                        ("OpenRouter", crate::config::OPENROUTER_DEFAULT_MODEL)
                    }
                    p if p == crate::config::HMANLAB_PROVIDER => {
                        self.ensure_hmanlab_models();
                        ("hmanlab", crate::config::HMANLAB_DEFAULT_MODEL)
                    }
                    _ => {
                        self.ensure_zai_models_for(&provider);
                        ("z.ai subscription", crate::config::ZAI_DEFAULT_MODEL)
                    }
                };
                self.persist_config();
                let name = default_model.to_string();
                let target_extra = self
                    .extra_models
                    .iter()
                    .find(|m| m.provider == provider && m.name == name)
                    .cloned();
                self.model = name;
                self.selected_extra = target_extra;
                self.mode = Mode::Chat;
                self.status = format!("{label} key saved · using {}", self.model);
                if provider == crate::config::OPENROUTER_PROVIDER {
                    self.refresh_openrouter_models(tx);
                }
                return AppAction::Continue;
            }
            _ => {}
        }
        let input: Input = key.into();
        self.add_model_input.input(input);
        AppAction::Continue
    }

    pub(in crate::app) fn switch_model(&mut self, name: &str) {
        let lower = name.to_lowercase();
        let exact = self
            .models
            .iter()
            .find(|m| m.eq_ignore_ascii_case(name))
            .cloned();
        let partial: Vec<String> = self
            .models
            .iter()
            .filter(|m| m.to_lowercase().contains(&lower))
            .cloned()
            .collect();
        let chosen = if let Some(e) = exact {
            Some(e)
        } else if partial.len() == 1 {
            Some(partial[0].clone())
        } else if partial.is_empty() {
            let available = if self.models.is_empty() {
                "(none — connect with /host <url>)".into()
            } else {
                self.models.join("\n  ")
            };
            self.push_info(format!(
                "No model matches '{name}'. Available:\n  {available}"
            ));
            self.status = format!("No match: {name}");
            None
        } else {
            self.push_info(format!(
                "Multiple matches for '{name}':\n  {}",
                partial.join("\n  ")
            ));
            self.status = format!("Ambiguous: {name}");
            None
        };
        if let Some(m) = chosen {
            self.model = m;
            self.selected_extra = None;
            self.push_info(format!("Switched to model: {}", self.model));
            self.status = format!("Model: {}", self.model);
            let _ = persist_last_model(&self.model, None);
        }
    }

    pub(in crate::app) fn list_models_inline(&mut self) {
        if self.models.is_empty() {
            self.push_info("No models. Try /host <url> first.".into());
            return;
        }
        let list: Vec<String> = self
            .models
            .iter()
            .map(|m| {
                if m == &self.model {
                    format!("  * {m}  (current)")
                } else {
                    format!("    {m}")
                }
            })
            .collect();
        self.push_info(format!(
            "Available models ({}):\n{}",
            self.models.len(),
            list.join("\n")
        ));
    }
}

pub(in crate::app) fn persist_last_model(
    model: &str,
    provider: Option<&str>,
) -> anyhow::Result<()> {
    let mut cfg = crate::config::load()?.unwrap_or_default();
    cfg.last_model = Some(model.to_string());
    cfg.last_provider = provider.map(|s| s.to_string());
    crate::config::save(&cfg)
}
