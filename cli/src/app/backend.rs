//! Backend selection — Ollama for local models, OpenAI-compat for BYOK
//! providers. Routing is driven by `App.selected_extra`: when it's `Some`,
//! the provider field decides which URL + key combo to use; when it's
//! `None`, we're on Ollama.

use tokio::sync::mpsc;

use crate::config::{
    ExtraModel, HMANLAB_BASE, HMANLAB_MODELS, HMANLAB_PROVIDER, MINIMAX_BASE, MINIMAX_MODELS,
    MINIMAX_PROVIDER, OLLAMA_CLOUD_BASE, OLLAMA_CLOUD_MODELS, OLLAMA_CLOUD_PROVIDER, OPENCODE_BASE,
    OPENCODE_MODELS, OPENCODE_PROVIDER, OPENROUTER_BASE, OPENROUTER_MODELS, OPENROUTER_PROVIDER,
    OPENROUTER_VENDORS, ZAI_MODELS, ZAI_SUBSCRIPTION_BASE, ZAI_SUBSCRIPTION_PROVIDER,
    ZAI_USAGE_BASE, ZAI_USAGE_PROVIDER,
};
use crate::ollama::Client;
use crate::openai_compat;

use super::{App, StreamMsg};

/// Backend that fulfills a chat turn. Ollama for local models, OpenAI-compat
/// for any BYOK provider (z.ai subscription, z.ai usage-based, etc).
#[derive(Clone)]
pub enum LlmBackend {
    Ollama(Client),
    OpenAi(openai_compat::Client),
}

impl App {
    /// Host URL that the header should show — reflects the actual backend
    /// for the current model, not just the Ollama URL.
    pub fn current_host(&self) -> &str {
        match self.selected_extra.as_ref().map(|e| e.provider.as_str()) {
            Some(ZAI_SUBSCRIPTION_PROVIDER) => ZAI_SUBSCRIPTION_BASE,
            Some(ZAI_USAGE_PROVIDER) => ZAI_USAGE_BASE,
            Some(OLLAMA_CLOUD_PROVIDER) => OLLAMA_CLOUD_BASE,
            Some(OPENCODE_PROVIDER) => OPENCODE_BASE,
            Some(OPENROUTER_PROVIDER) => OPENROUTER_BASE,
            Some(HMANLAB_PROVIDER) => HMANLAB_BASE,
            Some(MINIMAX_PROVIDER) => MINIMAX_BASE,
            _ => &self.client.base,
        }
    }

    /// Build the LLM backend to use for the current `self.model`. Thin
    /// wrapper around [`Self::make_backend_for`] that reads the active
    /// model's provider off `self.selected_extra`.
    pub fn make_backend(&self) -> Option<LlmBackend> {
        let provider = self.selected_extra.as_ref().map(|e| e.provider.as_str());
        self.make_backend_for(provider)
    }

    /// Build a backend for an arbitrary provider, NOT necessarily the
    /// active one. Used by the specialist agents (`/ask`, and phase 2's
    /// `consult_specialist` tool) so they can run on a different model
    /// without touching `self.model` / `self.selected_extra`. Pass
    /// `None` for local Ollama; pass `Some(provider_id)` (one of the
    /// `*_PROVIDER` constants from `crate::config`) for a BYOK provider.
    pub fn make_backend_for(&self, provider: Option<&str>) -> Option<LlmBackend> {
        let Some(provider) = provider else {
            return Some(LlmBackend::Ollama(self.client.clone()));
        };
        let key = self.byok_key(provider)?.to_string();
        match provider {
            ZAI_SUBSCRIPTION_PROVIDER => Some(LlmBackend::OpenAi(openai_compat::Client::new(
                ZAI_SUBSCRIPTION_BASE.to_string(),
                key,
            ))),
            ZAI_USAGE_PROVIDER => Some(LlmBackend::OpenAi(openai_compat::Client::new(
                ZAI_USAGE_BASE.to_string(),
                key,
            ))),
            OLLAMA_CLOUD_PROVIDER => Some(LlmBackend::Ollama(Client::with_api_key(
                OLLAMA_CLOUD_BASE.to_string(),
                key,
            ))),
            OPENCODE_PROVIDER => Some(LlmBackend::OpenAi(openai_compat::Client::new(
                OPENCODE_BASE.to_string(),
                key,
            ))),
            OPENROUTER_PROVIDER => Some(LlmBackend::OpenAi(openai_compat::Client::new(
                OPENROUTER_BASE.to_string(),
                key,
            ))),
            HMANLAB_PROVIDER => Some(LlmBackend::OpenAi(openai_compat::Client::new(
                HMANLAB_BASE.to_string(),
                key,
            ))),
            MINIMAX_PROVIDER => Some(LlmBackend::OpenAi(openai_compat::Client::new(
                MINIMAX_BASE.to_string(),
                key,
            ))),
            _ => None,
        }
    }

