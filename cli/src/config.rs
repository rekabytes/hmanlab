use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub const DEFAULT_OLLAMA_HOST: &str = "http://localhost:11434";

/// z.ai has two billing plans, each with its own base URL. Treated as
/// independent providers throughout the codebase so a user can hold a key
/// for one or both at the same time.
pub const ZAI_SUBSCRIPTION_BASE: &str = "https://api.z.ai/api/coding/paas/v4";
pub const ZAI_USAGE_BASE: &str = "https://api.z.ai/api/paas/v4";

/// Provider identifiers stored in `ExtraModel.provider`. Old configs that
/// say `"zai"` are migrated to `"zai-subscription"` at App init.
pub const ZAI_SUBSCRIPTION_PROVIDER: &str = "zai-subscription";
pub const ZAI_USAGE_PROVIDER: &str = "zai-usage";

/// Models exposed for each z.ai plan. Same lineup today — if z.ai diverges
/// the two lists, split this constant.
pub const ZAI_MODELS: &[&str] = &["glm-4.7", "glm-4.6", "glm-5.1"];
pub const ZAI_DEFAULT_MODEL: &str = "glm-4.7";

/// OpenCode Go — the opencode.ai subscription tier. Hits
/// `https://opencode.ai/zen/go/v1/chat/completions` with Bearer auth; the
/// same API key generated at https://opencode.ai/zen works for both Zen
/// (pay-per-credit) and Go (subscription-billed), but routing is by URL:
/// requests to `/zen/v1` bill against pay-per-credit; requests to
/// `/zen/go/v1` bill against the Go subscription. We point at the Go URL
/// because Go is what hmanlab users buying this provider are subscribed
/// to. Free-tier-only access (big-pickle, *-free) is intentionally NOT
/// served from this endpoint — those models 401 ModelError on Go and
/// require the Zen URL instead. That's a separate future provider if
/// anyone asks.
///
/// Slug convention: the wire protocol expects **bare model IDs**, not
/// `opencode-go/<id>`. The `opencode-go/` prefix in opencode.ai's docs is
/// only used by their own client's `opencode.json` config file — the
/// HTTPS API rejects it as "Model opencode-go/X not supported". Verified
/// 2026-05-16 against a real Go key.
///
/// Catalog: every Go model that responds 200 to a probe through
/// `/chat/completions`. The docs claim MiniMax requires the `/messages`
/// shape, but empirically it works through `/chat/completions` on Go, so
/// it's included. Closed-weight models (claude-*, gpt-*, gemini-*) still
/// use their own non-OpenAI endpoints (`/messages`, `/responses`,
/// `/models/...`) and are excluded.
pub const OPENCODE_PROVIDER: &str = "opencode";
pub const OPENCODE_BASE: &str = "https://opencode.ai/zen/go/v1";
pub const OPENCODE_MODELS: &[&str] = &[
    "glm-5.1",
    "glm-5",
    "qwen3.6-plus",
    "qwen3.5-plus",
    "kimi-k2.6",
    "kimi-k2.5",
    "minimax-m2.7",
    "minimax-m2.5",
];
pub const OPENCODE_DEFAULT_MODEL: &str = "glm-5.1";

/// Ollama Cloud — separate provider from local Ollama. Same wire protocol
/// (native /api/chat), but reached over HTTPS with a Bearer-auth API key
/// generated at https://ollama.com/settings/keys.
///
/// Full cloud catalog as of 2026-05-22, scraped from
/// https://ollama.com/search?c=cloud. Every entry is *technically* callable
/// from a free key — Ollama Cloud's tier model is GPU-time quota (resets
/// every 5 hours on Free, 50x larger on Pro $20/mo, 5x that again on Max
/// $100/mo), not per-model paywalls. Heavy models (level 3-4: deepseek-v4-*,
/// glm-5*, kimi, minimax, qwen3-next) burn through free quota in a few
/// prompts; light models (level 1-2: gpt-oss:20b, gemma4, ministral-3,
/// nemotron-3-nano, qwen3.5) stretch much further. We list all of them so
/// the picker reflects reality — quota errors surface naturally on use.
///
/// Note: the API accepts both `glm-4.7` and `glm-4.7:cloud` — the `:cloud`
/// suffix is optional. We use bare slugs so the picker display
/// (`[ollama-cloud] glm-4.7`) reads cleanly.
pub const OLLAMA_CLOUD_PROVIDER: &str = "ollama-cloud";
pub const OLLAMA_CLOUD_BASE: &str = "https://ollama.com";
pub const OLLAMA_CLOUD_MODELS: &[&str] = &[
    "deepseek-v3.2",
    "deepseek-v4-flash",
    "deepseek-v4-pro",
    "devstral-small-2",
    "gemini-3-flash-preview",
    "gemma4",
    "glm-4.7",
    "glm-5",
    "glm-5.1",
    "gpt-oss:120b-cloud",
    "kimi-k2.6",
    "minimax-m2.1",
    "minimax-m2.5",
    "minimax-m2.7",
    "ministral-3",
    "nemotron-3-nano",
    "nemotron-3-super",
    "qwen3.5",
    "qwen3-coder-next",
    "qwen3-next",
    "rnj-1",
];
pub const OLLAMA_CLOUD_DEFAULT_MODEL: &str = "glm-4.7";

