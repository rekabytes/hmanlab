//! Replacer strategies for `edit_file` / `multi_edit`.
//!
//! `find_match` walks a small cascade of progressively more permissive
//! matchers and returns the **byte range** in the original content
//! that an `old_string` snippet refers to. The strict contract is
//! preserved: the returned range is always a span of the original
//! content that — when literally swapped out for `new_string` — yields
//! the intended edit. Fuzziness lives in *how we find the span*, not
//! in *what we substitute*.
//!
//! Strategies, in order of strictness → permissiveness:
//!
//! 1. **Exact** — byte-for-byte substring search (the old default).
//! 2. **LineTrimmed** — trim trailing whitespace from every line of
//!    both `old` and the candidate window before comparing. Catches
//!    the common case where the model reproduces a snippet with
//!    one extra trailing space, or strips one the source had.
//! 3. **BlockAnchor** — anchor on the first and last non-empty lines
//!    of `old`; accept any middle content. Catches the case where
//!    the model reproduces the boundaries right but mangles middle
//!    lines (escaping drift inside code fences, e.g.).
//!
//! Each strategy still demands the final match be unambiguous. If
//! a strategy yields 0 matches we fall through to the next; if it
//! yields >1, the call is rejected as ambiguous (unless the caller
//! opted into `replace_all`).
//!
//! Modeled on OpenCode's `edit.ts` replacer cascade — we ship the
//! three highest-value strategies rather than the full 9, on the
//! theory that more fallbacks have rapidly diminishing returns and
//! every extra strategy is one more "magic match" failure mode.

/// One concrete span of `content` (in *bytes*) that a matcher
/// believes corresponds to an `old_string` snippet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchRange {
    pub start: usize,
    pub end: usize,
}

/// Outcome of running the cascade against one `(content, old)` pair.
#[derive(Clone, Debug, PartialEq)]
pub enum MatchResult {
    /// Exactly one strategy yielded exactly one match — safe to apply.
    Unique(MatchRange),
    /// At least one strategy yielded >1 candidate. Caller should
    /// either ask the model to expand context, or honor `replace_all`.
    Ambiguous {
        strategy: &'static str,
        count: usize,
    },
    /// No strategy found a match. Caller surfaces "not found".
    NotFound,
}

/// Walk the cascade and pick the first strategy that yields any
/// matches at all. We don't keep running once we get hits — that
/// would let a noisier strategy override the strict one when the
/// strict one was already definitive.
pub fn find_match(content: &str, old: &str) -> MatchResult {
    for (name, ranges_fn) in strategies() {
        let ranges = ranges_fn(content, old);
        if ranges.is_empty() {
            continue;
        }
        if ranges.len() == 1 {
            return MatchResult::Unique(ranges[0]);
        }
        return MatchResult::Ambiguous {
            strategy: name,
            count: ranges.len(),
        };
    }
    MatchResult::NotFound
}

/// Variant of [`find_match`] that returns *all* matches under the
/// first strategy that yields any. Used by `replace_all` callers —
/// they want every occurrence, not just the first.
pub fn find_all_matches(content: &str, old: &str) -> Vec<MatchRange> {
    for (_, ranges_fn) in strategies() {
        let ranges = ranges_fn(content, old);
        if !ranges.is_empty() {
            return ranges;
        }
    }
    Vec::new()
}

type Strategy = (&'static str, fn(&str, &str) -> Vec<MatchRange>);

fn strategies() -> &'static [Strategy] {
    &[
        ("exact", find_exact),
        ("line-trimmed", find_line_trimmed),
        ("block-anchor", find_block_anchor),
    ]
}

/// Byte-for-byte substring search. Returns every occurrence.
fn find_exact(content: &str, old: &str) -> Vec<MatchRange> {
    if old.is_empty() {
        return Vec::new();
    }
    content
        .match_indices(old)
        .map(|(start, m)| MatchRange {
            start,
            end: start + m.len(),
        })
        .collect()
}

/// Match by trimming trailing whitespace from every line of both
/// sides. Walks `content` line-by-line and checks whether the
/// normalised `old` lines appear as a contiguous run. The returned
/// `MatchRange` points at the ORIGINAL (un-trimmed) byte span so
/// the caller's `replace` substitutes the right slice.
fn find_line_trimmed(content: &str, old: &str) -> Vec<MatchRange> {
    let content_lines: Vec<(usize, &str)> = line_offsets(content);
    let old_lines: Vec<&str> = old.split('\n').collect();
    if old_lines.is_empty() {
        return Vec::new();
    }
    let trimmed_old: Vec<&str> = old_lines
        .iter()
        .map(|l| l.trim_end_matches([' ', '\t', '\r']))
        .collect();

    let mut hits = Vec::new();
    for start_i in 0..content_lines.len() {
        if start_i + old_lines.len() > content_lines.len() {
            break;
        }
        let mut ok = true;
        for j in 0..old_lines.len() {
            let cl = content_lines[start_i + j].1;
            let cl_trim = cl.trim_end_matches([' ', '\t', '\r']);
            if cl_trim != trimmed_old[j] {
                ok = false;
                break;
            }
        }
        if !ok {
            continue;
        }
        let start_byte = content_lines[start_i].0;
        let end_idx = start_i + old_lines.len() - 1;
        let last_line = content_lines[end_idx].1;
        let end_byte = content_lines[end_idx].0 + last_line.len();
        // If the match isn't byte-identical (it usually isn't, that's
        // the whole point of this strategy), skip it when the exact
        // strategy would already cover it — avoids double-counting a
        // match across both strategies.
        if &content[start_byte..end_byte] == old {
            continue;
        }
        hits.push(MatchRange {
            start: start_byte,
            end: end_byte,
        });
    }
    hits
}