    /// Build the live specialist roster for this turn — the list passed
    /// into [`crate::agent::agent_loop_with`] so the `consult_specialist`
    /// tool can reach them. Returns an empty vec when:
    ///   - `/agents on` hasn't been run (per-session opt-in),
    ///   - the roster is empty,
    ///   - or no enabled specialist has a working backend (missing API key etc).
    ///
    /// Specialists with a misconfigured backend are silently filtered —
    /// surfacing the partial mismatch every turn would be noisy; the
    /// user can spot the gap via `/agents list` if `consult_specialist`
    /// later complains "no specialist named X".
    pub fn live_specialist_runners(&self) -> Vec<crate::agent::SpecialistRunner> {
        if !self.agents_session_enabled {
            return Vec::new();
        }
        self.agents
            .specialists
            .iter()
            .filter(|s| s.enabled)
            .filter_map(|s| {
                let backend = self.make_backend_for(s.provider.as_deref())?;
                Some(crate::agent::SpecialistRunner {
                    name: s.name.clone(),
                    model: s.model.clone(),
                    task: s.task.clone(),
                    system_prompt: s.system_prompt.clone(),
                    backend,
                })
            })
            .collect()
    }

    /// Public bridge for main.rs to migrate older configs at startup.
    /// Idempotent: re-applies known models to extra_models for every
    /// configured provider key, and rewrites legacy provider strings.
    pub fn ensure_byok_models_pub(&mut self) {
        self.migrate_legacy_providers();
        if self.has_byok_key(ZAI_SUBSCRIPTION_PROVIDER) {
            self.ensure_zai_models_for(ZAI_SUBSCRIPTION_PROVIDER);
        }
        if self.has_byok_key(ZAI_USAGE_PROVIDER) {
            self.ensure_zai_models_for(ZAI_USAGE_PROVIDER);
        }
        if self.has_byok_key(OLLAMA_CLOUD_PROVIDER) {
            self.ensure_ollama_cloud_models();
        }
        if self.has_byok_key(OPENCODE_PROVIDER) {
            self.ensure_opencode_models();
        }
        if self.has_byok_key(OPENROUTER_PROVIDER) {
            self.ensure_openrouter_models();
        }
        if self.has_byok_key(HMANLAB_PROVIDER) {
            self.ensure_hmanlab_models();
        }
        if self.has_byok_key(MINIMAX_PROVIDER) {
            self.ensure_minimax_models();
        }
        self.persist_config();
    }

    /// Replace any persisted `hmanlab` entries with the current HMANLAB_MODELS
    /// seed. Same canonical-replacement pattern as other providers.
    pub(super) fn ensure_hmanlab_models(&mut self) {
        self.extra_models.retain(|m| m.provider != HMANLAB_PROVIDER);
        for name in HMANLAB_MODELS {
            self.extra_models.push(ExtraModel {
                provider: HMANLAB_PROVIDER.to_string(),
                name: (*name).to_string(),
            });
        }
    }

    /// Replace any persisted `minimax` entries with the current
    /// MINIMAX_MODELS seed. Canonical-replacement pattern so the picker
    /// stays in sync with whatever the current MiniMax plan exposes.
    pub(super) fn ensure_minimax_models(&mut self) {
        self.extra_models.retain(|m| m.provider != MINIMAX_PROVIDER);
        for name in MINIMAX_MODELS {
            self.extra_models.push(ExtraModel {
                provider: MINIMAX_PROVIDER.to_string(),
                name: (*name).to_string(),
            });
        }
    }

