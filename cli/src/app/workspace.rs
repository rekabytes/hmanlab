//! Workspace + trust state.
//!
//! Owns:
//!   - `/workspace <path>` implementation (`App::switch_workspace`)
//!   - workspace-trust gate (`/trust`, `/untrust`) and its persistence
//!   - tilde expansion + the entry-time trust banner
//!
//! Kept in one file because trust is intrinsically tied to "what folder
//! is the agent operating in" — splitting them would put every state
//! change in two places.

use std::path::PathBuf;

use super::App;

impl App {
    /// `/workspace <path>` — switch the agent's working directory.
    /// Relative paths resolve against the *current* workspace (not the
    /// process CWD), so chained `/workspace ../sibling` calls do what
    /// you'd expect. `~` and `~/path` expand against `$HOME`.
    pub(super) fn switch_workspace(&mut self, path: String) {
        let path = path.trim();
        if path.is_empty() {
            self.push_info(format!(
                "Current workspace: {}\nUsage: /workspace <path>",
                self.workspace.display()
            ));
            return;
        }
        // Expand `~` / `~/foo` to $HOME so users can paste home-relative
        // paths the way they would in a shell.
        let expanded = expand_tilde(path);
        // Relative paths are resolved against the *current* workspace, not
        // the process CWD. Without this, `/workspace ../sibling` always
        // means "from where I launched hmanlab" — so chained switches like
        // `/workspace ~/a` then `/workspace ../b` silently move from the
        // process CWD again, which feels like /workspace only worked once.
        let base = if expanded.is_absolute() {
            expanded
        } else {
            self.workspace.join(expanded)
        };
        let canonical = match base.canonicalize() {
            Ok(p) if p.is_dir() => p,
            Ok(_) => {
                self.push_info(format!("Not a directory: {path}"));
                return;
            }
            Err(e) => {
                self.push_info(format!(
                    "Cannot use '{path}' (resolved against {}): {e}",
                    self.workspace.display()
                ));
                return;
            }
        };
        if canonical == self.workspace {
            self.push_info(format!(
                "Already in workspace: {}",
                self.workspace.display()
            ));
            return;
        }
        self.workspace = canonical;
        // Sidebar state belongs to the previous workspace — reset it so the
        // new workspace gets its own top-level expansion and a fresh scroll
        // position. Without this, the picker would still hold paths from
        // the old tree that no longer exist in the new one. `seed` reads
        // `workspace_trusted()` so the dotfile-visibility decision picks
        // up the new workspace's trust state automatically.
        self.seed_sidebar_top_level();
        self.push_info(format!("Workspace: {}", self.workspace.display()));
        if !self.workspace_trusted() {
            self.push_info(workspace_trust_banner(&self.workspace));
        }
        self.status = format!("Workspace: {}", self.workspace.display());
    }

    /// Mark the current workspace as trusted and persist it to config.
    pub(super) fn trust_current_workspace(&mut self) {
        if self.workspace_trusted() {
            self.push_info(format!(
                "Workspace already trusted: {}",
                self.workspace.display()
            ));
            return;
        }
        self.trusted_workspaces.push(self.workspace.clone());
        // Re-seed the sidebar so dotfiles (.env, .hmanlab, etc.) become
        // visible immediately — without this the tree only refreshes on
        // a manual collapse/expand or workspace switch.
        self.seed_sidebar_top_level();
        if let Err(e) = persist_trusted_workspaces(&self.trusted_workspaces) {
            self.push_info(format!("Trusted in-session, but failed to save: {e}"));
            return;
        }
        self.push_info(format!(
            "Trusted workspace: {}\nThe agent can now edit files, run shell commands, \
             and save memories here. Hidden files are visible in the sidebar.",
            self.workspace.display()
        ));
        self.status = "Workspace trusted".into();
    }

    /// Remove the current workspace from the persisted trusted list.
    pub(super) fn untrust_current_workspace(&mut self) {
        let before = self.trusted_workspaces.len();
        self.trusted_workspaces.retain(|p| p != &self.workspace);
        if self.trusted_workspaces.len() == before {
            self.push_info(format!(
                "Workspace wasn't trusted: {}",
                self.workspace.display()
            ));
            return;
        }
        // Re-seed so the sidebar hides dotfiles again now that they're no
        // longer authorised — keeps the visible tree consistent with what
        // the agent can actually touch.
        self.seed_sidebar_top_level();
        if let Err(e) = persist_trusted_workspaces(&self.trusted_workspaces) {
            self.push_info(format!("Untrusted in-session, but failed to save: {e}"));
            return;
        }
        self.push_info(format!(
            "Untrusted workspace: {}\nDestructive tools are now blocked here. \
             Hidden files are hidden in the sidebar.",
            self.workspace.display()
        ));
        self.status = "Workspace untrusted".into();
    }
}

/// Expand a leading `~` to `$HOME` (handles both `~` and `~/path`). Any
/// other path is returned unchanged. Cross-platform: silently no-ops if
/// `$HOME` isn't set.
fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home);
        }
    } else if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Banner shown on entry to an untrusted workspace. Tells the user what's
/// gated and how to unblock it. `pub` because `main.rs` shows it once on
/// startup before the TUI alt-screen swaps in.
pub fn workspace_trust_banner(workspace: &std::path::Path) -> String {
    format!(
        "⚠ Untrusted workspace: {}\n\
         File edits, shell commands, and memory writes are blocked here.\n\
         Run /trust to authorise this workspace, or /workspace <path> to switch.",
        workspace.display()
    )
}

/// Rewrite the persisted config with the given trusted list. Loads the
/// existing config (or starts fresh if none), updates only the trusted
/// field, and saves. Keeping the rest of the config untouched is important
/// — we don't want /trust to clobber BYOK keys.
fn persist_trusted_workspaces(trusted: &[PathBuf]) -> anyhow::Result<()> {
    let mut cfg = crate::config::load()?.unwrap_or_default();
    cfg.trusted_workspaces = trusted.iter().map(|p| p.display().to_string()).collect();
    crate::config::save(&cfg)
}
