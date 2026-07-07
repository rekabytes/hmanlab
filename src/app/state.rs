//! UI mode + picker-row types. Kept out of `app/mod.rs` so that file
//! stays focused on the `App` struct itself; everything in here is a
//! small, mostly-enum value the input handlers and renderers reach for.

use std::path::PathBuf;

/// Renderer-produced, input-consumed scratch state. The chat and sidebar
/// renderers write geometry and hit-test tables here every frame; the
/// mouse handler reads them on the next event to translate screen
/// coordinates back into clicks on specific messages/files.
///
/// Used to live as 15 loose fields on `App`. Grouping them makes the
/// "renderer is the producer, input is the consumer" relationship
/// explicit and means a future contributor can't forget to reset one of
/// the Vecs at the top of a render pass without it being visibly grouped.
#[derive(Default)]
pub struct RenderState {
    /// Chat panel inner geometry from the last render.
    pub chat_x: u16,
    pub chat_y: u16,
    pub chat_w: u16,
    pub chat_h: u16,
    /// Sidebar panel inner geometry from the last render.
    pub sidebar_x: u16,
    pub sidebar_y: u16,
    pub sidebar_w: u16,
    pub sidebar_h: u16,
    /// One row per visible sidebar entry — `(logical_line_idx, abs_path,
    /// is_dir)`. The click handler converts `screen_y` to a logical
    /// line, looks up the matching entry, and toggles or opens it.
    pub sidebar_targets: Vec<(u16, PathBuf, bool)>,
    /// One row per visible card-style tool tile — `(absolute_screen_y,
    /// message_idx)`. The mouse handler reads this to convert click
    /// rows into expand/collapse toggles on `App::expanded_tools`. The
    /// post-render buffer pass uses it to paint the BG_CARD tile
    /// background under each row. Reset every frame by the chat
    /// renderer so stale rows from a prior frame can't catch a click.
    pub card_row_targets: Vec<(u16, usize)>,
    /// Start indices of every multi-tool group (≥2 consecutive tool
    /// messages) the renderer saw this frame. Populated by
    /// `render_chat`; read by the click handler to decide whether a
    /// click on `card_row_targets` row should toggle
    /// `App::expanded_tool_groups` (group expansion — shows every
    /// member's tile + body) or `App::expanded_tools` (single-tile
    /// body expansion). Reset every frame so a stale entry from a
    /// prior render can't route a click into the wrong set.
    pub multi_group_starts: std::collections::HashSet<usize>,
    /// `(message_idx, logical_line_start_excl, logical_line_end_excl)`
    /// per visible message. The mouse handler converts a click row into
    /// a logical line, then finds the message that owns it.
    pub message_line_ranges: Vec<(usize, u16, u16)>,
    /// Plain-text projection of each rendered chat line. Copy-on-drag
    /// extracts the selection from this without re-parsing styled spans.
    pub rendered_text_lines: Vec<String>,
    /// Last observed mouse cursor position from `MouseEventKind::Moved`.
    pub hover_x: u16,
    pub hover_y: u16,
    /// Inner content width (cols) of the input box from the last render.
    /// Soft-wrap on typed chars uses this to know when to break.
    pub input_inner_w: u16,
    /// Hit-test rect for the "● 1 shell running" status-bar indicator.
    /// Populated each frame by `render_status` only when a shell is in
    /// flight; the mouse handler reads it to translate clicks into
    /// `Mode::ShellMonitor`. Zeroed when no shell is running so a stale
    /// rect from a previous frame can't catch a stray click.
    pub shell_indicator_x: u16,
    pub shell_indicator_y: u16,
    pub shell_indicator_w: u16,

    // ── Sidebar tab hit-test ─────────────────────────────────────────
    /// Absolute screen rects `(x, y, w, h)` for each sidebar tab cell.
    /// Populated by `render_sidebar` each frame; the click handler uses
    /// them to switch the active tab. Zeroed by clearing the rect
    /// vec when the sidebar itself is hidden (width < SIDEBAR_MIN).
    pub sidebar_tab_rects: Vec<(u16, u16, u16, u16)>,