    /// Make sure all hardcoded z.ai models exist in `extra_models` for the
    /// given provider. Idempotent — no duplicates per provider.
    pub(super) fn ensure_zai_models_for(&mut self, provider: &str) {
        for name in ZAI_MODELS {
            let exists = self
                .extra_models
                .iter()
                .any(|m| m.provider == provider && m.name == *name);
            if !exists {
                self.extra_models.push(ExtraModel {
                    provider: provider.to_string(),
                    name: (*name).to_string(),
                });
            }
        }
    }

    /// Populate the canonical `OLLAMA_CLOUD_MODELS` lineup under the
    /// `ollama-cloud` provider. Unlike `ensure_zai_models_for`, this REPLACES
    /// any existing `ollama-cloud` entries — the cloud catalog has shipped
    /// paid-only models under names we previously seeded as free, and stale
    /// entries from earlier releases would otherwise leave broken picker
    /// rows that 403 on click. Treating `OLLAMA_CLOUD_MODELS` as the source
    /// of truth keeps the persisted config in sync with what the API
    /// actually serves on the free tier.
    pub(super) fn ensure_ollama_cloud_models(&mut self) {
        self.extra_models
            .retain(|m| m.provider != OLLAMA_CLOUD_PROVIDER);
        for name in OLLAMA_CLOUD_MODELS {
            self.extra_models.push(ExtraModel {
                provider: OLLAMA_CLOUD_PROVIDER.to_string(),
                name: (*name).to_string(),
            });
        }
    }

    /// Same canonical-replacement pattern as `ensure_ollama_cloud_models`:
    /// drop any existing `opencode` entries, then push the curated list
    /// from `OPENCODE_MODELS`. Keeps the picker honest if the catalog
    /// shifts between releases.
    pub(super) fn ensure_opencode_models(&mut self) {
        self.extra_models
            .retain(|m| m.provider != OPENCODE_PROVIDER);
        for name in OPENCODE_MODELS {
            self.extra_models.push(ExtraModel {
                provider: OPENCODE_PROVIDER.to_string(),
                name: (*name).to_string(),
            });
        }
    }

    /// Seed the curated OpenRouter model list under the `openrouter` provider.
    /// Additive (not replace) so users who've hand-added models to their
    /// config don't lose them when we bump the seed list — OpenRouter has a
    /// huge catalog and the curated subset is a starting point, not the
    /// authoritative set.
    pub(super) fn ensure_openrouter_models(&mut self) {
        for name in OPENROUTER_MODELS {
            let exists = self
                .extra_models
                .iter()
                .any(|m| m.provider == OPENROUTER_PROVIDER && m.name == *name);
            if !exists {
                self.extra_models.push(ExtraModel {
                    provider: OPENROUTER_PROVIDER.to_string(),
                    name: (*name).to_string(),
                });
            }
        }
    }

    /// Kick a background fetch of OpenRouter's live `/v1/models` catalog,
    /// filter it down to the curated vendor set, and let the stream
    /// handler swap the result into `extra_models`. No-op if no API key
    /// is configured (the catalog endpoint is technically public, but
    /// without a key we have no reason to refresh).
    ///
    /// Filtering rules (intentionally simple — see `OPENROUTER_VENDORS`):
    ///   - Drop anything not from a whitelisted vendor.
    ///   - Drop preview / experimental rows so the picker stays sane:
    ///     anything with `preview` / `experimental` / `beta` / `test` in
    ///     the id, the `~vendor/...-latest` alias rows (the `~` prefix is
    ///     OpenRouter's alias marker, not a real ID we can call), and
    ///     non-chat modalities like `-image`, `-tts`, `-embedding`,
    ///     `-search`.
    ///   - Free-tier `:free` variants stay — users can pick them
    ///     deliberately when they don't want to spend credits.
    pub fn refresh_openrouter_models(&self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let Some(key) = self.byok_key(OPENROUTER_PROVIDER).map(str::to_string) else {
            return;
        };
        let key = Some(key);
        let tx = tx.clone();
        tokio::spawn(async move {
            let raw = match openai_compat::fetch_openrouter_models(OPENROUTER_BASE, key.as_deref())
                .await
            {
                Ok(list) => list,
                Err(_) => {
                    // Silent fail: the static seed still works as fallback,
                    // and we don't want a transient network blip to clutter
                    // the chat with errors. Status bar will already say
                    // whatever it was before.
                    return;
                }
            };
            let filtered: Vec<String> = raw
                .into_iter()
                .filter(|id| {
                    // `~vendor/...` are alias rows, not real IDs.
                    if id.starts_with('~') {
                        return false;
                    }
                    // Vendor prefix gate.
                    let vendor = id.split('/').next().unwrap_or("");
                    if !OPENROUTER_VENDORS.contains(&vendor) {
                        return false;
                    }
                    // Drop non-chat modalities + experimental tags. Match
                    // on the id substring; keeps the filter compact.
                    let lower = id.to_ascii_lowercase();
                    const REJECT: &[&str] = &[
                        "preview",
                        "experimental",
                        "-beta",
                        "-test",
                        "-image",
                        "-tts",
                        "-embedding",
                        "-search",
                    ];
                    !REJECT.iter().any(|tag| lower.contains(tag))
                })
                .collect();
            let _ = tx.send(StreamMsg::OpenRouterModelsRefreshed(filtered));
        });
    }

