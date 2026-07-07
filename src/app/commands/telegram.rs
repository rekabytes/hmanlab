//! `/telegram <sub>` — Pattern-C pairing for the local Telegram bot.
//!
//! Each sub-handler follows the same shape as `commands/host.rs`:
//! validate input, mutate App / persist config / spawn a background
//! task if needed, push an info line so the user sees what happened.

use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tui_textarea::Input;

use super::super::{App, AppAction, Mode, StreamMsg, TelegramRuntime, TelegramSetupStep};
use super::TelegramSub;
use crate::telegram::{self, BotState, CallbackAction, TelegramCtl, TelegramOut};

impl App {
    /// Startup hook: if a Telegram bot token is persisted in
    /// `~/.config/hmanlab/config.json`, re-validate it and spawn the bot
    /// loop. Same path the user takes manually via `/telegram setup`,
    /// just sourced from disk instead of the textarea.
    pub fn boot_telegram(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let saved = crate::config::load().ok().flatten().unwrap_or_default();
        if let Some(token) = saved.telegram_bot_token {
            if !token.trim().is_empty() {
                self.telegram_setup(token, tx);
            }
        }
    }

    /// Treat an allowlisted Telegram DM as a user turn in the active
    /// session. If the TUI can't take a turn right now (no model, busy,
    /// API offline), DMs the sender a short diagnostic instead of going
    /// silent. The assistant's eventual reply is forwarded back to
    /// `chat_id` by `on_done` / `on_error` via `pending_telegram_reply_chat`.
    pub(in crate::app) fn handle_telegram_message(
        &mut self,
        chat_id: i64,
        from: String,
        text: String,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        if text.trim().is_empty() {
            self.send_telegram_dm(chat_id, "(empty message ignored)".into());
            return;
        }
        // HIGHEST priority: if this chat is mid-confirm and the user
        // typed a yes/no reply (legacy fallback for clients that don't
        // render inline keyboards), route to the same path as a button
        // tap. Tapping the buttons is the recommended UX.
        let pending_chat = self.pending_telegram_confirm.as_ref().map(|c| c.chat_id);
        if pending_chat == Some(chat_id) {
            return self.handle_telegram_confirm_reply(chat_id, &from, text.trim());
        }
        // Phase 4: a leading slash means "drive the TUI", not "ask the
        // model". Routed early so commands like /help work even when no
        // model is loaded or the TUI is busy.
        if text.trim_start().starts_with('/') {
            return self.handle_telegram_command(chat_id, from, text, tx);
        }
        // Bail early on conditions `send_to_llm` would silently swallow.
        // Each branch DMs back the reason so the sender isn't stranded.
        if self.models.is_empty() && self.extra_models.is_empty() {
            self.send_telegram_dm(
                chat_id,
                "Hibiscus has no model loaded right now. Try again once a model is configured."
                    .into(),
            );
            return;
        }
        if self.turn.is_busy() {
            self.send_telegram_dm(
                chat_id,
                "Hibiscus is busy on a previous turn — try again in a moment.".into(),
            );
            return;
        }
        if self
            .pending_telegram_reply_chat
            .map(|c| c != chat_id)
            .unwrap_or(false)
        {
            // Different Telegram chat already has a turn in flight.
            // Same race as the busy check above, just with a more
            // accurate message.
            self.send_telegram_dm(
                chat_id,
                "another Telegram chat is mid-conversation with Hibiscus — try again shortly."
                    .into(),
            );
            return;
        }

        // Note who's waiting before kicking off generation; `on_done`
        // reads these to route the reply.
        self.pending_telegram_reply_chat = Some(chat_id);
        self.pending_telegram_reply_from = Some(from.clone());

        // Mark the user message so the local terminal can tell it
        // wasn't typed locally. The LLM sees the same prefix — fine; a
        // sufficiently aware model can address the user by name without
        // any further plumbing.
        let prefixed = format!("[via telegram, from {from}]\n{text}");
        self.send_to_llm(prefixed, tx);

        // If `send_to_llm` failed its own internal checks it would have
        // returned without starting a turn. Treat that as "didn't
        // start" and unwind so we don't strand the pending flags.
        if !self.turn.is_generating() {
            self.pending_telegram_reply_chat = None;
            self.pending_telegram_reply_from = None;
            self.send_telegram_dm(
                chat_id,
                "Hibiscus declined the request — check the terminal for details.".into(),
            );
        }
    }

    /// Legacy text fallback (y/n) for clients that don't render inline
    /// keyboards. Funnels through the same `apply_confirm_decision`
    /// path the button-tap handler uses.
    fn handle_telegram_confirm_reply(&mut self, chat_id: i64, from: &str, text: &str) {
        let answer = parse_yes_no(text);
        let Some(allow) = answer else {
            self.send_telegram_dm(
                chat_id,
                "I'm waiting on a confirmation — tap the buttons above, or reply 'y' / 'n'.".into(),
            );
            return;
        };
        let action = if allow {
            CallbackAction::Allow
        } else {
            CallbackAction::Deny
        };
        self.apply_confirm_decision(action, from, Some(chat_id));
    }