/// OpenRouter — meta-provider that routes to many model vendors behind one
/// OpenAI-compatible endpoint and one billing relationship. Bearer auth
/// with a key from https://openrouter.ai/settings/keys. Pay-per-token;
/// some free-tier rows exist with a `:free` suffix.
///
/// Slug convention: OpenRouter expects namespaced model IDs of the form
/// `<vendor>/<model>` (e.g. `anthropic/claude-opus-4.7`, `openai/gpt-5.5`).
///
/// `OPENROUTER_MODELS` is the static fallback seed — written once when the
/// user pastes their API key, used immediately so the picker has something
/// to show. After that, `refresh_openrouter_models` fetches the live
/// `/api/v1/models` catalog in the background and replaces these entries
/// with whatever's actually current. So the seed only matters for offline
/// first-launch and as a safety net if openrouter.ai is unreachable.
pub const OPENROUTER_PROVIDER: &str = "openrouter";
pub const OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
pub const OPENROUTER_MODELS: &[&str] = &[
    // Anthropic Claude (current generation).
    "anthropic/claude-opus-4.7",
    "anthropic/claude-sonnet-4.6",
    "anthropic/claude-haiku-4.5",
    // OpenAI GPT-5 family.
    "openai/gpt-5.5",
    "openai/gpt-5.5-pro",
    "openai/gpt-5.4-mini",
    // Google Gemini.
    "google/gemini-3.1-flash-lite",
    "google/gemini-2.5-pro",
    // Other notable hosts.
    "deepseek/deepseek-v4-pro",
    "qwen/qwen3.6-plus",
    "x-ai/grok-4.3",
    "moonshotai/kimi-k2.6",
    "meta-llama/llama-3.3-70b-instruct",
];
pub const OPENROUTER_DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4.6";

/// Vendors whose IDs we keep when filtering the live OpenRouter catalog
/// down to a usable picker size. Everything else is dropped on the
/// theory that 400-row pickers are worse than missing a niche provider —
/// users can still add models manually by editing config.json.
pub const OPENROUTER_VENDORS: &[&str] = &[
    "anthropic",
    "openai",
    "google",
    "deepseek",
    "qwen",
    "meta-llama",
    "mistralai",
    "x-ai",
    "moonshotai",
    "z-ai",
];

/// Every BYOK provider hmanlab can hold an API key for. Used by the
/// `/model` picker (one "+ Add … key" row per unconfigured provider),
/// `/disconnect` (one row per configured one), and `/settings`. Listing
/// them once here keeps the dispatch tables in those files honest —
/// add a provider here and the UI surfaces it automatically.
pub const BYOK_PROVIDERS: &[&str] = &[
    ZAI_SUBSCRIPTION_PROVIDER,
    ZAI_USAGE_PROVIDER,
    OLLAMA_CLOUD_PROVIDER,
    OPENCODE_PROVIDER,
    OPENROUTER_PROVIDER,
    HMANLAB_PROVIDER,
    MINIMAX_PROVIDER,
];

