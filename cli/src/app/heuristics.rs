//! Heuristics over chat history.
//!
//! Tiny pure helpers that scan `app.messages` to decide things like:
//! - did the last assistant turn call any tools?
//! - does the latest assistant message look like an "I'll do X" intent
//!   announcement (vs. actually acting)?
//! - did the last assistant turn end with a yes/no question that should
//!   arm the Y/N quick-reply?
//!
//! Kept separate from `event.rs` because they're stateless analysis, not
//! event-driven action, and they get exercised from multiple call-sites
//! (the Y/N intercept, the post-turn auto-continue logic, etc.).

use super::App;

impl App {
    /// True when no `tool` message has appeared since the last user
    /// message. Used by the auto-continue path to detect "model announced
    /// intent but didn't actually run any tools" turns.
    pub(super) fn no_tools_since_last_user(&self) -> bool {
        for m in self.messages.iter().rev() {
            if m.role == "user" {
                return true;
            }
            if m.role == "tool" {
                return false;
            }
        }
        true
    }

    /// Heuristic: did the latest assistant message announce intent rather
    /// than acting? ('I'll look at…', 'Let me check…', etc.)
    pub(super) fn looks_like_intent_announcement(&self) -> bool {
        const PATTERNS: &[&str] = &[
            "i'll",
            "let's",
            "let me",
            "i'm going to",
            "i am going to",
            "going to",
            "i will",
            "i shall",
        ];
        let last = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.trim().is_empty());
        let Some(m) = last else { return false };
        let lc = m.content.to_lowercase();
        PATTERNS.iter().any(|p| lc.contains(p))
    }

    /// True when the last assistant turn ended with one of the configured
    /// trigger phrases — at which point Y/N should fire the quick-reply.
    /// Inspects ONLY the last `?`-terminated sentence, and skips when that
    /// sentence is a WH-question (open-ended, not yes/no).
    pub(super) fn last_assistant_invites_yn(&self) -> bool {
        const TRIGGERS: &[&str] = &[
            "would you like",
            "would you want",
            "shall i",
            "shall we",
            "want me to",
            "want more",
            "should i",
            "should we",
            "do you want",
            "do you need",
            "any specific",
            "anything else",
            "anything specific",
            "more detail",
            "interested in",
            "let me know if",
            "let me know which",
            "which one",
            "which would",
        ];
        const WH_WORDS: &[&str] = &["what", "which", "who", "where", "when", "why", "how"];
        let last = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant" && !m.content.trim().is_empty());
        let Some(m) = last else { return false };
        let trimmed = m.content.trim_end();
        if !trimmed.ends_with('?') {
            return false;
        }
        // Extract the last sentence: walk back from the trailing `?` to the
        // previous sentence terminator (or start of string).
        let bytes = trimmed.as_bytes();
        let mut start = 0usize;
        // Skip the trailing `?` itself, then scan backwards.
        for i in (0..bytes.len().saturating_sub(1)).rev() {
            let b = bytes[i];
            if b == b'.' || b == b'!' || b == b'?' || b == b'\n' {
                start = i + 1;
                break;
            }
        }
        let sentence = trimmed[start..].trim();
        // Open-ended questions ("What…", "Which…", "How…") are not Y/N.
        if let Some(first) = sentence.split_whitespace().next() {
            let first_lc = first
                .trim_matches(|c: char| !c.is_alphabetic())
                .to_lowercase();
            if WH_WORDS.iter().any(|w| *w == first_lc) {
                return false;
            }
        }
        let lc = sentence.to_lowercase();
        TRIGGERS.iter().any(|t| lc.contains(t))
    }
}