    /// Inline-keyboard tap handler. Called from the stream dispatcher
    /// when a TelegramCallback arrives. Validates the token matches
    /// the currently-pending confirm (drops stale taps), then delegates
    /// to `apply_confirm_decision` for the shared side effects.
    pub(in crate::app) fn handle_telegram_callback(
        &mut self,
        callback_token: String,
        action: CallbackAction,
        chat_id: i64,
        message_id: i64,
        from: String,
        _from_id: i64,
    ) {
        // Validate this tap belongs to the prompt we have on file.
        // Stale taps (user tapped a button from a previous turn after
        // we cleared state) just edit the old message away — don't
        // touch the current confirm.
        let token_matches = self
            .pending_telegram_confirm
            .as_ref()
            .map(|c| c.callback_token == callback_token && c.chat_id == chat_id)
            .unwrap_or(false);
        if !token_matches {
            self.edit_telegram_message(
                chat_id,
                message_id,
                "(This confirm is no longer pending — Hibiscus moved on.)".into(),
            );
            return;
        }
        self.apply_confirm_decision(action, &from, Some(chat_id));
    }

    /// Shared confirm resolution for both the button-tap and text
    /// y/n paths. Pulls the in-flight `ConfirmRequest`, fires its
    /// responder, edits the original Telegram message in place, and
    /// (for `Always`) extends the session allowlist.
    fn apply_confirm_decision(
        &mut self,
        action: CallbackAction,
        from: &str,
        chat_id_hint: Option<i64>,
    ) {
        let ctx = self.pending_telegram_confirm.take();
        let Some(req) = self.pending_confirm.take() else {
            // Race: local user answered between our DM and theirs.
            if let (Some(ctx), Some(_)) = (ctx.as_ref(), chat_id_hint) {
                if let Some(message_id) = ctx.message_id {
                    self.edit_telegram_message(
                        ctx.chat_id,
                        message_id,
                        "(Already answered locally — your reply landed too late.)".into(),
                    );
                } else {
                    self.send_telegram_dm(
                        ctx.chat_id,
                        "Already answered locally — your reply landed too late.".into(),
                    );
                }
            }
            return;
        };

        let allow = match action {
            CallbackAction::Allow | CallbackAction::Always => true,
            CallbackAction::Deny => false,
        };

        // Attach the diff to the running tool placeholder NOW (mirror
        // of the local handle_confirm path) so click-to-expand on the
        // tool row shows what was authorised.
        if allow && !req.diff.is_empty() {
            if let Some(idx) = self.active_tool_msg_idx {
                if let Some(msg) = self.messages.get_mut(idx) {
                    msg.diff = Some(req.diff.clone());
                }
            }
        }

        // Extend the session allowlist BEFORE firing the responder so
        // any immediately-issued confirm for the same head is already
        // auto-allowed by the time on_confirm_request runs.
        let mut always_label = String::new();
        if matches!(action, CallbackAction::Always) {
            if let Some(c) = ctx.as_ref() {
                self.telegram_always_allow.insert(c.prompt_head.clone());
                always_label = format!(" + Always-allow added for `{}`", c.prompt_head);
            }
        }

        let _ = req.responder.send(allow);
        self.mode = crate::app::Mode::Chat;

        // Local audit + remote ack. The Telegram side edits the
        // original message (preferred — keeps history clean) or DMs
        // a fresh ack if we never recorded the message_id.
        let verb = match action {
            CallbackAction::Allow => "✓ Allowed",
            CallbackAction::Always => "✓ Allowed (always for this session)",
            CallbackAction::Deny => "✗ Denied",
        };
        self.push_info(format!(
            "{verb} (via telegram, {from}): {}{always_label}",
            req.prompt
        ));
        if let Some(c) = ctx {
            let body = format!("{verb} by {from}: {}{always_label}", req.prompt);
            if let Some(message_id) = c.message_id {
                self.edit_telegram_message(c.chat_id, message_id, body);
            } else {
                self.send_telegram_dm(c.chat_id, body);
            }
        }
    }