/// hmanlab — OpenAI-compatible relay managed by hmanlab. Users buy
/// access on the hmanlab web app and paste their `sk-` key into the TUI.
/// Billing, quota, and model routing all live server-side; the TUI just
/// sends requests like any other BYOK provider.
///
/// Model list is fetched live from `GET /v1/models` on startup when a key
/// is configured; the seed below is the offline fallback only.
pub const HMANLAB_PROVIDER: &str = "hmanlab";
pub const HMANLAB_BASE: &str = "https://ai.hmanlab.pro/v1";
pub const HMANLAB_MODELS: &[&str] = &[
    "claude-sonnet-4-6",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-haiku-4-5",
    "gpt-5.5",
    "gpt-5.4",
];
pub const HMANLAB_DEFAULT_MODEL: &str = "claude-sonnet-4-6";

/// MiniMax — direct OpenAI-compatible endpoint at `api.minimax.io`.
/// Plan-based billing managed on the MiniMax side; the TUI just sends
/// requests with the user's Bearer key. Slug convention is the raw
/// model id (`MiniMax-M3`, `M2.7`, `M2.6`) — the API expects bare
/// IDs, not namespaced ones.
pub const MINIMAX_PROVIDER: &str = "minimax";
pub const MINIMAX_BASE: &str = "https://api.minimax.io/v1";
pub const MINIMAX_MODELS: &[&str] = &["MiniMax-M3", "M2.7", "M2.6"];
pub const MINIMAX_DEFAULT_MODEL: &str = "MiniMax-M3";

/// Human-readable provider name for UI display. Used by the picker's
/// `+ Add <name> key` rows, the `/disconnect` list, and `/settings`.
/// Falls back to the raw provider id for anything we don't know about
/// (so future additions don't render blank).
pub fn provider_label(provider: &str) -> &str {
    match provider {
        ZAI_SUBSCRIPTION_PROVIDER => "z.ai (subscription)",
        ZAI_USAGE_PROVIDER => "z.ai (usage-based)",
        OLLAMA_CLOUD_PROVIDER => "Ollama Cloud",
        OPENCODE_PROVIDER => "OpenCode Go",
        OPENROUTER_PROVIDER => "OpenRouter",
        HMANLAB_PROVIDER => "hmanlab",
        MINIMAX_PROVIDER => "MiniMax",
        other => other,
    }
}

/// One user-added model from a BYOK provider. Lives in extra_models so the
/// `/model` picker can list it alongside Ollama-discovered models.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExtraModel {
    /// "zai", later "openrouter", etc. Selects which client to use.
    pub provider: String,
    /// Model name as the provider expects it (e.g. "glm-4-plus").
    pub name: String,
}

/// Hard ceiling on the number of specialists a user can configure. Five is
/// arbitrary but generous — Claude Code subagent setups in the wild rarely
/// go past three or four — and capping it keeps the wizard's model picker
/// from sliding into a hierarchy that needs paging.
pub const MAX_SPECIALISTS: usize = 5;

/// One specialist agent: a different model + system prompt the user can
/// invoke via `/ask <name> <query>`. Each is a leaf — specialists do NOT
/// consult other specialists (see [`AgentsConfig`] notes).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SpecialistAgent {
    /// Short slug used in `/ask <name>` and (in phase 2) as the
    /// `consult_specialist` tool argument. Letters/digits/_/- only,
    /// 2-30 chars. Enforced when persisting via the wizard.
    pub name: String,
    /// Model identifier as the provider expects it (e.g. "glm-5.1",
    /// "moonshotai/kimi-k2.6"). Same conventions as `ExtraModel.name`.
    pub model: String,
    /// `None` = local Ollama; `Some("openrouter")` etc. matches the
    /// `ExtraModel.provider` strings, so picker plumbing applies.
    #[serde(default)]
    pub provider: Option<String>,
    /// One-line "use this when…" — shown in `/agents list` and (phase 2)
    /// fed into the `consult_specialist` tool's description so the main
    /// agent knows when to delegate.
    pub task: String,
    /// Full persona instructions delivered as the system prompt for any
    /// `/ask` or `consult_specialist` invocation against this specialist.
    pub system_prompt: String,
    /// Disabled specialists stay in the roster but neither `/ask` nor
    /// the consult tool can reach them. Lets users park a config without
    /// deleting it.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Roster of specialist agents persisted in `~/.config/hmanlab/config.json`
/// under the `agents` block. Session activation is NOT persisted — every
/// TUI launch starts in single-model mode and the user opts in with
/// `/agents on`, by design (no surprise double-spend).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentsConfig {
    #[serde(default)]
    pub specialists: Vec<SpecialistAgent>,
}

