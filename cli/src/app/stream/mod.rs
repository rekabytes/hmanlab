//! Stream-message handler. The agent loop and other background tasks
//! emit `StreamMsg` events; `handle_stream` is the single dispatcher,
//! routing each variant to a focused per-category handler:
//!
//!   - `chat`       — assistant chunks, turn lifecycle, errors.
//!   - `tools`      — tool start/result + the confirm popup intercept.
//!   - `compaction` — `/compact` done/error + memory persistence.
//!   - `system`     — host change, update notifications, `/settings`.

use tokio::sync::mpsc;

use super::{App, StreamMsg};

mod chat;
mod compaction;
mod sessions;
mod system;
mod tools;

impl App {
    pub fn handle_stream(&mut self, msg: StreamMsg, tx: &mpsc::UnboundedSender<StreamMsg>) {
        match msg {
            StreamMsg::Chunk(text) => self.on_chunk(text),
            StreamMsg::AssistantTurnEnded { tool_calls } => {
                self.on_assistant_turn_ended(tool_calls)
            }
            StreamMsg::ToolStart { name, args } => self.on_tool_start(name, args),
            StreamMsg::ToolResult { output } => self.on_tool_result(output),
            StreamMsg::ShellStart { command, kill_tx } => self.on_shell_start(command, kill_tx),
            StreamMsg::ShellOutput { line, is_stderr } => self.on_shell_output(line, is_stderr),
            StreamMsg::ShellDone { exit_code } => self.on_shell_done(exit_code),
            StreamMsg::NewAssistantTurn => self.on_new_assistant_turn(),
            StreamMsg::ConfirmRequest(req) => self.on_confirm_request(req),
            StreamMsg::Done {
                prompt_tokens,
                completion_tokens,
            } => self.on_done(prompt_tokens, completion_tokens, tx),
            StreamMsg::Error(e) => self.on_error(e),
            StreamMsg::CompactionDone {
                summary,
                prompt_tokens,
                completion_tokens,
            } => self.on_compaction_done(summary, prompt_tokens, completion_tokens, tx),
            StreamMsg::CompactionError(e) => self.on_compaction_error(e),
            StreamMsg::UpdateAvailable(latest) => {
                self.update_available = Some(latest);
            }
            StreamMsg::UpdateInfo(text) => {
                self.push_info(text);
            }
            StreamMsg::Settings(text) => self.on_settings(text),
            StreamMsg::UpdateResult { ok, text } => self.on_update_result(ok, text),
            StreamMsg::Models { models, base } => self.on_models(models, base),
            StreamMsg::LocalSessionList(rows) => self.on_local_session_list(rows),
            StreamMsg::LocalSessionLoaded {
                session_id,
                title,
                model,
                messages,
                path,
            } => self.on_local_session_loaded(session_id, title, model, messages, path),
            StreamMsg::OpenRouterModelsRefreshed(models) => {
                self.on_openrouter_models_refreshed(models)
            }
            StreamMsg::SpecialistTokens {
                name,
                prompt_tokens,
                completion_tokens,
            } => {
                // Phase 2 attribution — also bump the main session
                // totals so the global counter reflects the full spend
                // (header shows main + per-specialist breakdown).
                let entry = self.agent_token_tally.entry(name).or_insert((0, 0));
                entry.0 = entry.0.saturating_add(prompt_tokens as u64);
                entry.1 = entry.1.saturating_add(completion_tokens as u64);
                self.total_prompt_tokens = self
                    .total_prompt_tokens
                    .saturating_add(prompt_tokens as u64);
                self.total_completion_tokens = self
                    .total_completion_tokens
                    .saturating_add(completion_tokens as u64);
            }
            StreamMsg::SpecialistRequest {
                runner,
                query,
                reply_tx,
            } => {
                // Phase 3: top-level spawn for the specialist sub-agent.
                // Runs on its own Tokio worker so the parent agent's
                // task isn't blocked while the specialist iterates;
                // cancellation chains naturally via reply_tx.closed()
                // inside `run_specialist_consult`.
                let workspace = self.workspace.clone();
                let parent_tx = tx.clone();
                tokio::spawn(crate::agent::run_specialist_consult(
                    runner, query, workspace, parent_tx, reply_tx,
                ));
            }
            StreamMsg::FileLoaded {
                display,
                content,
                error,
            } => {
                // Drop the result if the user has already closed the
                // viewer or clicked a different file — the placeholder
                // we set on click carries the display path, and any
                // mismatch means the click that spawned this read is
                // no longer the active selection.
                if let Some(f) = self.open_file.as_mut() {
                    if f.display == display {
                        f.content = content;
                        f.error = error;
                        f.loading = false;
                    }
                }
            }
            StreamMsg::TelegramIncoming {
                chat_id,
                from,
                text,
            } => {
                self.handle_telegram_message(chat_id, from, text, tx);
            }
            StreamMsg::TelegramBotStatus(text) => {
                self.telegram_last_status = Some(text.clone());
                // If the wizard is open AND this is a setup failure
                // signal, surface it in the modal (instead of as a
                // chat info line) and let the user retry the token.
                // Also tear down the half-built runtime that
                // telegram_setup parked eagerly — keeping it around
                // would lie about "we're online" forever.
                if self.mode == super::Mode::TelegramSetup
                    && self.telegram_setup_step == super::TelegramSetupStep::Token
                    && text.contains("setup failed")
                {
                    if let Some(rt) = self.telegram.take() {
                        let _ = rt.ctl_tx.send(crate::telegram::TelegramCtl::Shutdown);
                    }
                    self.telegram_setup_validating = false;
                    self.telegram_setup_error = Some(text);
                    self.telegram_setup_input = super::fresh_textarea();
                    self.telegram_setup_input
                        .set_placeholder_text("Paste your @BotFather token here");
                    return;
                }
                self.push_info(text);
            }
            StreamMsg::TelegramBotReady { token, username } => {
                if let Some(rt) = self.telegram.as_mut() {
                    if rt.token == token {
                        rt.bot_username = username;
                    }
                }
                // If the wizard is waiting on this step's validation,
                // advance to Pair so the user can paste the code.
                if self.mode == super::Mode::TelegramSetup
                    && self.telegram_setup_step == super::TelegramSetupStep::Token
                {
                    self.telegram_setup_step = super::TelegramSetupStep::Pair;
                    self.telegram_setup_validating = false;
                    self.telegram_setup_error = None;
                    self.telegram_setup_input = super::fresh_textarea();
                    self.telegram_setup_input
                        .set_placeholder_text("Paste the 6-char code the bot DM'd you");
                    self.status =
                        "Telegram setup — DM the bot, then paste the code  ·  Esc to cancel".into();
                }
            }
            StreamMsg::TelegramConfirmSent {
                callback_token,
                message_id,
            } => {
                if let Some(ctx) = self.pending_telegram_confirm.as_mut() {
                    if ctx.callback_token == callback_token {
                        ctx.message_id = Some(message_id);
                    }
                }
            }
            StreamMsg::TelegramCallback {
                callback_token,
                action,
                chat_id,
                message_id,
                from,
                from_id,
            } => {
                self.handle_telegram_callback(
                    callback_token,
                    action,
                    chat_id,
                    message_id,
                    from,
                    from_id,
                );
            }
        }
    }

