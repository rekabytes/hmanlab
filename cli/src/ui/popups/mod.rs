//! Overlay surfaces — modal popups + the floating inline-autocomplete.
//!
//! Each render fn lives in its own submodule and is wired into
//! `ui::mod::render`. Modal popups receive the bottom half of the
//! split chat column as their full render area and fill it edge to
//! edge; inline autocomplete is non-modal and anchors above the input
//! box instead.
//!
//!   - `picker`        — model picker (Ctrl+M / `/model`).
//!   - `sessions`      — `/sessions` saved-session list.
//!   - `disconnect`    — `/disconnect` BYOK-provider picker.
//!   - `add_model`     — paste-your-API-key dialog after picking
//!     "+ Add … key" in the model picker.
//!   - `shell_monitor` — live `run_command` stdout/stderr viewer.
//!   - `inline`        — floating `/cmd` and `@file` autocomplete that
//!     anchors above the input.
//!
//! Note: the destructive-action confirm popup used to live here too
//! (`confirm` submodule). It has been replaced by an inline card in
//! the chat — see `ui::chat::messages::render_permission_card`. The
//! Mode::Confirm keymap still drives the y/n/Esc keyboard handler
//! (`handle_confirm`) and the Approve/Deny buttons.

mod add_model;
mod agents_setup;
mod disconnect;
mod inline;
mod mcp;
mod picker;
mod sessions;
mod shell_monitor;
mod telegram_setup;

pub(super) use add_model::render_add_model;
pub(super) use agents_setup::render_agents_setup;
pub(super) use disconnect::render_disconnect_picker;
pub(super) use inline::render_inline_popup;
pub(super) use mcp::render_mcp_setup;
pub(super) use picker::render_picker;
pub(super) use sessions::render_session_picker;
pub(super) use shell_monitor::render_shell_monitor;
pub(super) use telegram_setup::render_telegram_setup;
