//! Local session stream handlers: list picker and load-from-disk.

use crate::ollama::ChatMessage;

use super::super::{App, Mode, PageState};

impl App {
    pub(super) fn on_local_session_list(&mut self, rows: Vec<crate::session::SessionSummary>) {
        if rows.is_empty() {
            self.push_info(
                "No saved sessions yet. Chat first, then /sessions to browse history.".into(),
            );
            return;
        }
        // Convert to the api::Session shape the existing picker expects.
        let api_rows: Vec<crate::api::Session> = rows
            .iter()
            .map(|s| crate::api::Session {
                id: s.session_id.clone(),
                title: s.title.clone(),
                model: s.model.clone(),
            })
            .collect();
        self.session_picker.set_items(api_rows);
        // Stash summaries so Enter can resolve back to the path.
        self.local_session_summaries = rows;
        self.mode = Mode::SessionPicker;
        self.status = "↑↓ to navigate · Enter to load · Esc to cancel".into();
    }

    pub(super) fn on_local_session_loaded(
        &mut self,
        session_id: String,
        title: String,
        model: Option<String>,
        messages: Vec<ChatMessage>,
        path: std::path::PathBuf,
    ) {
        self.messages = messages;
        self.page_state = PageState::Idle;
        if let Some(recorded) = model.as_deref() {
            if recorded != self.model {
                self.push_info(format!(
                    "Session was recorded with model {recorded}; continuing with {} (run /model to switch).",
                    self.model
                ));
            }
        }
        // Wire the loaded session as the active one so new turns append to it.
        self.local_session_id = Some(session_id.clone());
        self.local_session_path = Some(path);
        self.local_session_meta_written = true;
        self.loaded_session_id = Some(session_id.clone());
        self.oldest_loaded_msg_id = None;
        self.follow = true;
        self.scroll = 0;
        let count = self.messages.len();
        let id_short = &session_id[..session_id.len().min(8)];
        self.status = format!("Loaded — {count} message(s)");
        self.push_info(format!(
            "Loaded session {id_short} — \"{title}\"  ·  {count} message(s)"
        ));
    }
}