impl AgentsConfig {
    /// Look up an enabled specialist by name (case-insensitive on the
    /// name slug). Returns `None` for missing OR disabled — callers
    /// don't need to distinguish; both mean "can't reach this".
    pub fn enabled_by_name(&self, name: &str) -> Option<&SpecialistAgent> {
        let target = name.trim().to_ascii_lowercase();
        self.specialists
            .iter()
            .find(|s| s.enabled && s.name.to_ascii_lowercase() == target)
    }

    /// Mutable counterpart for the enable/disable/edit handlers.
    pub fn by_name_mut(&mut self, name: &str) -> Option<&mut SpecialistAgent> {
        let target = name.trim().to_ascii_lowercase();
        self.specialists
            .iter_mut()
            .find(|s| s.name.to_ascii_lowercase() == target)
    }

    /// True if at least one specialist is in the enabled state. Driven
    /// by `consult_specialist` tool registration in phase 2 — we don't
    /// advertise the tool to the main model if nobody's available.
    #[allow(dead_code)] // wired in phase 2 (consult_specialist tool)
    pub fn any_enabled(&self) -> bool {
        self.specialists.iter().any(|s| s.enabled)
    }
}

/// What's persisted in ~/.config/hmanlab/config.json.
/// BYOK keys live here too — they go to the provider directly, never to the
/// hmanlab backend.
///
/// Legacy `zai_base` from earlier versions is silently ignored (serde drops
/// unknown fields by default). The URL is now derived per-plan from the
/// `ZAI_*_BASE` constants.
///
/// Legacy `api_url` and `api_key` (`bai_` token for hmanlab-stack) are kept
/// as `#[serde(default)]` fields so old config files continue to parse
/// without error; the values are simply ignored at runtime.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Config {
    // Legacy compat: parsed from old config files but never read at
    // runtime (see doc comment above). Kept so deserialization of
    // pre-existing configs doesn't error.
    #[allow(dead_code)]
    #[serde(default, skip_serializing)]
    pub api_url: Option<String>,
    #[allow(dead_code)]
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    pub ollama_host: Option<String>,
    /// z.ai SUBSCRIPTION (coding plan) key — historically called just `zai_api_key`.
    #[serde(default)]
    pub zai_api_key: Option<String>,
    /// z.ai USAGE-BASED key — pay-per-token endpoint.
    #[serde(default)]
    pub zai_usage_api_key: Option<String>,
    /// Ollama Cloud API key (Bearer auth against https://ollama.com).
    /// Independent of any local Ollama host configured via `ollama_host`.
    #[serde(default)]
    pub ollama_cloud_api_key: Option<String>,
    /// OpenCode Zen API key (Bearer auth against opencode.ai/zen/v1).
    /// Same key works for the free tier and the OpenCode Go subscription.
    #[serde(default)]
    pub opencode_api_key: Option<String>,
    /// OpenRouter API key (Bearer auth against openrouter.ai/api/v1).
    /// Generated at https://openrouter.ai/settings/keys.
    #[serde(default)]
    pub openrouter_api_key: Option<String>,
    /// hmanlab API key (`sk-` prefixed). Buy access on the hmanlab web
    /// app; paste the key here to use hmanlab-served models from the TUI.
    #[serde(default)]
    pub hmanlab_api_key: Option<String>,
    /// MiniMax API key (Bearer auth against api.minimax.io/v1).
    /// Plan-based billing managed on the MiniMax side.
    #[serde(default)]
    pub minimax_api_key: Option<String>,
    #[serde(default)]
    pub extra_models: Vec<ExtraModel>,
    /// Absolute workspace paths the user has explicitly authorised. The
    /// agent's destructive tools (write_file, edit_file, run_command,
    /// save_memory, forget_memory) refuse to run outside this list — gated
    /// at the `StreamMsg::ConfirmRequest` interceptor in `app::stream`.
    /// Modeled after Claude Code's per-folder trust prompt.
    #[serde(default)]
    pub trusted_workspaces: Vec<String>,
    /// Last model the user selected (via /model or the picker). Reused on
    /// the next launch so a user who switched to glm-5.1 doesn't get bounced
    /// back to the alphabetically-first Ollama model. Pair with
    /// `last_provider` to disambiguate: the same model name can exist on
    /// multiple BYOK providers (e.g. glm-4.7 on both z.ai plans).
    #[serde(default)]
    pub last_model: Option<String>,
    /// Provider tag for `last_model` — `None` means Ollama, otherwise it
    /// matches one of the `ExtraModel::provider` strings.
    #[serde(default)]
    pub last_provider: Option<String>,
    /// Telegram bot token from @BotFather. When present, the TUI spawns a
    /// long-poll task at startup so the bot can receive DMs. The bot is
    /// local to this TUI process — no hmanlab-api involvement.
    #[serde(default)]
    pub telegram_bot_token: Option<String>,
    /// Telegram user IDs that have completed Pattern-C pairing. Only DMs
    /// from these senders surface in the TUI; everyone else gets a fresh
    /// pair code. Stored as i64 because that's the wire type — Telegram
    /// IDs are immutable across username changes, which is the point.
    #[serde(default)]
    pub telegram_allowlist: Vec<i64>,
    /// When true, paired Telegram users are DM'd a notification (with a
    /// preview of the reply) every time a locally-typed turn finishes
    /// AND the terminal has been idle for at least
    /// [`crate::app::TELEGRAM_IDLE_THRESHOLD`]. Off by default — opt in
    /// via `/telegram notify on`. Telegram-triggered turns already DM
    /// their full reply via the phase-2 bridge, so we skip those.
    #[serde(default)]
    pub telegram_notify_on_idle: bool,
    /// Roster of named specialist agents (see [`AgentsConfig`] +
    /// [`SpecialistAgent`]). The roster persists; session activation
    /// does not (handled per-process on `App.agents_session_enabled`).
    #[serde(default)]
    pub agents: AgentsConfig,

    // ── MCP / web-search ────────────────────────────────────────────────
    /// Which search provider is active. One of "brave", "exa", "tavily",
    /// or "parallel". When set, the `web_search` and `web_fetch` tools are
    /// added to the model's tool surface on every turn.
    #[serde(default)]
    pub mcp_active_provider: Option<String>,
    /// API key for Brave Search (https://api.search.brave.com).
    /// Free tier: 2,000 queries/month.
    #[serde(default)]
    pub brave_search_api_key: Option<String>,
    /// API key for Exa AI (https://exa.ai). Optional — basic tier works
    /// unauthenticated; a key raises rate limits.
    #[serde(default)]
    pub exa_api_key: Option<String>,
    /// API key for Tavily (https://tavily.com). Free tier: 1,000/month.
    #[serde(default)]
    pub tavily_api_key: Option<String>,
}

