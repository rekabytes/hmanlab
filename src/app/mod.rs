//! `App` — the central UI state struct. Field is per-feature, not per-tab;
//! everything the renderer needs and every handler mutates lands here.
//!
//! The actual *behavior* (event handling, stream processing, slash
//! commands, workspace/trust state, etc.) lives in submodules; this
//! file is just the struct + constructor + module wiring + a couple of
//! tiny shared helpers (`fresh_textarea`, `seed_sidebar_top_level`).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tui_textarea::TextArea;

use crate::api::Session;
use crate::config::ExtraModel;
use crate::ollama::{Attachment, ChatMessage, Client};
use crate::tools;

mod backend;
mod commands;
pub mod event;
mod heuristics;
pub mod inline;
mod input;
pub mod mcp_providers;
mod plan;
pub mod state;
mod stream;
mod stream_msg;
mod viewer;
pub mod workspace;

pub use backend::LlmBackend;
pub use inline::{InlinePopup, SLASH_COMMANDS};
pub use state::{
    AgentsSetupStep, AppAction, DisconnectEntry, McpSetupScreen, Mode, ModelPickerLevel,
    ModelRow, PageState, Picker, ProviderRow, RenderState, ShellRuntime, TelegramSetupStep,
    TurnState,
};
pub use stream_msg::StreamMsg;
pub use viewer::OpenFile;

/// How long the terminal must sit untouched before a finished local turn
/// counts as "the user walked away" and triggers a Telegram notification
/// (phase 3). 30 s is short enough to fire for a coffee break, long
/// enough that a quick keypress race with `on_done` doesn't spam paired
/// devices.
pub const TELEGRAM_IDLE_THRESHOLD: std::time::Duration = std::time::Duration::from_secs(30);

/// Build a TextArea with no current-line underline (tui-textarea's default
/// behavior is to underline the cursor row, which looks like a stray line in
/// our chat input).
pub(super) fn fresh_textarea() -> TextArea<'static> {
    let mut ta = TextArea::default();
    ta.set_cursor_line_style(ratatui::style::Style::default());
    ta
}

impl App {
    /// Ensure a local JSONL session exists for the current workspace.
    /// Generates a new session ID + path and writes the meta record on
    /// first call per session. No-op if a session is already active.
    pub fn ensure_local_session(&mut self) {
        if self.local_session_id.is_some() {
            return;
        }
        let sid = crate::session::new_session_id();
        let path = crate::session::session_path(&self.workspace, &sid)
            .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/hmanlab-session.jsonl"));
        // Derive title from first visible user message, if any.
        let title = self
            .messages
            .iter()
            .find(|m| m.role == "user" && !m.hidden)
            .map(|m| crate::session::truncate_title(&m.content))
            .unwrap_or_else(|| "New session".to_string());
        let model = self.model.clone();
        let sid_clone = sid.clone();
        let path_clone = path.clone();
        tokio::spawn(async move {
            let _ = crate::session::write_meta(&path_clone, &sid_clone, &title, &model);
        });
        self.local_session_id = Some(sid);
        self.local_session_path = Some(path);
        self.local_session_meta_written = true;
    }

    /// Reset the sidebar state to defaults for the current workspace: clear
    /// any user expansion + reset scroll, then re-seed the expanded set with
    /// the workspace root and its immediate visible directories. Called once
    /// at startup (from `main`) and again whenever `/workspace` switches.
    pub fn seed_sidebar_top_level(&mut self) {
        let trusted = self.workspace_trusted();
        self.expanded_dirs = crate::ui::initial_expanded(&self.workspace, trusted);
        self.sidebar_scroll = 0;
    }

    /// Is the current workspace in the user's trusted list? Looked up
    /// directly each call — O(N) over `trusted_workspaces` which is
    /// usually <10 entries. Used to be a cached `bool` field, but the
    /// cache could drift if a write path forgot to refresh it, and the
    /// lookup is cheap enough that a method is strictly safer.
    pub fn workspace_trusted(&self) -> bool {
        self.trusted_workspaces.iter().any(|p| p == &self.workspace)
    }