    /// Phase 4 — dispatch a slash-prefixed Telegram DM into the
    /// curated TUI-from-Telegram surface. Parsing routes through the
    /// shared `parse_command` so aliases stay in sync with the local
    /// terminal (`/m` works the same way as `/model`); any `Command`
    /// variant not in the Telegram-allowed match arms gets a friendly
    /// "not supported" reply. The local terminal sees a
    /// `[telegram cmd …]` info line for every dispatch so the user
    /// knows what their bot just did on their behalf.
    fn handle_telegram_command(
        &mut self,
        chat_id: i64,
        from: String,
        text: String,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        let body = text.trim().trim_start_matches('/').trim();
        if body.is_empty() {
            self.send_telegram_dm(chat_id, telegram_help_text());
            return;
        }
        // Audit trail in the local TUI — the user notices when their
        // own bot is operating their session.
        self.push_info(format!("[telegram cmd from {from}] /{body}"));

        let parsed = super::parse_command(&format!("/{body}"));
        match parsed {
            Some(super::Command::Help) => {
                self.send_telegram_dm(chat_id, telegram_help_text());
            }
            Some(super::Command::Model(None)) => {
                self.send_telegram_dm(
                    chat_id,
                    format!("Currently using: {}\nUsage: /model <name>", self.model),
                );
            }
            Some(super::Command::Model(Some(name))) => {
                // `switch_model` mutates state + pushes an info line;
                // we then DM whatever ended up active (which may be
                // unchanged if the name didn't match).
                let before = self.model.clone();
                self.switch_model(&name);
                if self.model == before {
                    self.send_telegram_dm(
                        chat_id,
                        format!(
                            "No change — '{name}' didn't match a unique model. \
                             Run /model on Telegram to see the picker."
                        ),
                    );
                } else {
                    self.send_telegram_dm(chat_id, format!("✓ Switched to model: {}", self.model));
                }
            }
            Some(super::Command::New) => {
                self.new_session();
                self.send_telegram_dm(chat_id, "✓ New Hibiscus session started.".into());
            }
            Some(super::Command::ListSessions) => {
                self.telegram_cmd_sessions(chat_id);
            }
            Some(super::Command::Settings) => {
                self.telegram_cmd_settings(chat_id);
            }
            Some(super::Command::Agents(sub)) => {
                self.telegram_cmd_agents(chat_id, sub);
            }
            Some(super::Command::Ask { name, query }) => {
                self.telegram_cmd_ask(chat_id, from, name, query, tx);
            }
            Some(super::Command::Unknown(other)) => {
                self.send_telegram_dm(
                    chat_id,
                    format!(
                        "/{other} isn't a known command. \
                         Run /help on Telegram to see what's supported."
                    ),
                );
            }
            // Allowed-locally-only commands (Clear, Quit, Host, Load,
            // More, Workspace, Compact, Disconnect, Update, Trust,
            // Untrust, Telegram) — refuse politely. None should be
            // unreachable since body is non-empty by the early return.
            _ => {
                self.send_telegram_dm(
                    chat_id,
                    format!(
                        "/{body} isn't available via Telegram. \
                         Run /help on Telegram to see the supported set."
                    ),
                );
            }
        }
    }

    /// `/sessions` from Telegram — no longer available.
    fn telegram_cmd_sessions(&mut self, chat_id: i64) {
        self.send_telegram_dm(
            chat_id,
            "Session persistence is not available in this version. Use /new to start a fresh session.".into(),
        );
    }

    /// Async `/settings` for Telegram. Mirrors the local `/settings`
    /// card minus tool-running hints (those don't make sense on Telegram).
    fn telegram_cmd_settings(&mut self, chat_id: i64) {
        let current = env!("CARGO_PKG_VERSION");
        let model = self.model.clone();
        let host = self.client.base.clone();
        let ws = self.workspace.display().to_string();
        let byok: Vec<&str> = crate::config::BYOK_PROVIDERS
            .iter()
            .filter(|p| self.has_byok_key(p))
            .map(|p| crate::config::provider_label(p))
            .collect();
        let byok_line = if byok.is_empty() {
            "none".to_string()
        } else {
            byok.join(", ")
        };
        let local = format!(
            "Hibiscus v{current}\n\
             \x20 model       : {model}\n\
             \x20 ollama host : {host}\n\
             \x20 BYOK        : {byok_line}\n\
             \x20 workspace   : {ws}"
        );
        self.send_telegram_dm(chat_id, local);
    }

    /// `/agents` from Telegram — only the read + session-toggle subset.
    /// Add / edit / remove stay local-only because the wizard is modal
    /// and there's no good Telegram analog; users hand-rolling JSON via
    /// DM would be a worse experience than just opening the TUI.
    fn telegram_cmd_agents(&mut self, chat_id: i64, sub: super::AgentsSub) {
        use super::AgentsSub::*;
        match sub {
            Show | List => {
                let body = self.format_agents_for_telegram();
                self.send_telegram_dm(chat_id, body);
            }
            SetEnabled(true) => {
                if self.agents.specialists.is_empty() {
                    self.send_telegram_dm(
                        chat_id,
                        "No specialists configured. Add one from the local TUI \
                         with /agents add, then re-run /agents on."
                            .into(),
                    );
                    return;
                }
                if self.agents_session_enabled {
                    self.send_telegram_dm(
                        chat_id,
                        format!(
                            "Specialist session is already ON.\n\n{}",
                            self.format_agents_for_telegram()
                        ),
                    );
                    return;
                }
                self.agents_session_enabled = true;
                self.push_info("✓ Specialist session enabled via Telegram.".into());
                self.send_telegram_dm(
                    chat_id,
                    format!(
                        "✓ Specialist session enabled.\n\n{}",
                        self.format_agents_for_telegram()
                    ),
                );
            }
            SetEnabled(false) => {
                if !self.agents_session_enabled {
                    self.send_telegram_dm(chat_id, "Specialist session is already OFF.".into());
                    return;
                }
                self.agents_session_enabled = false;
                self.push_info("✓ Specialist session disabled via Telegram.".into());
                self.send_telegram_dm(chat_id, "✓ Specialist session disabled.".into());
            }
            // Roster mutation (add/remove/edit/enable-agent/disable-agent)
            // is intentionally local-TUI-only — too easy to fat-finger a
            // multi-line system prompt over Telegram, and the wizard is
            // the only sane editing surface for that.
            Add | Remove(_) | Edit(_) | SetSpecialistEnabled { .. } | Unknown(_) => {
                self.send_telegram_dm(
                    chat_id,
                    "/agents add/remove/edit/enable-agent/disable-agent aren't available \
                     via Telegram. Use /agents (no arg) for status, or /agents on|off|list. \
                     Edit the roster from the local TUI."
                        .into(),
                );
            }
        }
    }