pub fn path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config/hmanlab/config.json"))
}

pub fn load() -> Result<Option<Config>> {
    let p = path()?;
    if !p.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    let c: Config = serde_json::from_str(&s).context("parse config json")?;
    Ok(Some(c))
}

pub fn save(c: &Config) -> Result<()> {
    let p = path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let s = serde_json::to_string_pretty(c)?;
    std::fs::write(&p, s).with_context(|| format!("write {}", p.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&p)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&p, perms)?;
    }
    Ok(())
}

/// Pre-TUI prompt run on first launch (no providers configured yet).
///
/// Provider selection only — no account, no `bai_` key. Offers all
/// BYOK providers plus local Ollama. Every step is optional; pressing
/// Enter with nothing configured drops the user straight into the TUI
/// where they can use `/model` and `/host` later.
pub async fn run_setup_wizard(existing_ollama: Option<&str>) -> Result<Config> {
    println!();
    println!("\x1b[1mWelcome to hmanlab.\x1b[0m");
    println!();
    println!("The TUI is free — no account required. Connect a provider to start");
    println!("chatting, or skip this wizard and add one later with /model.");
    println!();

    let mut cfg = Config::default();

    println!("Connect a provider? (optional — press Enter to skip)");
    loop {
        let sub_state = if cfg.zai_api_key.is_some() {
            " (configured)"
        } else {
            ""
        };
        let usage_state = if cfg.zai_usage_api_key.is_some() {
            " (configured)"
        } else {
            ""
        };
        let ollama_cloud_state = if cfg.ollama_cloud_api_key.is_some() {
            " (configured)"
        } else {
            ""
        };
        let opencode_state = if cfg.opencode_api_key.is_some() {
            " (configured)"
        } else {
            ""
        };
        let openrouter_state = if cfg.openrouter_api_key.is_some() {
            " (configured)"
        } else {
            ""
        };
        let prox_state = if cfg.hmanlab_api_key.is_some() {
            " (configured)"
        } else {
            ""
        };
        let minimax_state = if cfg.minimax_api_key.is_some() {
            " (configured)"
        } else {
            ""
        };
        let ollama_state = match &cfg.ollama_host {
            Some(h) => format!(" (configured: {h})"),
            None => String::new(),
        };
        println!("  1) z.ai (subscription){sub_state}");
        println!("  2) z.ai (usage-based){usage_state}");
        println!("  3) Ollama Cloud{ollama_cloud_state}");
        println!("  4) OpenCode Go{opencode_state}");
        println!("  5) OpenRouter{openrouter_state}");
        println!("  6) hmanlab (prox){prox_state}");
        println!("  7) MiniMax{minimax_state}");
        println!("  8) local Ollama{ollama_state}");
        print!("  > ");
        io::stdout().flush()?;
        let mut choice = String::new();
        io::stdin().lock().read_line(&mut choice)?;
        let choice = choice.trim();
        if choice.is_empty() || choice.eq_ignore_ascii_case("skip") || choice == "0" {
            break;
        }
        match choice {
            "1" => {
                cfg.zai_api_key = prompt_api_key("z.ai subscription")?;
                if cfg.zai_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m z.ai subscription key saved.");
                }
            }
            "2" => {
                cfg.zai_usage_api_key = prompt_api_key("z.ai usage-based")?;
                if cfg.zai_usage_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m z.ai usage-based key saved.");
                }
            }
            "3" => {
                cfg.ollama_cloud_api_key = prompt_api_key("Ollama Cloud")?;
                if cfg.ollama_cloud_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m Ollama Cloud key saved.");
                }
            }
            "4" => {
                cfg.opencode_api_key = prompt_api_key("OpenCode Go")?;
                if cfg.opencode_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m OpenCode Go key saved.");
                }
            }
            "5" => {
                cfg.openrouter_api_key = prompt_api_key("OpenRouter")?;
                if cfg.openrouter_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m OpenRouter key saved.");
                }
            }
            "6" => {
                cfg.hmanlab_api_key = prompt_api_key("hmanlab (sk-…)")?;
                if cfg.hmanlab_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m hmanlab key saved.");
                }
            }
            "7" => {
                cfg.minimax_api_key = prompt_api_key("MiniMax")?;
                if cfg.minimax_api_key.is_some() {
                    println!("  \x1b[32m✓\x1b[0m MiniMax key saved.");
                }
            }
            "8" => {
                cfg.ollama_host = prompt_ollama_host(existing_ollama)?;
                if cfg.ollama_host.is_some() {
                    println!("  \x1b[32m✓\x1b[0m Ollama URL saved.");
                }
            }
            _ => {
                println!("  (unknown — type 1–8 or Enter to skip)");
            }
        }
        println!();
    }

    save(&cfg)?;
    let p = path()?;
    println!();
    println!("\x1b[32m✓\x1b[0m Saved to {} (mode 600)", p.display());
    println!();
    Ok(cfg)
}