    /// Read the BYOK key for a provider (e.g.
    /// `crate::config::ZAI_SUBSCRIPTION_PROVIDER`). Returns `None` if
    /// the user hasn't connected this provider.
    pub fn byok_key(&self, provider: &str) -> Option<&str> {
        self.byok_keys.get(provider).map(String::as_str)
    }

    /// True iff a key for `provider` is stored.
    pub fn has_byok_key(&self, provider: &str) -> bool {
        self.byok_keys.contains_key(provider)
    }

    /// Store / overwrite the BYOK key for a provider. Caller is
    /// responsible for persisting via `persist_config` afterwards.
    pub fn set_byok_key(&mut self, provider: &str, key: String) {
        self.byok_keys.insert(provider.to_string(), key);
    }

    /// Drop the BYOK key for a provider. No-op if unset. Returns the
    /// removed key so the disconnect path can show "removed: …" or use
    /// it for any final cleanup.
    pub fn remove_byok_key(&mut self, provider: &str) -> Option<String> {
        self.byok_keys.remove(provider)
    }
}

pub struct App {
    pub client: Client,
    pub model: String,
    pub models: Vec<String>,
    pub messages: Vec<ChatMessage>,
    pub mode: Mode,
    pub input: TextArea<'static>,
    pub scroll: u16,
    pub follow: bool,
    pub status: String,
    /// "What's the agent doing right now?" — Idle / Generating /
    /// Compacting. Replaces the legacy `generating` / `compacting` /
    /// `current_task` / `compact_task` / `pending_after_compact` field
    /// cluster (5 fields collapsed to 1 enum with type-enforced
    /// mutual exclusion).
    pub turn: TurnState,
    pub workspace: PathBuf,
    pub pending_confirm: Option<tools::ConfirmRequest>,
    /// Armed after the last assistant turn ended with one of the Y/N trigger
    /// phrases. Pressing Y or N silently injects a hidden user reply.
    pub yn_pending: bool,
    /// True for the one turn after a Y-injection. If that turn produces only
    /// an intent announcement ("I'll look at…") with no tool calls, we
    /// auto-inject a continuation prompt. Capped to one retry per Y.
    pub awaiting_yn_followup: bool,
    /// BYOK extras the user has added (z.ai, etc.). Mirrors config.extra_models.
    pub extra_models: Vec<ExtraModel>,
    /// Active extra-provider model, if any. `None` means we're on Ollama.
    /// Tracked separately from `model` so two providers can list a model
    /// with the same name (e.g. `glm-4.7` on both z.ai plans) without
    /// the picker / routing getting confused about which one is active.
    pub selected_extra: Option<ExtraModel>,
    /// All configured BYOK keys, keyed by provider id (matches the
    /// `*_PROVIDER` constants in `crate::config`). Replaces the legacy
    /// per-provider `<x>_api_key: Option<String>` cluster — the picker,
    /// disconnect, settings, and persistence paths now iterate this map
    /// instead of hand-rolling a 5-arm match per provider. Persisted to
    /// disk through `Config`'s per-field shape for backwards compat;
    /// `App::new`'s caller (`main.rs`) does the load-time conversion.
    pub byok_keys: HashMap<String, String>,
    /// Entries rendered by the `/model` picker level 1 (provider list).
    pub model_picker: Picker<ProviderRow>,
    /// Entries rendered by the `/model` picker level 2 (models within provider).
    pub model_picker_models: Picker<ModelRow>,
    /// Which level of the two-level model picker is active.
    pub model_picker_level: ModelPickerLevel,
    /// Provider being added in the current AddModel flow.
    pub add_model_provider: String,
    /// Free-text input for the AddModel modal (key or name).
    pub add_model_input: TextArea<'static>,
    /// `/sessions` picker rows + cursor.
    pub session_picker: Picker<Session>,
    /// Rows shown by the `/disconnect` picker — one per currently-
    /// connected BYOK provider. Rebuilt by `open_disconnect_picker`.
    pub disconnect_picker: Picker<DisconnectEntry>,
    /// Set when `/load` brings in a saved session, so /more knows where to page from.
    pub loaded_session_id: Option<String>,
    pub oldest_loaded_msg_id: Option<i64>,
    /// Local JSONL session — ID of the current live session. `None` until
    /// the first user message of the session is sent.
    pub local_session_id: Option<String>,
    /// Path to the `.jsonl` file for the current live session.
    pub local_session_path: Option<PathBuf>,
    /// True after the meta record has been written for this session.
    pub local_session_meta_written: bool,
    /// Cached summaries from the last `/sessions` list call.
    /// Needed so the session picker's Enter handler can resolve a row back
    /// to its file path without re-scanning disk.
    pub local_session_summaries: Vec<crate::session::SessionSummary>,
    /// Pagination state for session-history loading. Replaces the old
    /// `loading_more` + `no_more_history` bool pair. `Loading` debounces
    /// scroll-triggered auto-loads; `Exhausted` short-circuits the
    /// auto-loader after the server confirmed there's nothing older.
    pub page_state: PageState,
    /// Per-message expand state for tool-result tiles. The chat renderer
    /// shows a tool tile as a one-line header card (`📖 read_file · …  ⌄`)
    /// by default and only inlines the full output if the message index
    /// is in this set. Click a tile in the chat to toggle. The default
    /// collapsed behaviour is intentional — multi-tool turns would
    /// otherwise drown the column in raw output. Mirrors the field name
    /// the previous fold/expand design used (kept for compatibility with
    /// the `card_row_targets` docstring in `state.rs`).
    pub expanded_tools: HashSet<usize>,
    /// Start indices of multi-tool groups the user has clicked to
    /// expand. Distinct from `expanded_tools` (which tracks per-tile
    /// body expansion for single tools) because a multi-tool group has
    /// a different expand affordance: clicking the grouped tile or the
    /// `▼ N tools expanded` header reveals every member's tile + body
    /// inline, not just one. The renderer populates
    /// `RenderState::multi_group_starts` each frame so the click handler
    /// knows which set to toggle for a given row.
    pub expanded_tool_groups: HashSet<usize>,
    /// In-app text selection (since we capture the mouse to get an arrow cursor,
    /// native drag-select is disabled — we re-implement it).
    pub sel_start: Option<(u16, u16)>,
    pub sel_end: Option<(u16, u16)>,
    pub selecting: bool,
    /// Renderer-produced, input-consumed scratch state (geometry,
    /// hit-test tables, hover position). Renderers write here every
    /// frame; mouse / keyboard handlers read on the next event.
    pub render: RenderState,
    /// Running token tally for the current session (resets on /new and /clear).
    pub total_prompt_tokens: u64,
    pub total_completion_tokens: u64,
    /// Prompt-token count from the most recent assistant turn. Drives the
    /// auto-compaction trigger in `send_to_llm` — when this exceeds
    /// [`compact::AUTO_COMPACT_THRESHOLD`], the next user message is
    /// queued behind a compaction pass.
    pub last_prompt_tokens: u32,
    /// Monotonic counter incremented by the ~120 ms ticker in `main::run` while
    /// the agent is generating or a tool is running. Drives the breathing-color
    /// animation in `ui::chat`; stays still when the app is idle.
    pub anim_tick: u64,
    /// Index into `messages` of the tool placeholder currently executing —
    /// `Some` between the `ToolStart` and matching `ToolResult` stream events.
    /// Used by the renderer to apply the breathing style to the running row.
    pub active_tool_msg_idx: Option<usize>,
    /// Wall-clock timestamp of the current in-flight tool call. Stamped in
    /// `on_tool_start`, cleared in `on_tool_result`. Drives the
    /// "elapsed 4.2s" counter on the inspector pane — `active_tool_msg_idx`
    /// alone can't tell us how long a tool has been running.
    pub active_tool_started_at: Option<std::time::Instant>,
    /// Plan / todo list as last parsed from the assistant's most recent
    /// markdown. Each item is `(text, state)` where state is pending /
    /// active / done. The inspector renders this as a checklist; the
    /// plan auto-clears when the user sends the next message (see
    /// `state.rs::clear_plan_on_submit`).
    pub plan: Vec<crate::app::state::PlanItem>,
    /// Which sidebar tab is active. Default is `Files` — the legacy
    /// workspace tree. Sessions lists saved JSONL files for the current
    /// workspace (uses `local_session_summaries`); Agents lists the
    /// configured specialist roster. Tab strip at the top of the
    /// sidebar column is clickable; future keymap bindings (e.g.
    /// Ctrl+1/2/3) would dispatch here.
    pub active_sidebar_tab: crate::app::state::SidebarTab,
    /// In-flight (or just-finished) shell command + its streamed output.
    /// `Some` from `ShellStart` until either the user dismisses the
    /// monitor after `ShellDone`, or a fresh `ShellStart` evicts it.
    /// The footer indicator only renders while `running == true`; the
    /// `Mode::ShellMonitor` overlay is openable as long as this is
    /// `Some`. See [`ShellRuntime`] for the buffer-cap policy.
    pub active_shell: Option<ShellRuntime>,
    /// File the user opened from the sidebar. While `Some`, the file viewer
    /// occupies the chat column and intercepts keys (Esc closes; PgUp/Down
    /// scroll). Cleared when the user closes or quits.
    pub open_file: Option<OpenFile>,
    /// Directories the user has expanded in the sidebar. Workspace root is
    /// pre-seeded so the walker can use a single membership check at every
    /// level. Cleared and re-seeded on `/workspace`.
    pub expanded_dirs: HashSet<PathBuf>,
    /// Logical-line scroll offset for the sidebar (0 = top). Currently
    /// only matters when the Sessions tab overflows the visible area;
    /// reserved so future additions (file-tree view, agent log)
    /// inherit the same scroll plumbing without an App-shape change.
    pub sidebar_scroll: u16,
    pub update_available: Option<String>,
    /// Inline autocomplete popup overlaying the chat surface, if any.
    /// `Slash` when the user is typing `/<command>`, `File` when they're
    /// typing `@<path>`. Mutually exclusive; `None` otherwise.
    pub inline_popup: InlinePopup,
    /// Index of the placeholder `info` message pushed by `/settings` while
    /// the background `fetch_me` request is in flight. When the resolved
    /// `StreamMsg::Settings` arrives, the stream handler edits the message
    /// at this index in place instead of appending a second card — so
    /// "refresh" actually refreshes rather than stacking placeholders.
    pub pending_settings_msg_idx: Option<usize>,
    /// Absolute workspace paths the user has explicitly authorised via
    /// `/trust`. Persisted in `~/.config/hmanlab/config.json`. Use
    /// `App::workspace_trusted()` to check whether the active
    /// workspace is on this list.
    pub trusted_workspaces: Vec<PathBuf>,
    /// Scroll offset (in rendered lines) for the confirm popup body.
    /// Reset to 0 on each new ConfirmRequest; ↑↓/PgUp/PgDn move it in
    /// `handle_confirm`; clamped to a valid max by the renderer.
    pub confirm_scroll: u16,