    /// Plain-text roster for DM rendering. Mirrors `format_roster` from
    /// the local handler but in a Telegram-friendly layout (no leading
    /// "Roster:" header — the section context is the DM itself).
    fn format_agents_for_telegram(&self) -> String {
        let session = if self.agents_session_enabled {
            "ON"
        } else {
            "OFF"
        };
        let n = self.agents.specialists.len();
        let max = crate::config::MAX_SPECIALISTS;
        if n == 0 {
            return format!(
                "Specialists — 0/{max} configured · session: {session}\n\n\
                 No specialists yet. Add from the local TUI with /agents add."
            );
        }
        let mut s = format!("Specialists — {n}/{max} configured · session: {session}\n");
        for spec in &self.agents.specialists {
            let mark = if spec.enabled { "●" } else { "○" };
            let provider = spec.provider.clone().unwrap_or_else(|| "ollama".into());
            s.push_str(&format!(
                "  {mark} {name} [{provider}] {model} · {task}\n",
                name = spec.name,
                provider = provider,
                model = spec.model,
                task = spec.task,
            ));
        }
        if !self.agents_session_enabled {
            s.push_str("\n(Session is OFF — /agents on to enable /ask via Telegram.)\n");
        }
        s
    }

    /// `/ask <name> <query>` from Telegram. Validates preconditions
    /// up-front so the sender always gets a DM diagnostic on refusal
    /// (otherwise the local push_info would be the only signal). Once
    /// the specialist's loop starts, the existing `pending_telegram_reply_chat`
    /// + `on_done` bridge auto-forwards the consolidated reply back to
    ///   this chat — same path a normal Telegram→model turn uses.
    fn telegram_cmd_ask(
        &mut self,
        chat_id: i64,
        from: String,
        name: String,
        query: String,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        if name.is_empty() {
            self.send_telegram_dm(
                chat_id,
                "Usage: /ask <name> <query>   (run /agents to see the roster)".into(),
            );
            return;
        }
        if query.is_empty() {
            self.send_telegram_dm(chat_id, format!("Usage: /ask {name} <query>"));
            return;
        }
        if !self.agents_session_enabled {
            self.send_telegram_dm(
                chat_id,
                "Specialist session is OFF. Run /agents on first (per-session opt-in).".into(),
            );
            return;
        }
        if self.agents.enabled_by_name(&name).is_none() {
            self.send_telegram_dm(
                chat_id,
                format!("No enabled specialist named '{name}'. Run /agents to see the roster."),
            );
            return;
        }
        if self.turn.is_busy() {
            self.send_telegram_dm(
                chat_id,
                "Hibiscus is busy on a previous turn — try again in a moment.".into(),
            );
            return;
        }
        if self
            .pending_telegram_reply_chat
            .map(|c| c != chat_id)
            .unwrap_or(false)
        {
            self.send_telegram_dm(
                chat_id,
                "another Telegram chat is mid-conversation with Hibiscus — try again shortly."
                    .into(),
            );
            return;
        }

        // Park the reply route BEFORE handle_ask so the spawned agent
        // task can race past us without us missing the on_done flush.
        self.pending_telegram_reply_chat = Some(chat_id);
        self.pending_telegram_reply_from = Some(from);
        self.handle_ask(name, query, tx);
        if !self.turn.is_generating() {
            // handle_ask refused (missing API key, etc.) — push_info
            // would have explained on the local side; mirror it to
            // Telegram so the sender isn't stranded.
            self.pending_telegram_reply_chat = None;
            self.pending_telegram_reply_from = None;
            self.send_telegram_dm(
                chat_id,
                "Couldn't kick off the specialist — check the local TUI for details.".into(),
            );
        }
    }

    /// Send a one-shot plain-text DM via the running bot. No-op if the
    /// bot isn't up (the user must have run `/telegram off` mid-flow);
    /// the loss is logged via `telegram_last_status` so `/telegram
    /// status` shows it. Use for bot-authored messages (status, acks,
    /// pair confirmations) — characters render literally.
    pub(in crate::app) fn send_telegram_dm(&mut self, chat_id: i64, text: String) {
        match &self.telegram {
            Some(rt) => {
                if rt.out_tx.send(telegram::dm(chat_id, text)).is_err() {
                    self.telegram_last_status =
                        Some("telegram: DM channel closed — bot already shut down".into());
                }
            }
            None => {
                self.telegram_last_status =
                    Some("telegram: tried to DM with no bot configured".into());
            }
        }
    }

