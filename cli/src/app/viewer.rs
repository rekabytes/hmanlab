//! In-app file viewer state — opened by clicking a file row in the
//! sidebar. While `App::open_file` is `Some`, the chat surface is
//! replaced with a paginated text view; `Esc` closes it.

use std::path::PathBuf;
use tokio::sync::mpsc;

use super::{App, StreamMsg};

/// A file the user opened from the sidebar for inline reading.
pub struct OpenFile {
    /// Path shown in the viewer title (relative to workspace when possible).
    pub display: String,
    /// Either the file's text, or an empty string when `error` is set.
    pub content: String,
    /// Populated when the file couldn't be loaded (too large, binary, etc.).
    pub error: Option<String>,
    /// Scroll offset in lines (0 = top).
    pub scroll: u16,
    /// True between sidebar click and the spawn_blocking read completing.
    /// Renderer shows a "loading…" placeholder while this is true so the
    /// click feels responsive even on slow disks / large files.
    pub loading: bool,
}

/// Hard cap on file size loaded into the viewer — protects against opening
/// a 50 MB log by accident and dumping it through ratatui's text engine.
const VIEWER_MAX_BYTES: u64 = 256 * 1024;

impl App {
    /// Kick off a background read of a workspace file. Sets `self.open_file`
    /// immediately so the viewer paints a `loading…` placeholder without
    /// waiting; the actual content lands via `StreamMsg::FileLoaded` once
    /// `spawn_blocking` finishes. Caller passes an absolute path from
    /// `sidebar_targets`.
    pub fn open_workspace_file(&mut self, path: PathBuf, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let display = path
            .strip_prefix(&self.workspace)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        self.open_file = Some(OpenFile {
            display: display.clone(),
            content: String::new(),
            error: None,
            scroll: 0,
            loading: true,
        });
        let tx = tx.clone();
        let display_for_task = display.clone();
        tokio::task::spawn_blocking(move || {
            let (content, error) = match std::fs::metadata(&path) {
                Ok(m) if m.len() > VIEWER_MAX_BYTES => (
                    String::new(),
                    Some(format!(
                        "file is {} bytes; viewer cap is {} bytes",
                        m.len(),
                        VIEWER_MAX_BYTES
                    )),
                ),
                Ok(_) => match std::fs::read_to_string(&path) {
                    Ok(s) => (s, None),
                    // Binary or wrong encoding — surface a tidy message rather
                    // than dumping raw bytes through the renderer.
                    Err(e) => (String::new(), Some(format!("cannot read as text: {e}"))),
                },
                Err(e) => (String::new(), Some(format!("stat failed: {e}"))),
            };
            let _ = tx.send(StreamMsg::FileLoaded {
                display: display_for_task,
                content,
                error,
            });
        });
    }
}