    /// Telegram bot state, all in one bag because none of it makes sense
    /// in isolation: token without channels is unused, channels without
    /// allowlist let strangers in, etc. `None` until `/telegram setup`
    /// runs (or until startup spawns the bot from a saved token).
    pub telegram: Option<TelegramRuntime>,
    /// Most recent line emitted by the bot task — "bot online", error,
    /// etc. `/telegram status` prints this verbatim. Captured separately
    /// from `telegram` because we want to retain the last error even
    /// after `off` tears down the runtime.
    pub telegram_last_status: Option<String>,
    /// Set when the current in-flight assistant turn was triggered by an
    /// incoming Telegram DM. `on_done` / `on_error` look here to decide
    /// whether to forward the reply back to Telegram. Cleared on
    /// completion, cancel, or compaction reset.
    pub pending_telegram_reply_chat: Option<i64>,
    /// Human-readable label for the Telegram sender currently being
    /// replied to ("@alice"). Surfaces in the status line while we're
    /// generating so the local user knows "this turn is for Telegram".
    pub pending_telegram_reply_from: Option<String>,
    /// Mirror of `Config::telegram_notify_on_idle`. Held on App so the
    /// hot path in `on_done` doesn't have to touch disk to decide
    /// whether to notify.
    pub telegram_notify_on_idle: bool,
    /// Wall-clock timestamp of the most recent local key press. `None`
    /// means no keypress has been observed yet this session (treated as
    /// "not idle" so we don't blast a notification on boot). Stamped in
    /// `handle_event`'s key arm.
    pub last_keypress_at: Option<std::time::Instant>,
    // NOTE: `last_fold_toggle_at` removed when tool / thinking fold
    // toggling was stripped from the chat renderer. Re-introduce if we
    // re-add the debounce alongside any future redesign.
    /// Which step of the `/telegram` wizard is showing while
    /// `mode == Mode::TelegramSetup`. Ignored otherwise.
    pub telegram_setup_step: TelegramSetupStep,
    /// Textarea backing the active step's input (token on step 1, pair
    /// code on step 2). Reset when the modal opens.
    pub telegram_setup_input: TextArea<'static>,
    /// Inline error line rendered under the input on the modal — set
    /// when validation fails (bad token / unknown code) so the user
    /// sees what went wrong without dismissing the wizard.
    pub telegram_setup_error: Option<String>,
    /// True from "user pressed Enter on Token" until `getMe` resolves.
    /// Renderer reads this to swap the input for a "validating…" line
    /// and freeze the keyboard handler against double-submits.
    pub telegram_setup_validating: bool,
    /// Set while a tool wants user confirmation AND the triggering
    /// turn came in over Telegram. Holds the chat we DM'd, the
    /// `message_id` of the prompt (filled in by
    /// `TelegramConfirmSent`), the random `callback_token` encoded
    /// into the button payloads, and the first word of the prompt
    /// (used to extend `telegram_always_allow` when the user picks
    /// the Always button). Cleared whichever side answers first.
    pub pending_telegram_confirm: Option<TelegramConfirmContext>,
    /// Session-only set of "tool head" strings the user has tagged
    /// with Always. Subsequent `on_confirm_request` calls whose
    /// prompt starts with any of these auto-allow silently (with a
    /// DM ack). Not persisted — clears on TUI restart, matching
    /// OpenClaw's "always for this session" semantics.
    pub telegram_always_allow: HashSet<String>,
    /// Roster of named specialist agents loaded from config. The user
    /// edits these via the `/agents` wizard; persistence happens through
    /// `persist_config`. Mirrors `Config.agents` 1-to-1.
    pub agents: crate::config::AgentsConfig,
    /// Session-level enable flag for specialist consultation. False at
    /// boot; `/agents on` flips it. While false, `/ask` refuses and
    /// the (phase 2) `consult_specialist` tool is hidden from the main
    /// agent's tool list. Per-session by design — no surprise double-
    /// spend across restarts.
    pub agents_session_enabled: bool,
    /// Per-specialist (prompt, completion) token totals, keyed by name.
    /// The status line shows a per-agent breakdown whenever this map
    /// has at least one entry. Reset on `/clear` / `/new`.
    pub agent_token_tally: std::collections::HashMap<String, (u64, u64)>,
    /// Which step of the `/agents` wizard is showing while
    /// `mode == Mode::AgentsSetup`. Ignored otherwise.
    pub agents_setup_step: AgentsSetupStep,
    /// Whether the in-flight wizard is editing an existing specialist
    /// (`Some(original_name)`) or adding a new one (`None`). On commit,
    /// `Some` replaces the original entry instead of appending.
    pub agents_setup_editing: Option<String>,
    /// Scratch buffer for the four wizard fields. Filled in step-by-
    /// step as the user advances; persisted only when the user reaches
    /// the final step and presses Enter.
    pub agents_setup_draft: AgentsDraft,
    /// Currently-focused row in the model picker step of the wizard
    /// (Ollama + BYOK lines, both sourced live from `models` +
    /// `extra_models`). Re-clamped on each render.
    pub agents_setup_picker_index: usize,
    /// Free-form textarea used by the Name/Task/Prompt steps. Reset on
    /// every step transition so each input starts clean.
    pub agents_setup_input: TextArea<'static>,
    /// Most recent validation error from the wizard (e.g. duplicate
    /// name, roster full). Rendered as a red inline line and cleared
    /// on the next keystroke.
    pub agents_setup_error: Option<String>,
    /// Name of the specialist currently being addressed by `/ask`.
    /// `Some` while the specialist's task is in flight (controls token
    /// attribution + the "[name] working…" status line), `None`
    /// otherwise. Cleared in `on_done` / `on_error` / `cancel`.
    pub active_specialist: Option<String>,
    /// Image/media attachments queued by `/attach <path>` and waiting to
    /// ride along with the next user submit. Drained into the user
    /// `ChatMessage` in `start_turn`; cleared on `/clear`, `/new`,
    /// and `cancel`. Not persisted to disk — re-attach if reloading.
    pub pending_attachments: Vec<Attachment>,