/// Anchor on the first and last lines of `old`; accept any middle
/// content. Requires `old` to be at least 3 lines (otherwise the
/// anchors *are* the middle and exact would have already caught it).
/// The first-line anchor must be unique in the file for the strategy
/// to apply — if it appears multiple places, we don't try to guess
/// which closing anchor pairs with which opener.
fn find_block_anchor(content: &str, old: &str) -> Vec<MatchRange> {
    let old_lines: Vec<&str> = old.split('\n').collect();
    if old_lines.len() < 3 {
        return Vec::new();
    }
    let first = old_lines.first().copied().unwrap_or("").trim_end();
    let last = old_lines.last().copied().unwrap_or("").trim_end();
    if first.is_empty() || last.is_empty() {
        return Vec::new();
    }
    let content_lines = line_offsets(content);
    // Find every line that matches the first anchor (trimmed compare,
    // mirroring the line-trimmed strategy's tolerance).
    let first_positions: Vec<usize> = content_lines
        .iter()
        .enumerate()
        .filter_map(|(i, (_, l))| {
            if l.trim_end_matches([' ', '\t', '\r']) == first {
                Some(i)
            } else {
                None
            }
        })
        .collect();
    if first_positions.len() != 1 {
        // Multiple openers — we'd need a way to pair them with closers.
        // Bail and let the next/no strategy take over.
        return Vec::new();
    }
    let open_idx = first_positions[0];
    // Find the FIRST occurrence of the closing anchor after the opener.
    let close_idx = content_lines
        .iter()
        .enumerate()
        .skip(open_idx + 1)
        .find_map(|(i, (_, l))| (l.trim_end_matches([' ', '\t', '\r']) == last).then_some(i));
    let Some(close_idx) = close_idx else {
        return Vec::new();
    };
    let start_byte = content_lines[open_idx].0;
    let last_line = content_lines[close_idx].1;
    let end_byte = content_lines[close_idx].0 + last_line.len();
    let r = MatchRange {
        start: start_byte,
        end: end_byte,
    };
    // Same dedup as line-trimmed — if the span is byte-identical to
    // `old`, the exact strategy already had it.
    if &content[r.start..r.end] == old {
        return Vec::new();
    }
    vec![r]
}

/// Split `content` into lines, recording the byte offset of each
/// line's first character. Mirrors `str::lines()` semantics except
/// it carries the offsets we need to map fuzzy matches back to byte
/// positions in the original string.
fn line_offsets(content: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            // Strip a trailing \r if the file uses CRLF — same shape
            // `str::lines()` would hand back.
            let mut line_end = i;
            if line_end > start && content.as_bytes()[line_end - 1] == b'\r' {
                line_end -= 1;
            }
            out.push((start, &content[start..line_end]));
            start = i + 1;
        }
    }
    if start < content.len() {
        out.push((start, &content[start..]));
    } else if start == content.len() && content.ends_with('\n') {
        // File ends with a newline — there's no trailing "empty"
        // line in `lines()`-style enumeration, and we shouldn't
        // invent one (matchers would compare against "").
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_matches() {
        let c = "fn foo() {}\nfn bar() {}\n";
        let r = find_match(c, "fn foo() {}");
        assert_eq!(r, MatchResult::Unique(MatchRange { start: 0, end: 11 }));
    }

    #[test]
    fn exact_ambiguous() {
        let c = "---\nA\n---\nB\n---\n";
        let r = find_match(c, "---");
        match r {
            MatchResult::Ambiguous { count, .. } => assert_eq!(count, 3),
            other => panic!("expected ambiguous, got {other:?}"),
        }
    }

    #[test]
    fn line_trimmed_finds_trailing_space_drift() {
        // File has a trailing space; model's `old` snippet doesn't.
        let c = "let x = 1; \nlet y = 2;\n";
        let r = find_match(c, "let x = 1;\nlet y = 2;");
        // Exact strategy would miss this; line-trimmed finds it.
        match r {
            MatchResult::Unique(range) => {
                assert_eq!(&c[range.start..range.end], "let x = 1; \nlet y = 2;");
            }
            other => panic!("expected unique, got {other:?}"),
        }
    }

    #[test]
    fn block_anchor_with_drift_in_middle() {
        // First and last lines match; middle was reproduced with
        // a tab-vs-space drift that the line-trimmed strategy
        // wouldn't catch (it trims trailing only). Block anchor
        // accepts the middle as-is.
        let c = "// start\n    body\n// end\n";
        let r = find_match(c, "// start\n\tbody\n// end");
        match r {
            MatchResult::Unique(range) => {
                assert_eq!(&c[range.start..range.end], "// start\n    body\n// end");
            }
            other => panic!("expected unique, got {other:?}"),
        }
    }

    #[test]
    fn not_found_falls_through_all() {
        let c = "alpha\nbeta\ngamma\n";
        let r = find_match(c, "omega");
        assert_eq!(r, MatchResult::NotFound);
    }
}
