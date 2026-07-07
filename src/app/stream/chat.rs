//! Chat-flow stream handlers: assistant token chunks, end-of-turn
//! bookkeeping, fresh-turn placeholders, the `Done` finalizer (with
//! auto-continue-after-Y logic), and the error finaliser.

use tokio::sync::mpsc;

use crate::ollama::ToolCall;

use super::super::{App, StreamMsg};

impl App {
    pub(super) fn on_chunk(&mut self, text: String) {
        // When `/ask <name>` is in flight, lazily prepend `[name] ` to
        // the very first chunk of the assistant turn so the local user
        // can tell at a glance which specialist is talking. Subsequent
        // chunks just append.
        let prefix = if let Some(spec) = self.active_specialist.as_ref() {
            self.messages
                .last()
                .map(|m| m.role == "assistant" && m.content.is_empty())
                .unwrap_or(false)
                .then(|| format!("[{spec}] "))
        } else {
            None
        };
        if let Some(last) = self.messages.last_mut() {
            if last.role == "assistant" {
                if let Some(p) = prefix {
                    last.content.push_str(&p);
                }
                last.content.push_str(&text);
            }
        }
    }

    pub(super) fn on_assistant_turn_ended(&mut self, tool_calls: Vec<ToolCall>) {
        if let Some(last) = self.messages.last_mut() {
            if last.role == "assistant" {
                last.tool_calls = Some(tool_calls);
            }
        }
    }

    pub(super) fn on_new_assistant_turn(&mut self) {
        self.messages.push(crate::ollama::ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            ..Default::default()
        });
        self.follow = true;
    }

    pub(super) fn on_done(
        &mut self,
        prompt_tokens: u32,
        completion_tokens: u32,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        self.persist_assistant_if_any();
        // After the assistant's final text is settled, scan it for a
        // markdown checklist and lift it into `app.plan` for the
        // inspector pane. Empty plans stay empty (and the inspector
        // shows the "no plan yet" hint). Only replaces the plan when
        // one was found — if the assistant skipped the checklist this
        // turn, leave any prior plan alone so the user can still see
        // where they were.
        if let Some(last) = self.messages.last() {
            if last.role == "assistant" {
                let parsed = crate::app::plan::parse_plan(&last.content);
                if !parsed.is_empty() {
                    self.plan = parsed;
                }
            }
        }
        self.total_prompt_tokens = self
            .total_prompt_tokens
            .saturating_add(prompt_tokens as u64);
        self.total_completion_tokens = self
            .total_completion_tokens
            .saturating_add(completion_tokens as u64);
        // If this turn was a `/ask` invocation, accumulate the spend
        // under the specialist's name so the status line / `/agents
        // list` can show per-agent cost. The main totals above still
        // include it — they're "everything this session", the
        // per-agent tally is the breakdown.
        if let Some(spec) = self.active_specialist.take() {
            let entry = self.agent_token_tally.entry(spec).or_insert((0, 0));
            entry.0 = entry.0.saturating_add(prompt_tokens as u64);
            entry.1 = entry.1.saturating_add(completion_tokens as u64);
        }
        // Track this turn's prompt size for the next auto-compact
        // check in `send_to_llm`.
        self.last_prompt_tokens = prompt_tokens;
        self.turn = crate::app::TurnState::Idle;
        self.active_tool_msg_idx = None;
        self.status = format!(
            "Ready  ·  this turn: {} in / {} out",
            prompt_tokens, completion_tokens
        );

        // Phase 2 — Telegram bridge. If this turn was answering a DM,
        // forward the assistant's final text reply back to the sender.
        // Done BEFORE the Y-followup branch below so the Telegram user
        // always sees something land, even if we auto-continue locally.
        // Capture whether this turn was Telegram-triggered BEFORE the
        // flush clears the pending field, so the phase-3 notification
        // can skip turns that already DM'd via the bridge.
        let was_telegram_turn = self.pending_telegram_reply_chat.is_some();
        self.flush_pending_telegram_reply();

        // Phase 3 — idle notifications. If the user typed a turn
        // locally, walked away, and the reply landed quietly, DM paired
        // users so they don't have to come back to find out.
        if !was_telegram_turn {
            self.maybe_notify_idle();
        }

        // Auto-continue: if this turn was the reply to a Y-injection
        // and the model only announced intent (no tool calls + 'I'll
        // do X' text), nudge it to actually do the thing. One retry.
        if self.awaiting_yn_followup {
            self.awaiting_yn_followup = false;
            if self.no_tools_since_last_user() && self.looks_like_intent_announcement() {
                self.inject_hidden_user(
                    "You announced intent but didn't act. Call the necessary tools and \
                     actually do the work now — don't restate the plan.",
                    tx,
                );
                return;
            }
        }

        self.yn_pending = self.last_assistant_invites_yn();
    }

    pub(super) fn on_error(&mut self, e: String) {
        self.persist_assistant_if_any();
        self.turn = crate::app::TurnState::Idle;
        self.active_tool_msg_idx = None;
        // Specialist turn died — drop the attribution so the next
        // local turn doesn't get mis-tagged.
        self.active_specialist = None;
        // If a Telegram DM is still waiting on a reply, let it know the
        // turn died instead of leaving the sender hanging.
        if let Some(chat_id) = self.pending_telegram_reply_chat.take() {
            self.pending_telegram_reply_from = None;
            self.send_telegram_dm(chat_id, format!("hmanlab errored mid-reply: {e}"));
        }
        self.status = format!("Error: {e}");
    }

    /// If the most recent turn was triggered by a Telegram DM, forward
    /// the final assistant text (already persisted) back to the sender.
    /// Strips the `<think>` block and converts the model's markdown
    /// reply to Telegram HTML so `**bold**`, backtick code, and triple-
    /// fence blocks render natively instead of showing as raw markup.
    /// The TUI's own rendering is untouched — `messages` still holds
    /// the original markdown for the local renderer to use.
    fn flush_pending_telegram_reply(&mut self) {
        let Some(chat_id) = self.pending_telegram_reply_chat.take() else {
            return;
        };
        self.pending_telegram_reply_from = None;

        let reply = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
            .map(|m| strip_think(&m.content))
            .unwrap_or_default();

        if reply.trim().is_empty() {
            self.send_telegram_dm(
                chat_id,
                "(hmanlab finished the turn but produced no visible reply)".into(),
            );
            return;
        }

        let chunks = crate::telegram::format_for_telegram(&reply);
        if chunks.is_empty() {
            self.send_telegram_dm(chat_id, "(empty reply)".into());
            return;
        }
        for chunk in chunks {
            self.send_telegram_html(chat_id, chunk);
        }
    }
}