    // ── MCP / web-search ────────────────────────────────────────────────
    /// Which search provider is currently active, if any. Mirrors
    /// `Config::mcp_active_provider` and is set from that at startup,
    /// then kept up to date by the `/mcp` modal on save.
    pub mcp_active_provider: Option<String>,
    /// API keys for MCP providers that require them, keyed by provider id
    /// (e.g. `"brave"` → `"BSA..."`). Parallel and Exa (basic tier) need
    /// no key and will never appear here.
    pub mcp_keys: std::collections::HashMap<String, String>,
    /// Which screen of the `/mcp` wizard is active. Ignored unless
    /// `mode == Mode::McpSetup`.
    pub mcp_setup_screen: crate::app::state::McpSetupScreen,
    /// Cursor position within the provider list (Screen 1).
    pub mcp_setup_index: usize,
    /// Free-text input for the API key (Screen 2). Reset on every modal open.
    pub mcp_setup_input: tui_textarea::TextArea<'static>,
    /// Validation error shown under the key input. Cleared on each keystroke.
    pub mcp_setup_error: Option<String>,
}

/// Wizard scratch — what the user has typed across the four steps. On
/// commit, this becomes a [`crate::config::SpecialistAgent`]. The
/// `model_provider` field stores `Some(provider_id)` for BYOK and
/// `None` for Ollama, matching `SpecialistAgent.provider`.
#[derive(Clone, Debug, Default)]
pub struct AgentsDraft {
    pub name: String,
    pub model: String,
    pub model_provider: Option<String>,
    pub task: String,
    pub system_prompt: String,
}