    // ── Permission card hit-test (inline confirm UI) ─────────────────
    // Absolute screen rect for the Approve button, populated each
    // frame by `render_chat` when a permission card is on screen and
    // `pending_confirm.is_some()`. `(x, y, w, h)` — y is post-scroll
    // so the mouse handler doesn't need to know about chat scroll
    // state. Cleared to None when no permission is pending so a
    // stale rect from a prior frame can't catch a stray click.
    pub permission_btn_approve: Option<(u16, u16, u16, u16)>,
    pub permission_btn_deny: Option<(u16, u16, u16, u16)>,
}

/// Which level of the two-level model picker is active.
///
/// Level 1 lists providers (Ollama, z.ai, OpenRouter, …) plus "+"
/// rows for unconnected providers. Level 2 lists the models within
/// the provider the user selected on level 1. Esc on level 2 goes
/// back to level 1; Esc on level 1 closes the picker entirely.
#[derive(Clone, PartialEq, Default)]
pub enum ModelPickerLevel {
    #[default]
    Provider,
    Model,
}

/// What the model picker shows on level 1 — one row per provider group
/// or one "+ Connect" row for unconfigured providers.
#[derive(Clone, Debug)]
pub enum ProviderRow {
    /// An active provider (Ollama local, or a configured BYOK provider).
    /// Carries the provider tag and the model count for the subtitle.
    Active {
        /// `None` = local Ollama, `Some(provider_id)` = BYOK.
        provider: Option<String>,
        label: String,
        model_count: usize,
    },
    /// A provider the user hasn't connected yet. Selecting it opens AddModel.
    Add(String),
}

impl ProviderRow {
    pub fn display(&self) -> String {
        match self {
            ProviderRow::Active {
                label, model_count, ..
            } => {
                format!("{label}  ({model_count})")
            }
            ProviderRow::Add(p) => {
                format!("+ Connect {}", crate::config::provider_label(p))
            }
        }
    }
}

/// Two-level picker entry — used on level 2 (models within a provider).
#[derive(Clone, Debug)]
pub enum ModelRow {
    Ollama(String),
    Extra(crate::config::ExtraModel),
}

/// One row of the agent's plan / todo list. Parsed from the assistant's
/// streamed markdown (`- [ ] …`, `- [x] …`, `- [-] …`) and rendered as
/// a checklist in the inspector pane. `Active` is what the model marks
/// with `[~]` — we treat that as "currently working on" so the row gets
/// the peach accent instead of the dim pending color.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanItem {
    pub text: String,
    pub state: PlanState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanState {
    Pending,
    Active,
    Done,
}

/// Which sidebar view is active. Switched by clicking the tab strip
/// at the top of the sidebar column. Tabs are visual only — the data
/// sources they show (saved sessions, specialist agents) already live
/// on `App`; the tab just selects which one the renderer draws.
///
/// History: the sidebar used to also have a Files tab showing a
/// workspace tree. That was removed when the layout switched to the
/// flat opencode-style sidebar (Sessions + Agents only) — file
/// browsing is now strictly via the file viewer (click a path in
/// chat → opens).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SidebarTab {
    #[default]
    Sessions,
    Agents,
}

impl ModelRow {
    pub fn display(&self) -> String {
        match self {
            ModelRow::Ollama(n) => n.clone(),
            ModelRow::Extra(m) => m.name.clone(),
        }
    }
    pub fn name(&self) -> &str {
        match self {
            ModelRow::Ollama(n) => n.as_str(),
            ModelRow::Extra(m) => m.name.as_str(),
        }
    }
    pub fn provider(&self) -> Option<&str> {
        match self {
            ModelRow::Ollama(_) => None,
            ModelRow::Extra(m) => Some(m.provider.as_str()),
        }
    }
}
#[derive(PartialEq)]
pub enum AppAction {
    Continue,
    Quit,
}