/// Drop the `<think>…</think>` reasoning block — it's useful in-session
/// as a foldable block but pure noise on Telegram.
fn strip_think(raw: &str) -> String {
    match raw.find("</think>") {
        Some(idx) => raw[idx + "</think>".len()..]
            .trim_start_matches(['\n', '\r'])
            .to_string(),
        None => raw.to_string(),
    }
}

impl App {
    /// Phase 3 entrypoint, called from `on_done` for locally-typed
    /// turns. DMs every allowlisted Telegram user the final reply if
    /// the terminal sat idle past the threshold. No-op when the flag
    /// is off, the bot's down, the allowlist is empty, or the idle
    /// duration is under threshold.
    pub(in crate::app) fn maybe_notify_idle(&mut self) {
        if !self.telegram_notify_on_idle {
            return;
        }
        let Some(last) = self.last_keypress_at else {
            // No keypress yet this session — treat as "user is here,
            // just hasn't typed". Don't notify; a true walk-away can
            // only be measured against a real keypress.
            return;
        };
        let idle = last.elapsed();
        if idle < super::super::TELEGRAM_IDLE_THRESHOLD {
            return;
        }

        // Need a live bot to send. If it isn't running we silently
        // skip (the user removed it on purpose with `/telegram off`).
        let Some(rt) = &self.telegram else {
            return;
        };
        let recipients: Vec<i64> = match rt.allowlist.lock() {
            Ok(list) if !list.is_empty() => list.clone(),
            _ => return,
        };

        let reply = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
            .map(|m| strip_think(&m.content))
            .unwrap_or_default();
        if reply.trim().is_empty() {
            return;
        }

        let header = format!(
            "🔔 Hibiscus finished a turn while you were away ({}s idle).",
            idle.as_secs()
        );
        // Same conversion as the bridge reply — render markdown as
        // Telegram HTML on the way out, leave the TUI's `messages`
        // untouched so the local renderer keeps using its own engine.
        let chunks = crate::telegram::format_for_telegram(&reply);
        for chat_id in recipients {
            self.send_telegram_dm(chat_id, header.clone());
            for chunk in &chunks {
                self.send_telegram_html(chat_id, chunk.clone());
            }
        }
    }
}
