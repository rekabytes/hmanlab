//! `StreamMsg` — every event the background tasks send back to the UI.
//!
//! The agent loop, the persistence writer, the `/update` task, etc.
//! all hold an `mpsc::UnboundedSender<StreamMsg>` and push variants
//! through it. `app::stream::handle_stream_msg` consumes them.

use tokio::sync::oneshot;

use crate::agent::SpecialistRunner;
use crate::ollama::{ChatMessage, ToolCall};
use crate::tools;

pub enum StreamMsg {
    Chunk(String),
    Done {
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    Error(String),
    Models {
        models: Vec<String>,
        base: String,
    },
    /// Local JSONL session list loaded from disk.
    LocalSessionList(Vec<crate::session::SessionSummary>),
    /// A local session was loaded from disk — replace visible history.
    LocalSessionLoaded {
        session_id: String,
        title: String,
        model: Option<String>,
        messages: Vec<ChatMessage>,
        path: std::path::PathBuf,
    },
    /// Assistant turn just ended and produced tool calls (the assistant message
    /// content has already been streamed via `Chunk`).
    AssistantTurnEnded {
        tool_calls: Vec<ToolCall>,
    },
    /// Compaction (manual `/compact` or auto-triggered) finished — the
    /// model returned a summary that should replace the visible history.
    CompactionDone {
        summary: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// Compaction failed — surface the error and leave the existing
    /// history untouched.
    CompactionError(String),
    /// Background update check found a newer hmanlab on npm. Renders
    /// as a one-line notice in the header — never blocks anything.
    UpdateAvailable(String),
    /// `/update` finished. `ok` is the exit status; `text` is the
    /// message to surface inline (success summary or failure cause).
    UpdateResult {
        ok: bool,
        text: String,
    },
    /// `/update` interim progress line (e.g., "0.1.4 → 0.1.5, installing…").
    /// Pushed to the chat as an info message so the user can see what
    /// the background task is doing without blocking.
    UpdateInfo(String),
    /// `/settings` finished gathering account + version info. The text
    /// is a pre-formatted multi-line block ready to render verbatim.
    Settings(String),
    /// Begin executing a tool.
    ToolStart {
        name: String,
        args: serde_json::Value,
    },
    /// `run_command` has spawned its child process. Carries the kill
    /// channel so the user can interrupt from `Mode::ShellMonitor`
    /// (Ctrl+C) without aborting the whole agent turn. The App side
    /// stashes this into `active_shell` and renders the footer
    /// indicator. Tools serialise, so this evicts any prior runtime.
    ShellStart {
        command: String,
        kill_tx: oneshot::Sender<()>,
    },
    /// One line of stdout (`is_stderr=false`) or stderr (`is_stderr=true`)
    /// from the running shell. Appended to `active_shell.output` and
    /// dropped if no runtime is active (defensive — shouldn't fire).
    ShellOutput {
        line: String,
        is_stderr: bool,
    },
    /// Child process has reaped. `exit_code` is `Some(n)` for a normal
    /// exit, `None` if it died from a signal. Flips `active_shell.running`
    /// to `false`; the footer indicator disappears but the buffer
    /// stays around so the monitor still shows the final output if open.
    ShellDone {
        exit_code: Option<i32>,
    },
    /// Tool finished — its output replaces the placeholder content on the
    /// trailing `tool` message.
    ToolResult {
        output: String,
    },
    /// Start a fresh assistant placeholder for the next agent turn.
    NewAssistantTurn,
    /// The agent wants the user to confirm a risky action.
    ConfirmRequest(tools::ConfirmRequest),
    /// Live OpenRouter model catalog landed — replaces the static seed.
    /// Already filtered to the popular-vendors set; the stream handler
    /// just swaps them into `extra_models` and persists.
    OpenRouterModelsRefreshed(Vec<String>),
    /// Phase 2 attribution: a `consult_specialist` tool call just
    /// finished; route these tokens into `App.agent_token_tally[name]`
    /// so the per-agent split in the header tracks delegation cost.
    /// Emitted from inside the tool handler via the parent stream tx
    /// snapshot carried on `ToolContext`.
    SpecialistTokens {
        name: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// Phase 3: the main agent's `consult_specialist` tool wants to run
    /// a specialist. The tool handler sends this with a `reply_tx`
    /// `oneshot` and awaits the receiver. App's stream handler spawns
    /// a top-level task that runs the sub-agent, drains its stream into
    /// a single reply string, and sends the result down `reply_tx`.
    ///
    /// This replaces the old recursive `Box::pin + tokio::join!` shape:
    /// the sub-agent is now an independent Tokio task that doesn't
    /// block the parent agent's worker thread. If the parent task is
    /// cancelled, the dropped `reply_rx` causes the spawn task's
    /// `reply_tx.closed()` future to fire, which aborts the sub-agent
    /// — chain cancellation falls out for free.
    SpecialistRequest {
        runner: SpecialistRunner,
        query: String,
        reply_tx: oneshot::Sender<Result<String, String>>,
    },
    /// Background file read for the in-app viewer finished. `display`
    /// matches the open_file's display path; the stream handler swaps
    /// the placeholder content / error in only if the active open_file
    /// still points at the same path (user may have closed or switched
    /// while the read was in flight).
    FileLoaded {
        display: String,
        content: String,
        error: Option<String>,
    },

    /// Incoming Telegram DM from an allowlisted (paired) sender.
    /// The stream handler injects `text` as a user turn in the active
    /// session and stores `chat_id` so the assistant reply lands back in
    /// the originating DM thread (see `App::handle_telegram_message`
    /// and the Telegram dispatch in `on_done` / `on_error`).
    TelegramIncoming {
        chat_id: i64,
        from: String,
        text: String,
    },
    /// One-line lifecycle / error update from the bot task.
    /// Surfaced inline so the user can see "bot online", failures, etc.
    TelegramBotStatus(String),
    /// Sent by the setup task once `getMe` validates the token. The
    /// stream handler stashes `username` on the live runtime so
    /// `/telegram status` can name the bot without re-calling the API.
    TelegramBotReady {
        token: String,
        username: Option<String>,
    },
    /// Confirm-bridge bookkeeping: the bot just delivered a
    /// SendConfirm DM, here's the `message_id` Telegram assigned. The
    /// stream handler folds this into `pending_telegram_confirm` so
    /// the eventual `editMessageText` knows what to overwrite.
    TelegramConfirmSent {
        callback_token: String,
        message_id: i64,
    },
    /// Inline_keyboard tap from a paired Telegram user — the App-side
    /// half of the confirm bridge. `from` is the display handle for
    /// the chat audit line; `from_id` is the numeric Telegram id used
    /// for the (already-validated) allowlist check.
    TelegramCallback {
        callback_token: String,
        action: crate::telegram::CallbackAction,
        chat_id: i64,
        message_id: i64,
        from: String,
        from_id: i64,
    },
}