    /// Send an HTML-formatted DM. `text` must already be the output of
    /// `telegram::format_for_telegram` (or otherwise valid Telegram HTML)
    /// — Telegram rejects unbalanced or unescaped tags with 400.
    /// Used for AI replies so `**bold**`, backtick code, and triple-
    /// fence blocks render natively instead of leaking markdown syntax.
    pub(in crate::app) fn send_telegram_html(&mut self, chat_id: i64, text: String) {
        match &self.telegram {
            Some(rt) => {
                if rt.out_tx.send(telegram::dm_html(chat_id, text)).is_err() {
                    self.telegram_last_status =
                        Some("telegram: DM channel closed — bot already shut down".into());
                }
            }
            None => {
                self.telegram_last_status =
                    Some("telegram: tried to DM with no bot configured".into());
            }
        }
    }

    /// Edit a previously-sent message in place — used to swap the
    /// confirm prompt with its resolved state ("✓ Allowed by …").
    /// No-op if the bot isn't running.
    pub(in crate::app) fn edit_telegram_message(
        &mut self,
        chat_id: i64,
        message_id: i64,
        text: String,
    ) {
        if let Some(rt) = &self.telegram {
            let _ = rt.out_tx.send(telegram::TelegramOut::EditMessage {
                chat_id,
                message_id,
                text,
            });
        }
    }

    pub(in crate::app) fn handle_telegram(
        &mut self,
        sub: TelegramSub,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        match sub {
            // `setup` with a token → CLI fast path. `setup` with no
            // token → opens the wizard so the user can paste it in a
            // dedicated input rather than retyping the command.
            TelegramSub::Setup(token) if !token.trim().is_empty() => self.telegram_setup(token, tx),
            TelegramSub::Setup(_) => self.open_telegram_setup(),
            // Same for `pair`: with a code → run; without → modal.
            TelegramSub::Pair(code) if !code.trim().is_empty() => self.telegram_pair(code),
            TelegramSub::Pair(_) => self.open_telegram_setup(),
            TelegramSub::Status => self.telegram_status(),
            TelegramSub::Unpair => self.telegram_unpair(),
            TelegramSub::Off => self.telegram_off(),
            TelegramSub::Notify(set) => self.telegram_notify(set),
            // Bare `/telegram` → wizard if any setup is unfinished,
            // otherwise show status (don't pop a modal at someone
            // who's already configured + paired).
            TelegramSub::Help => self.open_telegram_setup(),
        }
    }

    /// Pop the `/telegram` wizard at the right step for current state:
    ///   - no bot token configured           → start at Token
    ///   - bot online but allowlist is empty → start at Pair
    ///   - bot online and at least one user paired → no modal, just
    ///     show status inline (the user is already done; if they want
    ///     to add a second device they can DM the bot and run
    ///     `/telegram pair <code>` directly).
    pub(in crate::app) fn open_telegram_setup(&mut self) {
        let starting = match self.telegram.as_ref() {
            None => super::super::TelegramSetupStep::Token,
            Some(rt) => {
                let empty = rt.allowlist.lock().map(|l| l.is_empty()).unwrap_or(true);
                if empty {
                    super::super::TelegramSetupStep::Pair
                } else {
                    // Already configured and paired — fall back to
                    // the inline status card.
                    return self.telegram_status();
                }
            }
        };
        self.telegram_setup_step = starting;
        self.telegram_setup_input = super::super::fresh_textarea();
        self.telegram_setup_input
            .set_placeholder_text(match starting {
                super::super::TelegramSetupStep::Token => "Paste your @BotFather token here",
                super::super::TelegramSetupStep::Pair => "Paste the 6-char code the bot DM'd you",
            });
        self.telegram_setup_error = None;
        self.telegram_setup_validating = false;
        self.mode = super::super::Mode::TelegramSetup;
        self.status = match starting {
            super::super::TelegramSetupStep::Token => {
                "Telegram setup — paste bot token  ·  Esc to cancel".into()
            }
            super::super::TelegramSetupStep::Pair => {
                "Telegram setup — DM the bot, then paste the code  ·  Esc to cancel".into()
            }
        };
    }