/// Which keymap is active. The dispatcher in `event.rs::handle_event`
/// reads this to route key events to the right handler; the popup
/// renderers in `ui::popups` use it to decide what to draw on top of
/// the chat.
#[derive(Clone, PartialEq)]
pub enum Mode {
    Chat,
    ModelPicker,
    Confirm,
    /// Asking for a BYOK API key (e.g. z.ai). Use `add_step` to track which
    /// step of the add-model flow we're on.
    AddModel,
    /// Listing saved chat sessions; Up/Down navigate, Enter loads the
    /// highlighted session.
    SessionPicker,
    /// Listing currently-connected BYOK providers for removal; Up/Down
    /// navigate, Enter disconnects the highlighted provider, Esc cancels.
    DisconnectPicker,
    /// Two-step Telegram pairing wizard: paste token, then paste the
    /// code the bot DM's you. Step is tracked via
    /// [`TelegramSetupStep`].
    TelegramSetup,
    /// `/agents add` / `/agents edit` wizard — four steps walking the
    /// user through name → model → task description → system prompt.
    /// Step tracked via [`AgentsSetupStep`].
    AgentsSetup,
    /// Live-output viewer for the in-flight (or just-finished) shell
    /// command. Reached by clicking the "● 1 shell running" indicator
    /// in the status bar. Esc returns to chat (shell keeps running);
    /// Ctrl+C kills the child but stays in the monitor so the user can
    /// read the final output before dismissing.
    ShellMonitor,
    /// `/mcp` — web search provider setup. Three-screen wizard:
    ///  1. ProviderList — pick Brave, Exa, Tavily, or Parallel
    ///  2. KeyInput — enter the API key (skipped for key-free providers)
    ///  3. Confirmed — success screen, press Enter/Esc to return to Chat
    McpSetup,
}

/// Live state for the one shell command the agent has in flight (or
/// just finished). Owned by `App.active_shell`. Replaced whenever a
/// new shell starts — tools run serially, so we only ever hold one.
///
/// `kill_tx` is consumed on the first Ctrl+C; further Ctrl+C presses
/// in the monitor are no-ops. The buffer is line-bounded
/// (`MAX_BUFFERED_LINES`) so a chatty `cargo test --nocapture` can't
/// balloon process memory — older lines are dropped from the live
/// view but the final tool result (built from a separate local
/// collector in `tool_run_command`) still carries the byte-truncated
/// archive for the chat tile.
pub struct ShellRuntime {
    pub command: String,
    pub started_at: std::time::Instant,
    /// `(line, is_stderr)` for each chunk the readers emitted. Capped
    /// at [`ShellRuntime::MAX_BUFFERED_LINES`] (ring-trims oldest).
    pub output: Vec<(String, bool)>,
    /// `true` while the child is running. Flipped to `false` on
    /// `ShellDone`. The footer indicator only renders while `true`,
    /// the monitor overlay can still be opened either way.
    pub running: bool,
    /// Exit status once the child has reaped. `None` while running,
    /// `Some(Some(code))` for a normal exit, `Some(None)` if the
    /// process died from a signal without a numeric code.
    pub exit_code: Option<Option<i32>>,
    /// One-shot Sender consumed by the first kill (Ctrl+C in monitor
    /// or workspace-trust block). The receiver lives inside the shell
    /// tool task's `tokio::select!`; firing this races against
    /// `child.wait()` and triggers a `child.kill()`.
    pub kill_tx: Option<tokio::sync::oneshot::Sender<()>>,
    /// Scroll offset into `output` (in display lines), driven by the
    /// monitor overlay's PgUp/PgDn. Reset to 0 (= follow tail) when a
    /// fresh shell starts.
    pub scroll: u16,
    /// `true` while the user wants the viewport pinned to the bottom
    /// of the buffer. Cleared by manual scrolling, re-armed by PgDn
    /// past the last row.
    pub follow_tail: bool,
}

impl ShellRuntime {
    pub const MAX_BUFFERED_LINES: usize = 5000;

