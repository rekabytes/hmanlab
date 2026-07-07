//! Thread-local memoization for the inline-markdown parse + word-wrap
//! pipeline. The chat renderer used to re-run `parse_inline_md` and
//! `wrap_styled_segments` for every visible paragraph on every frame —
//! cheap per call but it stacks up fast when a model streams tokens and
//! every chunk forces a full redraw of the transcript.
//!
//! The cache is keyed by `(content_hash, style_hash)` at a given
//! viewport width. On a window resize we toss the entire cache (every
//! width key would mismatch anyway) so we don't accumulate stale
//! entries. A coarse hard cap drops everything if the cache grows past
//! ~512 entries, which keeps memory bounded during a long streaming
//! session where the in-flight tail message changes content every chunk.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use ratatui::{style::Style, text::Span};

use super::markdown::{parse_inline_md, wrap_styled_segments};

thread_local! {
    static MD_CACHE: RefCell<Cache> = RefCell::new(Cache::default());
    static PLAIN_CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

#[derive(Default)]
struct Cache {
    entries: HashMap<u64, Vec<Vec<Span<'static>>>>,
    width: Option<usize>,
}

impl Cache {
    fn prepare(&mut self, width: usize) {
        if self.width != Some(width) {
            self.entries.clear();
            self.width = Some(width);
        }
        if self.entries.len() > 512 {
            self.entries.clear();
        }
    }
}

fn hash_str_style(text: &str, style: Style) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    style.hash(&mut h);
    h.finish()
}

fn clone_lines(src: &[Vec<Span<'static>>]) -> Vec<Vec<Span<'static>>> {
    src.to_vec()
}

/// Memoised `parse_inline_md` + `wrap_styled_segments`.
pub(super) fn wrap_md_paragraph(text: &str, base: Style, width: usize) -> Vec<Vec<Span<'static>>> {
    MD_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.prepare(width);
        let key = hash_str_style(text, base);
        if let Some(v) = c.entries.get(&key) {
            return clone_lines(v);
        }
        let segments = parse_inline_md(text, base);
        let wrapped = wrap_styled_segments(segments, width);
        let cloned = clone_lines(&wrapped);
        c.entries.insert(key, wrapped);
        cloned
    })
}

/// Memoised `wrap_styled_segments` for a single (text, style) chunk —
/// used for tool output / diff bodies where markdown parsing is
/// intentionally skipped.
pub(super) fn wrap_plain_paragraph(
    text: &str,
    style: Style,
    width: usize,
) -> Vec<Vec<Span<'static>>> {
    PLAIN_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        c.prepare(width);
        let key = hash_str_style(text, style);
        if let Some(v) = c.entries.get(&key) {
            return clone_lines(v);
        }
        let wrapped = wrap_styled_segments(vec![(text.to_string(), style)], width);
        let cloned = clone_lines(&wrapped);
        c.entries.insert(key, wrapped);
        cloned
    })
}
