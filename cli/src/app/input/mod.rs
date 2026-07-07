//! Input event handling — keyboard, mouse, and the bits of state they
//! poke. Split by surface:
//!
//!   - `mouse`  — pointer events, sidebar clicks, tool-row toggle,
//!     drag-select → clipboard.
//!   - `modals` — key handling for popups that briefly own the screen
//!     (model picker, confirm dialog, session picker, file viewer).
//!   - `chat`   — the main chat-mode keymap: Ctrl shortcuts, Enter/submit,
//!     inline `/` and `@` autocomplete, soft-wrap, etc.
//!
//! The top-level dispatcher (`App::handle_event`) lives in `event.rs` and
//! routes each event to one of these handlers based on `self.mode` /
//! event kind.

mod chat;
mod modals;
mod mouse;
