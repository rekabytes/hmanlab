//! Compaction stream handlers: turn the model's summary into a `summary`
//! role message, persist it to project memory, and replay any user
//! message that was buffered while compaction was running.

use tokio::sync::mpsc;

use crate::ollama::ChatMessage;

use super::super::{App, StreamMsg, TurnState};

impl App {
    pub(super) fn on_compaction_done(
        &mut self,
        summary: String,
        prompt_tokens: u32,
        completion_tokens: u32,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        // Extract the buffered user message (if any) before flipping
        // back to Idle — the Compacting variant owns it.
        let pending = match std::mem::replace(&mut self.turn, TurnState::Idle) {
            TurnState::Compacting { pending_user, .. } => pending_user,
            _ => None,
        };
        self.total_prompt_tokens = self
            .total_prompt_tokens
            .saturating_add(prompt_tokens as u64);
        self.total_completion_tokens = self
            .total_completion_tokens
            .saturating_add(completion_tokens as u64);

        // Persist the summary as a project-scope memory so it
        // survives /clear, restart, and cross-session use. Rolling
        // single file (`compact-current.md`) — each new compaction
        // overwrites it. The model picks it up on next launch via
        // the always-loaded MEMORY.md index.
        let description = compact_description(&summary);
        let persisted = crate::memory::save_memory(
            crate::memory::MemoryScope::Project,
            "compact-current",
            "project",
            &description,
            &summary,
            &self.workspace,
        );

        // Replace the visible history with a single `summary`-role
        // entry. The `send_to_llm` filter translates summary →
        // system before each next turn, so the model still sees
        // the compacted context as load-bearing background.
        let summary_msg = ChatMessage {
            role: "summary".into(),
            content: summary,
            ..Default::default()
        };
        self.messages.clear();
        self.messages.push(summary_msg);
        // Reset the auto-compact trigger — the next turn starts
        // small again.
        self.last_prompt_tokens = 0;
        self.follow = true;
        self.status = format!(
            "Compacted  ·  summary: {} in / {} out",
            prompt_tokens, completion_tokens
        );
        match persisted {
            Ok(path) => self.push_info(format!(
                "Saved to {} (project memory `compact-current`).",
                path.display()
            )),
            Err(e) => self.push_info(format!("Compaction succeeded but persistence failed: {e}")),
        }
        // Replay the user message that was buffered while
        // compaction ran, if any. Done last so all state is
        // consistent before the new turn starts.
        if let Some(text) = pending {
            self.send_to_llm(text, tx);
        }
    }

    pub(super) fn on_compaction_error(&mut self, e: String) {
        // Drop any buffered message — re-running it would just
        // re-trigger the auto-compact. The original history is
        // intact since we didn't touch it before the result came back.
        self.turn = TurnState::Idle;
        self.push_info(format!("Compaction failed: {e}. History unchanged."));
        self.status = format!("Compact error: {e}");
    }
}

/// Pull a one-line description out of a compaction summary for use as the
/// memory's index entry. Tries to find the first non-empty bullet,
/// strips the marker (`- `, `* `, `• `), and caps at 200 chars so the
/// MEMORY.md index stays scannable.
fn compact_description(summary: &str) -> String {
    let mut picked = String::new();
    for line in summary.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let cleaned = trimmed.trim_start_matches(['-', '*', '•']).trim();
        if !cleaned.is_empty() {
            picked = cleaned.replace(['\r', '\n'], " ");
            break;
        }
    }
    if picked.is_empty() {
        picked = "Compacted session summary".to_string();
    }
    if picked.chars().count() > 200 {
        let mut t: String = picked.chars().take(199).collect();
        t.push('…');
        picked = t;
    }
    picked
}