/// In-flight Telegram confirm bridge. Built by `on_confirm_request`
/// when the active turn is Telegram-triggered; consumed by
/// `handle_telegram_callback` / `handle_confirm` whichever side fires
/// first.
pub struct TelegramConfirmContext {
    pub chat_id: i64,
    /// `None` between sending the prompt and receiving
    /// `TelegramConfirmSent`. The `editMessageText` path skips itself
    /// when this is still `None` (rare race; the local side will
    /// still push_info either way).
    pub message_id: Option<i64>,
    /// Opaque token encoded into the button payloads. Match against
    /// incoming `TelegramCallback` to confirm this is the prompt
    /// the user tapped (not a stale one from a prior turn).
    pub callback_token: String,
    /// First whitespace-delimited word of `ConfirmRequest.prompt` —
    /// e.g. `"run_command:"` → `"run_command"`. Drives the Always
    /// allowlist match in `on_confirm_request`.
    pub prompt_head: String,
}

/// Live Telegram bot handles. Created by `/telegram setup` (or at
/// startup if a token is already in config) and torn down by
/// `/telegram off`.
pub struct TelegramRuntime {
    /// The active bot token. Held here so `/telegram status` can show
    /// who's online and so a second `setup` knows whether to restart.
    pub token: String,
    /// Bot's @username from `getMe`. Cached so status doesn't have to
    /// re-call the API.
    pub bot_username: Option<String>,
    /// Allowlist of paired Telegram user IDs. Wrapped in `Arc<Mutex>`
    /// because the bot task reads it on every incoming DM and
    /// `/telegram pair` mutates it.
    pub allowlist: std::sync::Arc<std::sync::Mutex<Vec<i64>>>,
    /// Shared pending-codes map (the bot writes; `/telegram pair`
    /// redeems).
    pub pending: crate::telegram::PendingCodes,
    /// Outbound DM channel — `/telegram pair` uses this to send the
    /// "✓ paired" confirmation back to the user.
    pub out_tx: mpsc::UnboundedSender<crate::telegram::TelegramOut>,
    /// Control channel — `/telegram off` sends `Shutdown` to stop the
    /// long-poll task cleanly.
    pub ctl_tx: mpsc::UnboundedSender<crate::telegram::TelegramCtl>,
}