    /// Wizard keyboard handler. Esc cancels (returns to Chat).
    /// Enter on Token submits → spawns getMe; we stay on the step
    /// (locked by `telegram_setup_validating`) until the stream
    /// handler advances or surfaces an error.
    /// Enter on Pair submits → synchronous redeem; success dismisses,
    /// failure stays on Pair with the error rendered.
    pub(in crate::app) fn handle_telegram_setup_key(
        &mut self,
        key: KeyEvent,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        if key.code == KeyCode::Esc {
            self.mode = Mode::Chat;
            self.telegram_setup_error = None;
            self.telegram_setup_validating = false;
            self.status = "Telegram setup cancelled".into();
            return AppAction::Continue;
        }
        // Block input while a validation round-trip is in flight so the
        // user can't double-submit a second token before getMe returns.
        if self.telegram_setup_validating {
            return AppAction::Continue;
        }
        if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
            let val = self
                .telegram_setup_input
                .lines()
                .join("")
                .trim()
                .to_string();
            if val.is_empty() {
                self.telegram_setup_error =
                    Some("(empty — paste something, or Esc to cancel)".into());
                return AppAction::Continue;
            }
            self.telegram_setup_error = None;
            match self.telegram_setup_step {
                TelegramSetupStep::Token => {
                    self.telegram_setup_validating = true;
                    // Reuse the existing setup path. It pushes its own
                    // info lines + emits TelegramBotReady on success or
                    // a "setup failed" status on error; the stream
                    // handler advances the modal off of those signals.
                    self.telegram_setup(val, tx);
                }
                TelegramSetupStep::Pair => match self.try_telegram_pair(&val) {
                    Ok(msg) => {
                        // Success — drop the modal, leave a confirmation
                        // line in the chat so the user sees what happened.
                        self.mode = Mode::Chat;
                        self.telegram_setup_input = super::super::fresh_textarea();
                        self.telegram_setup_error = None;
                        self.push_info(msg);
                        self.status = "Telegram paired".into();
                    }
                    Err(msg) => {
                        // Stay on Pair so the user can DM the bot for
                        // a fresh code and try again without re-opening
                        // the wizard.
                        self.telegram_setup_error = Some(msg);
                        self.telegram_setup_input = super::super::fresh_textarea();
                        self.telegram_setup_input
                            .set_placeholder_text("Paste the 6-char code the bot DM'd you");
                    }
                },
            }
            return AppAction::Continue;
        }
        // Anything else falls through to the textarea.
        let input: Input = key.into();
        self.telegram_setup_input.input(input);
        AppAction::Continue
    }

    fn telegram_notify(&mut self, set: Option<bool>) {
        match set {
            None => {
                let state = if self.telegram_notify_on_idle {
                    "ON"
                } else {
                    "OFF"
                };
                self.push_info(format!(
                    "Telegram idle notifications: {state}\n\
                     Threshold: {}s of no key presses before a finished local turn DMs paired users.\n\
                     /telegram notify on  — enable\n\
                     /telegram notify off — disable",
                    super::super::TELEGRAM_IDLE_THRESHOLD.as_secs(),
                ));
            }
            Some(want) => {
                let was = self.telegram_notify_on_idle;
                self.telegram_notify_on_idle = want;
                let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
                cfg.telegram_notify_on_idle = want;
                if let Err(e) = crate::config::save(&cfg) {
                    // Roll the in-memory flag back so the user's view
                    // stays consistent with what's actually persisted.
                    self.telegram_notify_on_idle = was;
                    self.push_info(format!("Couldn't persist notify preference: {e}"));
                    return;
                }
                if want {
                    self.push_info(
                        "✓ Telegram idle notifications enabled. Paired users will be DM'd \
                         when a local turn finishes after the terminal sits idle."
                            .into(),
                    );
                } else {
                    self.push_info("✓ Telegram idle notifications disabled.".into());
                }
            }
        }
    }

    fn telegram_setup(&mut self, token: String, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let token = token.trim().to_string();
        if token.is_empty() {
            self.push_info("Usage: /telegram setup <bot-token>".into());
            return;
        }

        // Validate via getMe, then spawn the bot. Done as a tokio task
        // so the TUI stays responsive; result lands back through the
        // stream channel as either TelegramBotStatus (success/restart
        // notice) or a regular error info line.
        let tx_clone = tx.clone();
        let token_clone = token.clone();

        // If a bot is already running with a different token, tear it
        // down first so we don't end up with two long-poll loops
        // fighting over getUpdates' single-consumer guarantee.
        let replacing_existing = self
            .telegram
            .as_ref()
            .map(|rt| rt.token != token)
            .unwrap_or(false);
        if replacing_existing {
            if let Some(rt) = self.telegram.take() {
                let _ = rt.ctl_tx.send(TelegramCtl::Shutdown);
            }
        } else if let Some(rt) = &self.telegram {
            // Same token already configured — just confirm.
            let uname = rt
                .bot_username
                .clone()
                .unwrap_or_else(|| "(unknown)".into());
            self.push_info(format!(
                "Telegram bot already running as @{uname}. Use /telegram status to inspect."
            ));
            return;
        }

        // Carry forward the existing allowlist (e.g. if a previous
        // `setup` ran in the same session, we don't want to forget who
        // was paired). For first-ever setup the list is empty.
        let saved = crate::config::load().ok().flatten().unwrap_or_default();
        let allowlist = Arc::new(Mutex::new(saved.telegram_allowlist.clone()));
        let pending = telegram::new_pending_map();
        let (out_tx, out_rx) = mpsc::unbounded_channel::<TelegramOut>();
        let (ctl_tx, ctl_rx) = mpsc::unbounded_channel::<TelegramCtl>();

        self.push_info("Validating Telegram token…".into());
        self.status = "Telegram: validating token…".into();

        let allowlist_for_task = Arc::clone(&allowlist);
        let pending_for_task = Arc::clone(&pending);

        tokio::spawn(async move {
            let info = match telegram::get_me(&token_clone).await {
                Ok(info) => info,
                Err(e) => {
                    let _ = tx_clone.send(StreamMsg::TelegramBotStatus(format!(
                        "telegram: setup failed — {e}"
                    )));
                    return;
                }
            };
            // Persist the token only after validation succeeds, so a
            // bogus paste doesn't leave a junk value on disk.
            let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
            cfg.telegram_bot_token = Some(token_clone.clone());
            if let Err(e) = crate::config::save(&cfg) {
                let _ = tx_clone.send(StreamMsg::TelegramBotStatus(format!(
                    "telegram: setup ok, but persisting token failed — {e}"
                )));
            }
            let uname = info.username.clone().unwrap_or_else(|| "(unknown)".into());
            let _ = tx_clone.send(StreamMsg::TelegramBotReady {
                token: token_clone.clone(),
                username: info.username,
            });
            let _ = tx_clone.send(StreamMsg::TelegramBotStatus(format!(
                "telegram: connected as @{uname}. DM the bot to start pairing."
            )));
            telegram::run(
                BotState {
                    token: token_clone,
                    allowlist: allowlist_for_task,
                    pending: pending_for_task,
                },
                tx_clone,
                out_rx,
                ctl_rx,
            )
            .await;
        });

        // Park the half-built runtime now so subsequent `/telegram
        // status` reflects the in-flight setup. The username gets filled
        // in by the stream handler once TelegramBotReady arrives.
        self.telegram = Some(TelegramRuntime {
            token,
            bot_username: None,
            allowlist,
            pending,
            out_tx,
            ctl_tx,
        });
    }

    /// CLI-form `/telegram pair <code>`. Thin wrapper that funnels both
    /// success and failure into `push_info`. The modal (which wants to
    /// branch on outcome) uses [`Self::try_telegram_pair`] directly.
    fn telegram_pair(&mut self, code: String) {
        match self.try_telegram_pair(&code) {
            Ok(msg) | Err(msg) => self.push_info(msg),
        }
    }

    /// Core pair logic — usable from both the inline command path and
    /// the wizard. Returns the user-visible result either way: the
    /// caller decides whether to `push_info` (CLI) or render in a modal
    /// (wizard).
    pub(in crate::app) fn try_telegram_pair(&mut self, code: &str) -> Result<String, String> {
        let code = code.trim();
        if code.is_empty() {
            return Err("Usage: /telegram pair <code>".into());
        }
        let Some(rt) = &self.telegram else {
            return Err("Telegram bot isn't running. Run /telegram setup <token> first.".into());
        };
        let Some(tg_id) = telegram::redeem_code(&rt.pending, code) else {
            return Err(format!(
                "No pending code matches '{code}' (or it expired). DM the bot again to get a fresh one."
            ));
        };

        // Add to allowlist (in-memory + persisted). The bot reads under
        // the same mutex so this takes effect on the very next DM.
        let already_paired = {
            let mut list = rt.allowlist.lock().expect("telegram allowlist mutex");
            if list.contains(&tg_id) {
                true
            } else {
                list.push(tg_id);
                false
            }
        };

        // Persist. We re-load + re-save to avoid clobbering anything else
        // that may have mutated config since startup (BYOK keys, last
        // model, etc.).
        let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
        if !cfg.telegram_allowlist.contains(&tg_id) {
            cfg.telegram_allowlist.push(tg_id);
        }
        if let Err(e) = crate::config::save(&cfg) {
            return Err(format!(
                "Paired with Telegram id {tg_id}, but persisting the allowlist failed: {e}"
            ));
        }

        let hostname = hostname_for_greeting();
        let _ = rt.out_tx.send(telegram::dm(
            tg_id,
            if already_paired {
                format!("Already paired with hmanlab on {hostname}.")
            } else {
                format!("✓ Paired with Hibiscus on {hostname}.")
            },
        ));

        if already_paired {
            Ok(format!("Telegram id {tg_id} is already paired."))
        } else {
            Ok(format!("✓ Paired with Telegram id {tg_id}."))
        }
    }

    fn telegram_status(&mut self) {
        let Some(rt) = &self.telegram else {
            self.push_info(
                "Telegram bot is not configured. /telegram setup <token> to start.".into(),
            );
            return;
        };
        let uname = rt
            .bot_username
            .clone()
            .unwrap_or_else(|| "(handshake in progress)".into());
        let allow_count = rt.allowlist.lock().map(|l| l.len()).unwrap_or(0);
        let pending_count = rt.pending.lock().map(|p| p.len()).unwrap_or(0);
        let last = self
            .telegram_last_status
            .clone()
            .unwrap_or_else(|| "(no status yet)".into());
        let notify = if self.telegram_notify_on_idle {
            "ON"
        } else {
            "OFF"
        };
        self.push_info(format!(
            "Telegram bot @{uname}\n\
             \x20 paired users: {allow_count}\n\
             \x20 pending codes: {pending_count}\n\
             \x20 idle notify:  {notify}\n\
             \x20 last status:  {last}"
        ));
    }

    fn telegram_unpair(&mut self) {
        let cleared = match &self.telegram {
            Some(rt) => {
                let mut list = rt.allowlist.lock().expect("telegram allowlist mutex");
                let n = list.len();
                list.clear();
                n
            }
            None => 0,
        };
        let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
        cfg.telegram_allowlist.clear();
        if let Err(e) = crate::config::save(&cfg) {
            self.push_info(format!(
                "Cleared allowlist in memory, but persisting failed: {e}"
            ));
            return;
        }
        self.push_info(format!(
            "Cleared {cleared} paired Telegram user(s). Bot is still running — new DMs will get a fresh pair code."
        ));
    }

    fn telegram_off(&mut self) {
        let was_running = self.telegram.is_some();
        if let Some(rt) = self.telegram.take() {
            let _ = rt.ctl_tx.send(TelegramCtl::Shutdown);
        }
        let mut cfg = crate::config::load().ok().flatten().unwrap_or_default();
        cfg.telegram_bot_token = None;
        cfg.telegram_allowlist.clear();
        if let Err(e) = crate::config::save(&cfg) {
            self.push_info(format!(
                "Stopped the bot, but persisting the cleared config failed: {e}"
            ));
            return;
        }
        if was_running {
            self.push_info("Telegram bot stopped; token and allowlist cleared.".into());
        } else {
            self.push_info("Telegram bot wasn't running. Config cleared anyway.".into());
        }
    }
}