    /// Rewrite legacy provider strings to current ids. Safe to call repeatedly.
    pub(super) fn migrate_legacy_providers(&mut self) {
        for em in self.extra_models.iter_mut() {
            if em.provider == "zai" {
                em.provider = ZAI_SUBSCRIPTION_PROVIDER.to_string();
            }
            if em.provider == "hmanlab-prox" {
                em.provider = HMANLAB_PROVIDER.to_string();
            }
        }
    }

    /// Write the current BYOK settings back to ~/.config/hmanlab/config.json.
    /// Silent on error — the running state is still consistent.
    ///
    /// The actual `load + modify + save` sequence runs on a blocking
    /// worker thread so the UI loop never blocks on disk I/O. A static
    /// mutex serialises the writes so two persist calls in quick
    /// succession (e.g. add a BYOK key + auto-seed its models) can't
    /// race and lose each other's updates — each `spawn_blocking`
    /// reads the file fresh under the lock and writes back.
    pub(in crate::app) fn persist_config(&self) {
        // Snapshot grabs the BYOK map by clone so the writer task is
        // independent of `&self`. On-disk layout still uses one
        // `Option<String>` per provider for backwards compat — the
        // closure below fans `byok_keys` back out into those fields.
        let snap = PersistSnapshot {
            byok_keys: self.byok_keys.clone(),
            extra_models: self.extra_models.clone(),
            agents: self.agents.clone(),
        };
        tokio::task::spawn_blocking(move || {
            let _g = CONFIG_WRITE_LOCK.lock();
            let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
            cfg.zai_api_key = snap.byok_keys.get(ZAI_SUBSCRIPTION_PROVIDER).cloned();
            cfg.zai_usage_api_key = snap.byok_keys.get(ZAI_USAGE_PROVIDER).cloned();
            cfg.ollama_cloud_api_key = snap.byok_keys.get(OLLAMA_CLOUD_PROVIDER).cloned();
            cfg.opencode_api_key = snap.byok_keys.get(OPENCODE_PROVIDER).cloned();
            cfg.openrouter_api_key = snap.byok_keys.get(OPENROUTER_PROVIDER).cloned();
            cfg.hmanlab_api_key = snap.byok_keys.get(HMANLAB_PROVIDER).cloned();
            cfg.minimax_api_key = snap.byok_keys.get(MINIMAX_PROVIDER).cloned();
            cfg.extra_models = snap.extra_models;
            cfg.agents = snap.agents;
            let _ = crate::config::save(&cfg);
        });
    }
}

/// Owned copy of just the fields `persist_config` writes. Moved into
/// the spawn_blocking closure so the snapshot is independent of the
/// `&self` borrow. The BYOK map is fanned out into per-provider config
/// fields inside the closure (config keeps the legacy on-disk shape).
struct PersistSnapshot {
    byok_keys: std::collections::HashMap<String, String>,
    extra_models: Vec<ExtraModel>,
    agents: crate::config::AgentsConfig,
}

/// Serialises concurrent `persist_config` calls so the load-modify-save
/// sequence is atomic across worker threads.
static CONFIG_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
