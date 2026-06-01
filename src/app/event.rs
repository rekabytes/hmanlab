//! Top-level event dispatcher + the helpers that turn slash commands and
//! agent IO into chat state and outbound LLM calls.
//!
//! Input handling proper lives in `app/input/` — this file only routes
//! events to the right handler based on `self.mode` and `event` kind.
//! The slash-command dispatch (`handle_command`) and the chat-state
//! plumbing (`push_info`, `reset_input`, `inject_hidden_user`,
//! `send_to_llm`, `cancel`, `start_compact`) live here too because they
//! straddle the input/output boundary — they're called from input but
//! kick off the agent loop and stream-event pipeline.

use anyhow::Result;
use crossterm::event::{Event, KeyEventKind};
use tokio::sync::mpsc;

use crate::ollama::ChatMessage;

use super::commands::Command;
use super::{fresh_textarea, App, AppAction, Mode, StreamMsg, TurnState};

impl App {
    pub async fn handle_event(
        &mut self,
        event: Event,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> Result<AppAction> {
        match event {
            Event::Mouse(m) => {
                self.handle_mouse(m, tx);
                Ok(AppAction::Continue)
            }
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    return Ok(AppAction::Continue);
                }
                // Phase-3 idle tracking: any key press resets the
                // "user walked away" clock. Stamping here, before any
                // mode-specific handler, means every key — picker
                // navigation, scrolling, confirms — counts as activity.
                self.last_keypress_at = Some(std::time::Instant::now());
                match self.mode {
                    Mode::ModelPicker => Ok(self.handle_picker(key)),
                    Mode::Confirm => Ok(self.handle_confirm(key)),
                    Mode::AddModel => Ok(self.handle_add_model(key, tx)),
                    Mode::SessionPicker => Ok(self.handle_session_picker(key, tx)),
                    Mode::DisconnectPicker => Ok(self.handle_disconnect_picker(key)),
                    Mode::TelegramSetup => Ok(self.handle_telegram_setup_key(key, tx)),
                    Mode::AgentsSetup => Ok(self.handle_agents_setup_key(key, tx)),
                    Mode::ShellMonitor => Ok(self.handle_shell_monitor_key(key)),
                    Mode::Chat => Ok(self.handle_chat(key, tx)),
                }
            }
            _ => Ok(AppAction::Continue),
        }
    }

    pub(in crate::app) fn handle_command(
        &mut self,
        cmd: Command,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        match cmd {
            Command::Model(None) => self.open_picker(),
            Command::Model(Some(name)) => self.switch_model(&name),
            Command::ListModels => self.list_models_inline(),
            Command::Clear => self.clear_history(),
            Command::Quit => return AppAction::Quit,
            Command::Help => self.show_help_inline(),
            Command::Host(url) => self.switch_host(url, tx),
            Command::New => self.new_session(),
            Command::ListSessions => self.list_sessions_inline(tx),
            Command::Load(prefix) => self.load_session(prefix, tx),
            Command::More => self.push_info("/more is not needed — local sessions load fully. Use /sessions and /load <prefix>.".into()),
            Command::Workspace(path) => self.switch_workspace(path),
            Command::Compact => self.start_compact(tx, None),
            Command::Disconnect(name) => self.handle_disconnect(&name),
            Command::Update => self.start_update(tx),
            Command::Settings => self.show_settings(tx),
            Command::Trust => self.trust_current_workspace(),
            Command::Untrust => self.untrust_current_workspace(),
            Command::Telegram(sub) => self.handle_telegram(sub, tx),
            Command::Agents(sub) => self.handle_agents(sub, tx),
            Command::Ask { name, query } => self.handle_ask(name, query, tx),
            Command::Attach(path) => self.handle_attach(path),
            Command::Detach(arg) => self.handle_detach(arg),
            Command::Paste => self.paste_from_clipboard(),
            Command::Unknown(name) => {
                self.push_info(format!(
                    "Unknown command: /{name}\nType /help to see available commands."
                ));
                self.status = format!("Unknown: /{name}");
            }
        }
        AppAction::Continue
    }

    pub(super) fn push_info(&mut self, content: String) {
        self.messages.push(ChatMessage {
            role: "info".into(),
            content,
            ..Default::default()
        });
        self.follow = true;
    }

    pub(in crate::app) fn reset_input(&mut self) {
        let mut fresh = fresh_textarea();
        fresh.set_placeholder_text(
            "Type a message, or /help for commands.  (Enter=send, Alt+Enter / Ctrl+J=newline)",
        );
        self.input = fresh;
    }

    /// Send a user message that goes to the model but is NOT rendered in the
    /// chat UI. Used by the Y/N quick-reply so accept/deny doesn't pollute the
    /// visible transcript.
    pub(super) fn inject_hidden_user(&mut self, text: &str, tx: &mpsc::UnboundedSender<StreamMsg>) {
        if (self.models.is_empty() && self.extra_models.is_empty()) || self.turn.is_generating() {
            return;
        }
        self.start_turn(text.to_string(), true, tx);
    }

    pub(super) fn send_to_llm(&mut self, text: String, tx: &mpsc::UnboundedSender<StreamMsg>) {
        if self.models.is_empty() && self.extra_models.is_empty() {
            self.push_info(
                "Not connected to a model. Use /host <url> for Ollama, or /model to add a BYOK provider.".into(),
            );
            self.status = "No model".into();
            return;
        }

        // Auto-compaction: if the last assistant turn's prompt was over
        // threshold, fold the visible history into a summary first, then
        // re-issue this user message once compaction completes. Bail out
        // if we're already compacting (avoid re-entry) or generating.
        if self.turn.is_idle()
            && self.last_prompt_tokens > crate::compact::AUTO_COMPACT_THRESHOLD
            && self
                .messages
                .iter()
                .any(|m| !m.hidden && m.role == "assistant")
        {
            self.push_info(format!(
                "Context at {} tokens — compacting before sending so your next turn has room.",
                self.last_prompt_tokens
            ));
            self.start_compact(tx, Some(text));
            return;
        }

        self.start_turn(text, false, tx);
    }

    /// Shared core of `send_to_llm` and `inject_hidden_user`: persist
    /// the user message, push the visible placeholder pair, build the
    /// history snapshot (translating compaction summaries to system
    /// prompts), spawn the agent loop, and flip `turn` to Generating.
    ///
    /// Callers do their own pre-checks (model availability, auto-
    /// compact trigger, generating-already guards) and call this with
    /// `hidden = true` to keep the user message off the visible
    /// transcript (Y/N injections) or `false` for the normal flow.
    fn start_turn(&mut self, text: String, hidden: bool, tx: &mpsc::UnboundedSender<StreamMsg>) {
        // Persist user message to local JSONL session (skip hidden Y/N injections).
        if !hidden {
            self.ensure_local_session();
            if let (Some(sid), Some(path)) = (
                self.local_session_id.clone(),
                self.local_session_path.clone(),
            ) {
                let model = self.model.clone();
                let content = text.clone();
                tokio::spawn(async move {
                    let _ = crate::session::write_user(&path, &sid, &content, &model);
                });
            }
        }
        // Drain any /attach-queued media onto this user turn. Attachments
        // are in-memory only (not persisted), so they ride along once and
        // then disappear — re-attach per message if you want the model to
        // see the same image again on a follow-up turn.
        let attachments = std::mem::take(&mut self.pending_attachments);
        self.messages.push(ChatMessage {
            role: "user".into(),
            content: text,
            hidden,
            attachments,
            ..Default::default()
        });
        self.messages.push(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            ..Default::default()
        });
        self.follow = true;
        self.status = format!("Generating with {}…", self.model);

        // History sent to the model: prior user/assistant/tool turns plus
        // any compaction summary translated to a `system` role. The
        // trailing empty assistant placeholder is dropped.
        let history: Vec<ChatMessage> = self.messages[..self.messages.len() - 1]
            .iter()
            .filter(|m| matches!(m.role.as_str(), "user" | "assistant" | "tool" | "summary"))
            .map(|m| {
                if m.role == "summary" {
                    ChatMessage {
                        role: "system".into(),
                        content: format!("(Compacted summary of earlier turns:)\n\n{}", m.content),
                        ..m.clone()
                    }
                } else {
                    m.clone()
                }
            })
            .collect();

        let Some(backend) = self.make_backend() else {
            self.status = format!("No API key configured for model {}", self.model);
            return;
        };
        let model = self.model.clone();
        let workspace = self.workspace.clone();
        let runners = self.live_specialist_runners();
        let tool_defs = crate::tools::tool_definitions_with(&runners);
        let tx = tx.clone();
        let handle = tokio::spawn(async move {
            crate::agent::agent_loop_with(
                backend, model, history, workspace, tx, tool_defs, runners,
            )
            .await;
        });
        self.turn = TurnState::Generating { task: handle };
    }

    pub(in crate::app) fn cancel(&mut self) {
        // One match handles both task variants. The Idle arm exists so
        // a `cancel` during nothing-in-flight stays a no-op rather than
        // panicking on an unwrap somewhere.
        match std::mem::replace(&mut self.turn, TurnState::Idle) {
            TurnState::Idle => {}
            TurnState::Generating { task } => task.abort(),
            TurnState::Compacting { task, .. } => task.abort(),
        }
        self.persist_assistant_if_any();
        self.active_tool_msg_idx = None;
        self.active_specialist = None;
        // If a shell was in flight, the dropped tool task never gets to
        // send `ShellDone` — `kill_on_drop(true)` on the Command kills
        // the child, but the runtime would sit forever with
        // `running=true` and the footer would lie. Finalize manually
        // here so the indicator clears and the monitor (if open) reads
        // as killed.
        if let Some(rt) = self.active_shell.as_mut() {
            if rt.running {
                rt.running = false;
                rt.exit_code = Some(None);
                rt.kill_tx = None;
            }
        }
        // If the cancelled turn was answering a Telegram DM, let the
        // sender know — silence on their end would be worse than a curt
        // explanation. Best-effort; the bot may already be down.
        if let Some(chat_id) = self.pending_telegram_reply_chat.take() {
            self.pending_telegram_reply_from = None;
            if let Some(rt) = &self.telegram {
                let _ = rt.out_tx.send(crate::telegram::dm(
                    chat_id,
                    "(cancelled by the local user before a reply was produced)".into(),
                ));
            }
        }
        // Same for an in-flight Telegram-bridged confirm: tell the
        // remote sender we tore down the prompt before they answered
        // AND blank out the buttons on the original message so the
        // keyboard doesn't sit there pretending to be actionable.
        if let Some(ctx) = self.pending_telegram_confirm.take() {
            if let Some(req) = self.pending_confirm.take() {
                let _ = req.responder.send(false);
            }
            if let Some(rt) = &self.telegram {
                if let Some(message_id) = ctx.message_id {
                    let _ = rt.out_tx.send(crate::telegram::TelegramOut::EditMessage {
                        chat_id: ctx.chat_id,
                        message_id,
                        text: "(confirmation cancelled by the local user before you answered)"
                            .into(),
                    });
                } else {
                    let _ = rt.out_tx.send(crate::telegram::dm(
                        ctx.chat_id,
                        "(confirmation cancelled by the local user before you answered)".into(),
                    ));
                }
            }
        }
        self.status = "Cancelled".into();
    }

    /// Kick off an asynchronous compaction. Sends the current visible
    /// history (minus hidden / info / system entries) to the active model
    /// with a summarization system prompt. The reply lands as
    /// `StreamMsg::CompactionDone`, where `app::stream` replaces the
    /// visible history with a `summary`-role message and, if a pending
    /// user message was buffered via `pending_after_compact`, re-issues it.
    pub(super) fn start_compact(
        &mut self,
        tx: &mpsc::UnboundedSender<StreamMsg>,
        pending_user_message: Option<String>,
    ) {
        if self.turn.is_compacting() {
            self.push_info("A compaction is already running.".into());
            return;
        }
        if self.turn.is_generating() {
            self.push_info("Wait for the current turn to finish, then /compact.".into());
            return;
        }
        let to_compact_count = self
            .messages
            .iter()
            .filter(|m| !m.hidden && matches!(m.role.as_str(), "user" | "assistant" | "tool"))
            .count();
        if to_compact_count < 2 {
            self.push_info("Nothing meaningful to compact yet.".into());
            return;
        }
        let Some(backend) = self.make_backend() else {
            self.push_info(format!(
                "Can't compact — no backend configured for model {}.",
                self.model
            ));
            return;
        };

        // Snapshot the visible history for the task. Hidden user messages
        // (Y/N injections) are dropped — they're not real conversation
        // turns the summary should preserve.
        let snapshot: Vec<ChatMessage> = self
            .messages
            .iter()
            .filter(|m| !m.hidden)
            .cloned()
            .collect();
        let model = self.model.clone();
        let tx2 = tx.clone();
        self.status = "Compacting conversation…".into();
        self.follow = true;
        self.push_info("/compact — summarising prior turns into a single context briefing.".into());

        let handle = tokio::spawn(async move {
            match crate::compact::compact_history(&backend, &model, snapshot).await {
                Ok((summary, prompt_tokens, completion_tokens)) => {
                    let _ = tx2.send(StreamMsg::CompactionDone {
                        summary,
                        prompt_tokens,
                        completion_tokens,
                    });
                }
                Err(e) => {
                    let _ = tx2.send(StreamMsg::CompactionError(e.to_string()));
                }
            }
        });
        self.turn = TurnState::Compacting {
            task: handle,
            pending_user: pending_user_message,
        };
    }

    /// Defensively clear any `pending_user` buffered against an
    /// in-flight compaction. Called by `/clear` and `/new` so a stale
    /// user message can't get auto-resent after the user reset.
    pub(in crate::app) fn drop_pending_compact_user(&mut self) {
        if let TurnState::Compacting { pending_user, .. } = &mut self.turn {
            *pending_user = None;
        }
    }
}
