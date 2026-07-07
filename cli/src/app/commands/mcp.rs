//! `/mcp` — web search provider configuration modal.
//!
//! `handle_mcp` opens the three-screen provider-setup overlay.
//! The provider catalog lives in `app::mcp_providers` (a public module)
//! so the UI renderer can also reach it without fighting module privacy.

use tokio::sync::mpsc;

use crate::app::{fresh_textarea, App, McpSetupScreen, Mode, StreamMsg};
// Re-export so internal modules can reach the catalog without fighting privacy.
#[allow(unused_imports)]
pub use crate::app::mcp_providers::{provider_by_id, McpProvider, MCP_PROVIDERS};

impl App {
    /// Open the MCP provider setup modal. Called by the `/mcp` command handler.
    pub(in crate::app) fn handle_mcp(&mut self) {
        self.mcp_setup_screen = McpSetupScreen::ProviderList;
        self.mcp_setup_index = 0;
        self.mcp_setup_input = fresh_textarea();
        self.mcp_setup_error = None;
        self.mode = Mode::McpSetup;
    }

    /// Persist the current MCP state (active provider + keys) to the config
    /// file on disk. Fire-and-forget via `tokio::spawn`.
    pub(in crate::app) fn persist_mcp_config(&self, _tx: &mpsc::UnboundedSender<StreamMsg>) {
        let active = self.mcp_active_provider.clone();
        let brave = self.mcp_keys.get("brave").cloned();
        let exa = self.mcp_keys.get("exa").cloned();
        let tavily = self.mcp_keys.get("tavily").cloned();

        tokio::spawn(async move {
            let saved = crate::config::load().ok().flatten().unwrap_or_default();
            let updated = crate::config::Config {
                mcp_active_provider: active,
                brave_search_api_key: brave,
                exa_api_key: exa,
                tavily_api_key: tavily,
                ..saved
            };
            if let Err(e) = crate::config::save(&updated) {
                eprintln!("warn: failed to persist MCP config: {e}");
            }
        });
    }
}
