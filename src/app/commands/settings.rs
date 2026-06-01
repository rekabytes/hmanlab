//! `/settings` and `/update`.

use tokio::sync::mpsc;

use super::super::{App, StreamMsg};

impl App {
    /// `/settings` — show version, active model, Ollama host, configured
    /// BYOK providers (presence only, never the key), and workspace.
    pub(in crate::app) fn show_settings(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let current = env!("CARGO_PKG_VERSION");
        let byok: Vec<&str> = crate::config::BYOK_PROVIDERS
            .iter()
            .filter(|p| self.has_byok_key(p))
            .map(|p| crate::config::provider_label(p))
            .collect();
        let byok_line = if byok.is_empty() {
            "none".to_string()
        } else {
            byok.join(", ")
        };
        let upstream = self.update_available.as_deref();
        let version_line = match upstream {
            Some(latest) if crate::update_check::newer(current, latest) => {
                format!("{current}  (npm has {latest} — run /update)")
            }
            _ => current.to_string(),
        };
        let body = format!(
            "Settings\n\
             \x20 hmanlab version  : {version_line}\n\
             \x20 model            : {model}\n\
             \x20 ollama host      : {host}\n\
             \x20 BYOK providers   : {byok_line}\n\
             \x20 workspace        : {ws}",
            model = self.model,
            host = self.client.base,
            ws = self.workspace.display(),
        );
        self.push_info(body.clone());
        self.status = "Settings".into();

        // Background version check (npm). If a newer version is found the
        // stream handler appends an update line; if not, it's a no-op.
        let current_owned = current.to_string();
        let tx = tx.clone();
        let pending_idx = self.messages.len().saturating_sub(1);
        self.pending_settings_msg_idx = Some(pending_idx);
        let body_owned = body;
        tokio::spawn(async move {
            let latest = crate::update_check::fetch_latest_npm().await.ok();
            let version_tail = match latest {
                Some(l) if crate::update_check::newer(&current_owned, &l) => {
                    format!("\n\nnpm latest: {l} — run /update to install.")
                }
                Some(l) => format!("\n\nnpm latest: {l} (you're up to date)."),
                None => return,
            };
            let _ = tx.send(StreamMsg::Settings(format!("{body_owned}\n{version_tail}")));
        });
    }