/// Best-effort host label for the "✓ paired with hmanlab on <hostname>"
/// DM. Falls back to "this machine" when the env var is unset — the
/// confirmation still works, it's just less specific.
fn hostname_for_greeting() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "this machine".to_string())
}

/// Loose y/n parser for confirm-bridge replies. Matches Telegram chat
/// norms (people type "yes please", "no thanks") rather than a strict
/// single letter. Returns `None` for anything ambiguous so the caller
/// can re-prompt instead of guessing.
fn parse_yes_no(text: &str) -> Option<bool> {
    let t = text.trim().to_ascii_lowercase();
    let head = t.split_whitespace().next().unwrap_or("");
    // Strip trailing punctuation so "yes!" / "no." still parse.
    let head = head.trim_end_matches(|c: char| !c.is_alphanumeric());
    match head {
        "y" | "yes" | "yeah" | "yep" | "ok" | "okay" | "sure" | "allow" | "approve" | "accept" => {
            Some(true)
        }
        "n" | "no" | "nope" | "nah" | "deny" | "reject" | "decline" | "cancel" => Some(false),
        _ => None,
    }
}

/// Cheat-sheet DM'd back when the Telegram user sends `/help` (or `/`
/// with no body). Lists ONLY the commands actually wired in
/// `handle_telegram_command` — anything missing from here would fall
/// through to the "not available via Telegram" branch.
fn telegram_help_text() -> String {
    "Hibiscus via Telegram — commands:\n\
     \x20 /help              this list\n\
     \x20 /sessions          recent saved sessions\n\
     \x20 /new               start a fresh session\n\
     \x20 /model             open model picker\n\
     \x20 /model <name>      switch Ollama model (BYOK switch is local-only)\n\
     \x20 /settings          account + version snapshot\n\
     \x20 /agents            specialist roster + session state\n\
     \x20 /agents on|off     flip specialist session activation\n\
     \x20 /ask <name> <q>    manually invoke a specialist (run /agents on first)\n\
     \n\
     Any non-slash message is sent to the active model as a normal turn; \
     the assistant's reply lands back here when it finishes."
        .into()
}