    /// Persist the trailing assistant message if it's the final reply
    /// (no tool_calls) and non-empty. Otherwise drop empties.
    pub(super) fn persist_assistant_if_any(&mut self) {
        if let Some(last) = self.messages.last() {
            if last.role != "assistant" {
                return;
            }
            let has_tool_calls = last
                .tool_calls
                .as_ref()
                .map(|tc| !tc.is_empty())
                .unwrap_or(false);
            if last.content.trim().is_empty() && !has_tool_calls {
                self.messages.pop();
            } else if !has_tool_calls && !last.content.trim().is_empty() {
                // Strip the `<think>…</think>` reasoning block before persisting.
                // It's useful in-session as a foldable block but is in-flight
                // scratch — durable storage should hold only the visible answer.
                let raw = &last.content;
                let content = match raw.find("</think>") {
                    Some(idx) => raw[idx + "</think>".len()..]
                        .trim_start_matches(['\n', '\r'])
                        .to_string(),
                    None => raw.clone(),
                };
                if content.trim().is_empty() {
                    return;
                }
                // Write to local JSONL session.
                if let (Some(sid), Some(path)) = (
                    self.local_session_id.clone(),
                    self.local_session_path.clone(),
                ) {
                    let model = self.model.clone();
                    let c = content.clone();
                    tokio::spawn(async move {
                        let _ = crate::session::write_assistant(&path, &sid, &c, &model);
                    });
                }
            }
        }
    }
}