    /// `/update` — upgrade the on-disk binary through whichever channel the
    /// user installed from. The currently running process keeps serving the
    /// chat; the upgrade replaces the on-disk binary; the user picks it up
    /// on next launch.
    ///
    /// Channel detection is path-based off `std::env::current_exe()`:
    ///   - cargo (`.cargo/bin/…`, `target/…`) → surface the `cargo install`
    ///     command instead of running anything ourselves.
    ///   - curl (binary sits under `.local/bin`, `/usr/local/bin`, or any
    ///     other path not matching the other two) → surface the install.sh
    ///     one-liner.
    ///   - npm (`node_modules` in the path) → actually run
    ///     `npm install -g hmanlab@latest` for the user.
    ///
    /// We only auto-run for npm because that's the only channel where the
    /// upgrade command is self-contained (no sudo, no curl-from-internet,
    /// no compiler). The other two we show the command and let the user
    /// run it themselves.
    pub(in crate::app) fn start_update(&mut self, tx: &mpsc::UnboundedSender<StreamMsg>) {
        let current = env!("CARGO_PKG_VERSION");

        match detect_install_channel() {
            InstallChannel::Cargo(hint) => {
                self.push_info(format!(
                    "hmanlab looks like a cargo install ({hint}).\n\
                     Run this in another terminal to upgrade:\n\
                     \x20 cargo install hmanlab --force"
                ));
                self.status = "Cargo install detected — see message".into();
                return;
            }
            InstallChannel::Curl(path) => {
                self.push_info(format!(
                    "hmanlab looks like a curl install ({path}).\n\
                     Run this in another terminal to upgrade:\n\
                     \x20 curl -fsSL https://github.com/hmanlab/hmanlab/releases/latest/download/install.sh | sh\n\
                     \n\
                     (Set HMANLAB_INSTALL_DIR if the binary lives somewhere other than ~/.local/bin.)"
                ));
                self.status = "Curl install detected — see message".into();
                return;
            }
            InstallChannel::Npm => {
                // Fall through to the npm branch below.
            }
        }

        self.push_info(format!(
            "Checking npm for a newer hmanlab (current {current})…"
        ));
        self.status = "Checking latest version…".into();

        let tx = tx.clone();
        let current_owned = current.to_string();
        tokio::spawn(async move {
            // Step 1: ask npm what's published. If the lookup fails we still
            // proceed to install — the user explicitly asked, and a flaky
            // registry shouldn't block them. If it succeeds and the current
            // version is already latest, bail out without spawning npm.
            match crate::update_check::fetch_latest_npm().await {
                Ok(latest) if !crate::update_check::newer(&current_owned, &latest) => {
                    let _ = tx.send(StreamMsg::UpdateResult {
                        ok: true,
                        text: format!(
                            "Already up to date — hmanlab {current_owned} matches the latest \
                             on npm ({latest}). No install needed."
                        ),
                    });
                    return;
                }
                Ok(latest) => {
                    let _ = tx.send(StreamMsg::UpdateInfo(format!(
                        "Update available: {current_owned} → {latest}. \
                         Running: npm install -g hmanlab@latest"
                    )));
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::UpdateInfo(format!(
                        "Couldn't reach npm registry ({e}). Trying install anyway…"
                    )));
                }
            }

            let result = tokio::process::Command::new("npm")
                .args(["install", "-g", "hmanlab@latest"])
                .output()
                .await;
            let msg = match result {
                Ok(out) if out.status.success() => StreamMsg::UpdateResult {
                    ok: true,
                    text: "Update complete. Restart hmanlab to use the new version.".into(),
                },
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let tail = stderr.lines().rev().take(8).collect::<Vec<_>>();
                    let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
                    StreamMsg::UpdateResult {
                        ok: false,
                        text: format!(
                            "npm install failed (exit {}).\n{}",
                            out.status.code().unwrap_or(-1),
                            if tail.is_empty() {
                                "No stderr output.".into()
                            } else {
                                tail
                            }
                        ),
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => StreamMsg::UpdateResult {
                    ok: false,
                    text: "Couldn't run `npm` — it's not on PATH.\n\
                           Install Node.js (https://nodejs.org) and try again, or grab a\n\
                           prebuilt binary from https://github.com/rekabytes/hmanlab/releases."
                        .into(),
                },
                Err(e) => StreamMsg::UpdateResult {
                    ok: false,
                    text: format!("Failed to launch npm: {e}"),
                },
            };
            let _ = tx.send(msg);
        });
    }
}

/// Where `/update` thinks the binary came from. Drives which upgrade
/// command we suggest (or run).
enum InstallChannel {
    /// Built / installed via cargo. Carries the path fragment we matched
    /// on for the user-facing message (`.cargo/bin`, `target/release`,
    /// `target/debug`).
    Cargo(&'static str),
    /// Dropped in place by `install.sh` (curl one-liner). Carries the
    /// binary's full path string so the user can see exactly where the
    /// match came from.
    Curl(String),
    /// Installed via `npm install -g hmanlab`. The binary lives somewhere
    /// under a `node_modules` directory.
    Npm,
}

/// Guess which install channel the running binary came from, off
/// `std::env::current_exe()`. Heuristic order:
///   1. Cargo if the path contains a cargo-specific fragment.
///   2. Npm if the path contains `node_modules` (npm symlinks resolve to
///      the platform package binary inside `node_modules/@hmanlab/…`).
///   3. Curl as the catch-all — `install.sh` drops to `~/.local/bin` by
///      default, but the user could override `HMANLAB_INSTALL_DIR`. Any
///      install path that isn't cargo or npm is treated as curl.
///
/// If `current_exe()` itself fails (rare), we fall back to Curl with an
/// empty path — the on-screen message still works because the upgrade
/// command is self-contained.
fn detect_install_channel() -> InstallChannel {
    let path = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    // `.cargo/bin/hmanlab` covers `cargo install`; `target/release` and
    // `target/debug` cover devs running from a local checkout.
    for needle in [".cargo/bin", "target/release", "target/debug"] {
        if path.contains(needle) {
            return InstallChannel::Cargo(needle);
        }
    }
    if path.contains("node_modules") {
        return InstallChannel::Npm;
    }
    InstallChannel::Curl(path)
}