    pub fn push_line(&mut self, line: String, is_stderr: bool) {
        if self.output.len() >= Self::MAX_BUFFERED_LINES {
            // Drop the oldest 10% rather than one-at-a-time. Single
            // pops are O(n) on Vec, which would be a real problem
            // under high-throughput output (cargo test, npm install).
            let drop = Self::MAX_BUFFERED_LINES / 10;
            self.output.drain(..drop);
        }
        self.output.push((line, is_stderr));
    }
}

/// Which step of the `/telegram` wizard is showing. Token first, then
/// Pair once getMe succeeds. Modal dismisses itself on a successful
/// pair; failure paths set an error on App and stay on the same step.
#[derive(Clone, Copy, PartialEq)]
pub enum TelegramSetupStep {
    /// Step 1 — paste the `@BotFather` token. Submit triggers async
    /// `getMe`; success advances to `Pair`.
    Token,
    /// Step 2 — wait for the user to DM the bot, then paste the 6-char
    /// code it replied with. Submit redeems via `try_telegram_pair`.
    Pair,
}

/// Generic "list + cursor" pair used by every navigable picker in the
/// UI: model picker, session picker, disconnect picker, …
///
/// Replaces the previous habit of carrying a parallel `<thing>_items:
/// Vec<T>` and `<thing>_index: usize` pair on `App` for each picker.
/// Methods clamp + bounds-check, so callers don't reimplement the
/// "don't crash on empty list" guard every time.
pub struct Picker<T> {
    pub items: Vec<T>,
    pub index: usize,
    /// Scroll offset — first visible row index. Kept in sync with `index`
    /// by `select_next`/`select_prev` so the highlighted row is always on
    /// screen. Callers that set `index` directly must call `clamp_scroll`
    /// afterwards to keep the invariant.
    pub scroll_offset: usize,
}

impl<T> Default for Picker<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            index: 0,
            scroll_offset: 0,
        }
    }
}

