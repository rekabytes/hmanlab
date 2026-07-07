//! The main chat surface — message history (`render_chat`) and input box
//! (`render_input`). Split for readability:
//!
//!   - `messages` — the big `render_chat` loop that assembles every
//!     visible row (assistant, user, single-tool tiles, multi-tool
//!     group tiles, hover overlay, selection overlay).
//!   - `input`    — `render_input`, the bottom textarea with mode-aware
//!     border / title.
//!   - `helpers`  — small pure helpers used by both: tool-call
//!     summarisation, multi-tool grouping (`compute_tool_groups`,
//!     `summarize_group`, `group_progress`), breath animations, etc.
//!
//! `ui::mod` only sees `render_chat` and `render_input`; everything else
//! is private to this directory.

mod helpers;
mod input;
mod messages;

pub(super) use input::render_input;
pub(super) use messages::render_chat;