/// Prompt for an API key for any provider. Returns `None` if the user
/// submits an empty line (they decided to skip this provider).
fn prompt_api_key(label: &str) -> Result<Option<String>> {
    print!("    Paste your {label} API key (or Enter to cancel): ");
    io::stdout().flush()?;
    let mut key = String::new();
    io::stdin().lock().read_line(&mut key)?;
    let key = key.trim().to_string();
    if key.is_empty() {
        return Ok(None);
    }
    Ok(Some(key))
}

/// Prompt for the Ollama URL. Returns `None` if the user submits an empty
/// line with no existing default — they decided to skip Ollama after all.
/// If they have a previous setting OR they press Enter when the localhost
/// default is shown, we keep that value.
fn prompt_ollama_host(existing: Option<&str>) -> Result<Option<String>> {
    let default = existing.unwrap_or(DEFAULT_OLLAMA_HOST);
    print!("    Ollama URL [{default}] (or 'skip' to cancel): ");
    io::stdout().flush()?;
    let mut h = String::new();
    io::stdin().lock().read_line(&mut h)?;
    let h = h.trim();
    if h.eq_ignore_ascii_case("skip") {
        return Ok(None);
    }
    if h.is_empty() {
        return Ok(Some(default.to_string()));
    }
    Ok(Some(h.to_string()))
}