impl<T> Picker<T> {
    pub fn set_items(&mut self, items: Vec<T>) {
        self.items = items;
        self.index = 0;
        self.scroll_offset = 0;
    }
    pub fn select_next(&mut self) {
        if self.index + 1 < self.items.len() {
            self.index += 1;
        }
    }
    pub fn select_prev(&mut self) {
        if self.index > 0 {
            self.index -= 1;
        }
    }
    /// Clamp `scroll_offset` so `index` is always within [offset, offset+visible).
    /// Call this after any direct `index` assignment and after rendering to
    /// learn the visible row count.
    pub fn clamp_scroll(&mut self, visible_rows: usize) {
        if visible_rows == 0 {
            return;
        }
        if self.index < self.scroll_offset {
            self.scroll_offset = self.index;
        } else if self.index >= self.scroll_offset + visible_rows {
            self.scroll_offset = self.index + 1 - visible_rows;
        }
    }
    pub fn select_first(&mut self) {
        self.index = 0;
        self.scroll_offset = 0;
    }
    pub fn select_last(&mut self) {
        self.index = self.items.len().saturating_sub(1);
    }
    pub fn selected(&self) -> Option<&T> {
        self.items.get(self.index)
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
}

/// Single-source-of-truth for "what's the agent doing right now?".
/// Replaces the old `generating` / `compacting` bool pair plus the two
/// `JoinHandle` Options and the stranded `pending_after_compact` field
/// — those used to be five fields that callers had to keep in sync
/// manually, and a panicked task could leave the bools out of sync
/// with the handles.
///
/// The three variants are mutually exclusive: an agent turn can't run
/// while a compaction is in flight, and vice versa (each entry point
/// already gated on that via two-bool checks). This enum makes the
/// invariant a type-level fact.
pub enum TurnState {
    /// Nothing in flight. The chat is accepting keypresses and the
    /// renderer skips the streaming-only paths.
    Idle,
    /// An assistant turn is streaming. The contained `JoinHandle` is
    /// the abort target for `cancel()`. Cleared back to `Idle` on
    /// `on_done` / `on_error` / `cancel`.
    Generating { task: tokio::task::JoinHandle<()> },
    /// A `/compact` call (manual or auto-triggered) is running. The
    /// optional `pending_user` is a user message that was buffered
    /// because the compaction was triggered by it — replayed via
    /// `send_to_llm` when the summary arrives.
    Compacting {
        task: tokio::task::JoinHandle<()>,
        pending_user: Option<String>,
    },
}

impl TurnState {
    pub fn is_idle(&self) -> bool {
        matches!(self, TurnState::Idle)
    }
    pub fn is_generating(&self) -> bool {
        matches!(self, TurnState::Generating { .. })
    }
    pub fn is_compacting(&self) -> bool {
        matches!(self, TurnState::Compacting { .. })
    }
    pub fn is_busy(&self) -> bool {
        !self.is_idle()
    }
}

/// Pagination state for `/load`-ed session history. Replaces the
/// `loading_more` + `no_more_history` bool pair so callers can't get
/// into the impossible "loading and exhausted at the same time" state.
///
/// State transitions:
///   - `Idle`     — nothing in flight, more history might exist
///   - `Loading`  — a `/more` request is in flight (manual or auto)
///   - `Exhausted` — server told us no older messages exist
///
/// Resets to `Idle` on `/clear`, `/new`, and `/load` (each starts a
/// fresh pagination cursor).
// Loading/Exhausted + their predicates are scaffolding for the in-flight
// `/more` pagination work; constructed once that lands. Kept to avoid
// churning the enum back and forth.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
pub enum PageState {
    Idle,
    Loading,
    Exhausted,
}

#[allow(dead_code)]
impl PageState {
    pub fn is_loading(self) -> bool {
        matches!(self, PageState::Loading)
    }
    pub fn is_exhausted(self) -> bool {
        matches!(self, PageState::Exhausted)
    }
}

/// Steps in the `/agents add` (and `/agents edit`) wizard. Linear: each
/// Enter advances to the next step; Esc cancels the whole flow. The
/// add path starts at `Template` (pre-fills name/task/prompt from a
/// canned recipe); the edit path skips `Template` and starts at `Name`
/// since you don't re-template-ize an existing specialist.
#[derive(Clone, Copy, PartialEq)]
pub enum AgentsSetupStep {
    /// Step 1 (add only) — pick a canned recipe ("code-reviewer",
    /// "planner", etc.) that pre-fills name/task/prompt. Pick "blank"
    /// to opt out and fill everything by hand.
    Template,
    /// Pick a short slug (`coder`, `reviewer`, etc.). Used by
    /// `/ask <name>` and as the `consult_specialist` argument.
    Name,
    /// Pick a model. Reuses the existing picker entries (Ollama + BYOK)
    /// via a list rendered inline in the wizard.
    Model,
    /// One-line "use this when …" description. Shown in `/agents list`
    /// + (phase 2) fed into the consult tool description.
    Task,
    /// Full system prompt (multi-line textarea allowed).
    Prompt,
}

/// One row in the `/disconnect` picker — a currently-connected BYOK
/// provider plus a short preview of the models that will be removed
/// alongside its API key.
#[derive(Clone)]
pub struct DisconnectEntry {
    /// Provider identifier (e.g. `"zai-subscription"`).
    pub provider: String,
    /// Pretty label shown in the popup (e.g. `"z.ai subscription"`).
    pub label: String,
    /// Three-or-fewer model names + a "+N more" suffix when the provider
    /// seeds a longer catalog. Lets the user see what they're about to
    /// drop before pressing Enter.
    pub preview: String,
}

/// Which screen of the `/mcp` three-step wizard is active.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum McpSetupScreen {
    /// Screen 1 — list of supported providers. Up/Down navigates,
    /// Enter advances to KeyInput (or directly to Confirmed for
    /// key-free providers like Exa and Parallel).
    #[default]
    ProviderList,
    /// Screen 2 — free-text API key entry. Enter saves and advances
    /// to Confirmed; Ctrl+D clears the stored key; Esc goes back to
    /// ProviderList.
    KeyInput,
    /// Screen 3 — success confirmation. Enter or Esc returns to Chat.
    Confirmed,
}