impl App {
    pub fn new(client: Client, model: String, models: Vec<String>, workspace: PathBuf) -> Self {
        let mut input = fresh_textarea();
        input.set_placeholder_text(
            "Type a message, or /help for commands.  (Enter=send, Alt+Enter / Ctrl+J=newline)",
        );
        let status = if models.is_empty() {
            format!(
                "No models — try /host <url> or check Ollama  ·  ws={}",
                workspace.display()
            )
        } else {
            format!(
                "Ready — {} model(s)  ·  ws={}  ·  /help for commands",
                models.len(),
                workspace.display()
            )
        };
        Self {
            client,
            model,
            models,
            messages: Vec::new(),
            mode: Mode::Chat,
            input,
            scroll: 0,
            follow: true,
            status,
            turn: TurnState::Idle,
            workspace,
            pending_confirm: None,
            yn_pending: false,
            awaiting_yn_followup: false,
            extra_models: Vec::new(),
            selected_extra: None,
            byok_keys: HashMap::new(),
            model_picker: Picker::default(),
            model_picker_models: Picker::default(),
            model_picker_level: ModelPickerLevel::Provider,
            add_model_provider: crate::config::ZAI_SUBSCRIPTION_PROVIDER.to_string(),
            add_model_input: fresh_textarea(),
            session_picker: Picker::default(),
            disconnect_picker: Picker::default(),
            loaded_session_id: None,
            oldest_loaded_msg_id: None,
            local_session_id: None,
            local_session_path: None,
            local_session_meta_written: false,
            local_session_summaries: Vec::new(),
            page_state: PageState::Idle,
            sel_start: None,
            sel_end: None,
            selecting: false,
            render: RenderState::default(),
            total_prompt_tokens: 0,
            total_completion_tokens: 0,
            last_prompt_tokens: 0,
            anim_tick: 0,
            active_tool_msg_idx: None,
            active_tool_started_at: None,
            plan: Vec::new(),
            active_sidebar_tab: crate::app::state::SidebarTab::Sessions,
            active_shell: None,
            open_file: None,
            expanded_dirs: HashSet::new(),
            expanded_tools: HashSet::new(),
            expanded_tool_groups: HashSet::new(),
            sidebar_scroll: 0,
            update_available: None,
            inline_popup: InlinePopup::None,
            pending_settings_msg_idx: None,
            trusted_workspaces: Vec::new(),
            confirm_scroll: 0,
            telegram: None,
            telegram_last_status: None,
            pending_telegram_reply_chat: None,
            pending_telegram_reply_from: None,
            telegram_notify_on_idle: false,
            last_keypress_at: None,
            telegram_setup_step: TelegramSetupStep::Token,
            telegram_setup_input: fresh_textarea(),
            telegram_setup_error: None,
            telegram_setup_validating: false,
            pending_telegram_confirm: None,
            telegram_always_allow: HashSet::new(),
            agents: crate::config::AgentsConfig::default(),
            agents_session_enabled: false,
            agent_token_tally: std::collections::HashMap::new(),
            agents_setup_step: AgentsSetupStep::Name,
            agents_setup_editing: None,
            agents_setup_draft: AgentsDraft::default(),
            agents_setup_picker_index: 0,
            agents_setup_input: fresh_textarea(),
            agents_setup_error: None,
            active_specialist: None,
            pending_attachments: Vec::new(),
            mcp_active_provider: None,
            mcp_keys: std::collections::HashMap::new(),
            mcp_setup_screen: McpSetupScreen::ProviderList,
            mcp_setup_index: 0,
            mcp_setup_input: fresh_textarea(),
            mcp_setup_error: None,
        }
    }
}