#[cfg(test)]
mod yes_no_tests {
    use super::parse_yes_no;

    #[test]
    fn accepts_common_yes_forms() {
        for s in [
            "y", "Y", "yes", "Yes!", "YES.", "ok", "okay", "sure", "yep", "yeah", "Allow",
            "approve", "accept",
        ] {
            assert_eq!(parse_yes_no(s), Some(true), "expected Some(true) for {s:?}");
        }
    }

    #[test]
    fn accepts_common_no_forms() {
        for s in [
            "n", "N", "no", "No.", "NO!", "nope", "nah", "deny", "reject", "decline", "cancel",
        ] {
            assert_eq!(
                parse_yes_no(s),
                Some(false),
                "expected Some(false) for {s:?}"
            );
        }
    }

    #[test]
    fn rejects_ambiguous_text() {
        // Anything whose first word isn't in our yes/no vocab → None.
        // (We deliberately accept "ok?" as yes — punctuation gets
        // stripped — so "ok? not sure" still parses to true. That's
        // the intended trade-off: humans rarely type a bare "ok?" to
        // mean "I'm uncertain.")
        for s in ["maybe", "?", "y/n", "", "   ", "hmm yes"] {
            assert_eq!(parse_yes_no(s), None, "expected None for {s:?}");
        }
    }

    #[test]
    fn picks_first_word() {
        // "yes please" -> first word "yes" parses true. "no thanks"
        // -> first word "no" parses false.
        assert_eq!(parse_yes_no("yes please"), Some(true));
        assert_eq!(parse_yes_no("no thanks"), Some(false));
    }
}
