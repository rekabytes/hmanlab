//! Tool-execution stream handlers: appending the placeholder when a tool
//! starts, replacing it with the output when the tool returns, and the
//! confirm-popup intercept (with the workspace-trust short-circuit).

use crate::ollama::ChatMessage;
use crate::tools;

use super::super::{App, Mode, ShellRuntime};

impl App {
    pub(super) fn on_tool_start(&mut self, name: String, args: serde_json::Value) {
        let args_str = serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
        self.messages.push(ChatMessage {
            role: "tool".into(),
            name: Some(name),
            content: format!("(running… args: {args_str})"),
            ..Default::default()
        });
        self.active_tool_msg_idx = Some(self.messages.len() - 1);
        self.follow = true;
    }

    pub(super) fn on_tool_result(&mut self, output: String) {
        let mut tool_name: Option<String> = None;
        for msg in self.messages.iter_mut().rev() {
            if msg.role == "tool" {
                msg.content = output.clone();
                tool_name = msg.name.clone();
                break;
            }
        }
        // Write tool result to local session.
        if let (Some(name), Some(sid), Some(path)) = (
            tool_name,
            self.local_session_id.clone(),
            self.local_session_path.clone(),
        ) {
            let out = output.clone();
            tokio::spawn(async move {
                let _ = crate::session::write_tool(&path, &sid, &name, &out);
            });
        }
        self.active_tool_msg_idx = None;
    }

    pub(super) fn on_shell_start(
        &mut self,
        command: String,
        kill_tx: tokio::sync::oneshot::Sender<()>,
    ) {
        self.active_shell = Some(ShellRuntime {
            command,
            started_at: std::time::Instant::now(),
            output: Vec::new(),
            running: true,
            exit_code: None,
            kill_tx: Some(kill_tx),
            scroll: 0,
            follow_tail: true,
        });
    }

    pub(super) fn on_shell_output(&mut self, line: String, is_stderr: bool) {
        if let Some(rt) = self.active_shell.as_mut() {
            rt.push_line(line, is_stderr);
        }
    }

    pub(super) fn on_shell_done(&mut self, exit_code: Option<i32>) {
        if let Some(rt) = self.active_shell.as_mut() {
            rt.running = false;
            rt.exit_code = Some(exit_code);
            // Drop the kill handle — there's nothing left to kill, and
            // leaving the sender around invites a confused later
            // `.send(())` that would panic on a closed receiver.
            rt.kill_tx = None;
        }
    }

    pub(super) fn on_confirm_request(&mut self, req: tools::ConfirmRequest) {
        if !self.workspace_trusted() {
            // Trust gate: short-circuit before showing the popup.
            // Sending `false` through the oneshot makes the tool
            // return "user denied" to the agent loop, which then
            // surfaces it as a normal tool error in the chat.
            let _ = req.responder.send(false);
            self.push_info(format!(
                "Blocked: {}\nWorkspace not trusted. Run /trust to authorise this folder, \
                 or /workspace <path> to switch.",
                req.prompt
            ));
            self.status = "Blocked — workspace not trusted".into();
            return;
        }

        // Session-level "always allow" from a previous Telegram tap.
        // Matches OpenClaw's per-session policy: once the user picks
        // 🔏 Always for run_command, subsequent run_command prompts
        // auto-resolve in both places (no TUI popup, no DM).
        let head = prompt_head(&req.prompt);
        if self.telegram_always_allow.contains(&head) {
            let _ = req.responder.send(true);
            self.push_info(format!(
                "✓ Auto-allowed via Telegram session policy ({head}): {}",
                req.prompt
            ));
            if let Some(chat_id) = self.pending_telegram_reply_chat {
                self.send_telegram_dm(
                    chat_id,
                    format!("✓ Auto-allowed via your Always policy ({head})."),
                );
            }
            return;
        }

        // If the triggering turn came from Telegram, DM a confirm
        // prompt with OpenClaw-style buttons. We still open the
        // local popup — whichever side answers first wins (see
        // handle_confirm + handle_telegram_callback).
        if let Some(chat_id) = self.pending_telegram_reply_chat {
            let callback_token = crate::telegram::generate_callback_token();
            self.pending_telegram_confirm = Some(super::super::TelegramConfirmContext {
                chat_id,
                message_id: None,
                callback_token: callback_token.clone(),
                prompt_head: head,
            });
            let body = format_confirm_for_telegram(&req);
            if let Some(rt) = &self.telegram {
                let _ = rt.out_tx.send(crate::telegram::TelegramOut::SendConfirm {
                    chat_id,
                    text: body,
                    callback_token,
                });
            }
        }
        self.pending_confirm = Some(req);
        self.mode = Mode::Confirm;
        // Fresh prompt → start at the top. Without this, a long
        // first diff scrolled to its bottom would still be
        // scrolled when the next, possibly-short prompt opens.
        self.confirm_scroll = 0;
        self.status = "Confirmation needed — y/n".into();
    }
}

/// First whitespace-or-colon-delimited word of a confirm prompt.
/// `"run_command: ls -la"` → `"run_command"`. Used by the Always
/// policy match — same word here as the head of any future prompt
/// from the same tool.
fn prompt_head(prompt: &str) -> String {
    let trimmed = prompt.trim_start();
    let stop = trimmed
        .find(|c: char| c.is_whitespace() || c == ':')
        .unwrap_or(trimmed.len());
    trimmed[..stop].to_string()
}

/// Plain-text rendering of a `ConfirmRequest` for Telegram. The TUI's
/// popup colours the diff; on Telegram we keep it ASCII with the
/// existing `+ ` / `- ` / `  ` prefixes the diff lines already carry.
/// Cap at ~50 lines so a giant rewrite doesn't blow past the
/// `sendMessage` ceiling on its own — `chunk_for_telegram` would split
/// it but a 4000-char body of code is unreadable on a phone anyway.
fn format_confirm_for_telegram(req: &tools::ConfirmRequest) -> String {
    let mut s = format!(
        "🔐 hmanlab wants to run a tool that needs your approval:\n\n{}\n",
        req.prompt
    );
    if !req.diff.is_empty() {
        s.push('\n');
        const MAX_LINES: usize = 50;
        let shown: Vec<&tools::DiffLine> = req.diff.iter().take(MAX_LINES).collect();
        for line in &shown {
            s.push_str(&line.text);
            s.push('\n');
        }
        if req.diff.len() > MAX_LINES {
            s.push_str(&format!(
                "…(+{} more lines — see the local TUI for the full diff)\n",
                req.diff.len() - MAX_LINES
            ));
        }
    }
    s.push_str(
        "\nTap a button below — or reply 'y' / 'n' if your client doesn't render inline keyboards.",
    );
    s
}
