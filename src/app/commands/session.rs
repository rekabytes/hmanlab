//! Session lifecycle commands: `/new`, `/clear`, `/sessions`, `/load`.
//!
//! Sessions are persisted locally as JSONL files under
//! `~/.config/hmanlab/sessions/<cwd-hash>/<session-uuid>.jsonl`.
//! No server, no login required.

use tokio::sync::mpsc;

use super::super::{App, StreamMsg};

impl App {
    /// `/clear` — drop visible history. The current session file stays intact
    /// on disk; the next message sent will append to a *new* session.
    pub(in crate::app) fn clear_history(&mut self) {
        self.messages.clear();
        self.total_prompt_tokens = 0;
        self.total_completion_tokens = 0;
        self.last_prompt_tokens = 0;
        self.agent_token_tally.clear();
        self.drop_pending_compact_user();
        self.pending_attachments.clear();
        self.scroll = 0;
        self.follow = true;
        self.page_state = crate::app::PageState::Idle;
        self.local_session_id = None;
        self.local_session_path = None;
        self.local_session_meta_written = false;
        self.status = "History cleared".into();
    }

    /// `/new` — wipe visible history and start a fresh local session.
    pub(in crate::app) fn new_session(&mut self) {
        self.messages.clear();
        self.total_prompt_tokens = 0;
        self.total_completion_tokens = 0;
        self.last_prompt_tokens = 0;
        self.agent_token_tally.clear();
        self.drop_pending_compact_user();
        self.pending_attachments.clear();
        self.scroll = 0;
        self.follow = true;
        self.loaded_session_id = None;
        self.oldest_loaded_msg_id = None;
        self.page_state = crate::app::PageState::Idle;
        self.local_session_id = None;
        self.local_session_path = None;
        self.local_session_meta_written = false;
        self.push_info("New session started.".into());
        self.status = "New session".into();
    }

    /// `/sessions` — list local sessions for the current workspace, show picker.
    pub(in crate::app) fn list_sessions_inline(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let cwd = self.workspace.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            match crate::session::list_sessions(&cwd) {
                Ok(rows) => {
                    let _ = tx.send(StreamMsg::LocalSessionList(rows));
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("list sessions: {e}")));
                }
            }
        });
    }

    /// `/load <prefix>` — find a session by UUID prefix and load it.
    pub(in crate::app) fn load_session(
        &mut self,
        prefix: String,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        if prefix.trim().is_empty() {
            self.push_info("Usage: /load <id-prefix>  (run /sessions for the list)".into());
            return;
        }
        let cwd = self.workspace.clone();
        let tx = tx.clone();
        tokio::spawn(async move {
            let summaries = match crate::session::list_sessions(&cwd) {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("load: {e}")));
                    return;
                }
            };
            let summary = match crate::session::find_by_prefix(&summaries, &prefix) {
                Ok(s) => s.clone(),
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("load: {e}")));
                    return;
                }
            };
            match crate::session::load_records(&summary.path) {
                Ok(records) => {
                    let messages = crate::session::records_to_chat(&records);
                    let _ = tx.send(StreamMsg::LocalSessionLoaded {
                        session_id: summary.session_id,
                        title: summary.title,
                        model: summary.model,
                        messages,
                        path: summary.path,
                    });
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(format!("load records: {e}")));
                }
            }
        });
    }
}
