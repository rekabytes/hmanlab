//! Pure helpers shared by the chat renderer — no `App` mutation, no
//! `Frame` access. Keeping them here means the render functions stay
//! focused on layout, and these can be unit-tested in isolation.
//!
//! `thinking_breath` and `tool_breath` were removed when the breathing
//! indicators on thinking / tool cards were stripped from the chat
//! renderer. The remaining helpers are:
//!
//!   - `split_thinking` — strips `<think>…</think>` from the visible
//!     assistant body.
//!   - `tool_icon` / `tool_label` / `tool_arg_summary` — populate the
//!     collapsed tool-tile header (`📖 read_file · src/main.rs · 312 lines`).
//!   - `progress_bar` — the `▰▰▱▱▱ 3.2s` indicator on the active tool tile.
//!   - `ToolGroup` / `compute_tool_groups` / `summarize_group` /
//!     `group_noun` / `group_progress` — drive the multi-tool
//!     consolidation tile: a run of N consecutive tool messages from
//!     one assistant turn collapses into a single
//!     `◰×3 read 3 files · …` row instead of N standalone tiles.

use crate::ollama::ChatMessage;

/// Split an assistant message into its `<think>` reasoning block and the
/// visible answer. Qwen3's chat template *prepends* `<think>\n` to the
/// assistant prefix, so streamed output starts directly with reasoning
/// text and emits `</think>` once the model is ready to answer.
///
/// Returns `(thinking, visible)` where:
///   - `thinking` is `Some(text)` if the model produced any reasoning
///     content, `None` if the message has no thinking (or thinking is
///     empty).
///   - `visible` is the post-`</think>` answer.
///
/// While still streaming and `</think>` hasn't arrived yet, everything
/// so far is reasoning — we report `visible = ""` so the existing
/// "generating dots" branch renders progress without leaking raw
/// thoughts. Once generation finishes without ever emitting
/// `</think>`, we fall back to treating the whole content as visible
/// (legacy / non-reasoning models).
pub(super) fn split_thinking(s: &str, generating: bool) -> (Option<&str>, &str) {
    const CLOSE: &str = "</think>";
    const OPEN: &str = "<think>";
    if let Some(idx) = s.find(CLOSE) {
        let raw_think = &s[..idx];
        // Strip a leading "<think>" if present (some templates include it
        // in the streamed content rather than the prompt) plus
        // surrounding whitespace.
        let trimmed_think = raw_think
            .trim_start_matches(OPEN)
            .trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ');
        let after = &s[idx + CLOSE.len()..];
        // Strip stray leading markers that some models leak into the
        // visible slice:
        //   - duplicate `</think>` tags (e.g. `<think>...</think>`
        //     immediately followed by a stray `</think>`)
        //   - leading `<think>` (model emitted opening after closing —
        //     malformed but seen in the wild)
        //   - whitespace / newlines between the close tag and the
        //     answer
        // The loop is necessary because `trim_start_matches` only
        // strips a prefix that matches the *current* start of the
        // slice — so `\n</think>\nanswer` needs the newline stripped
        // first to expose the `</think>` to the next pass. Repeating
        // until the slice stops shrinking handles any interleaving of
        // whitespace + stray tags.
        let mut visible = after;
        loop {
            let prev_len = visible.len();
            visible = visible.trim_start();
            visible = visible.strip_prefix(CLOSE).unwrap_or(visible);
            visible = visible.strip_prefix(OPEN).unwrap_or(visible);
            if visible.len() == prev_len {
                break;
            }
        }
        if trimmed_think.is_empty() {
            (None, visible)
        } else {
            (Some(trimmed_think), visible)
        }
    } else if generating {
        // Mid-stream: thinking in progress, no answer yet. Hide content;
        // the generating-spinner branch will show a "…" placeholder.
        (None, "")
    } else {
        // Finished without a closing </think>: legacy / non-thinking
        // model. Render content as-is.
        (None, s)
    }
}

/// Single icon prepended to every tool tile. We use one glyph — ⚡ —
/// across the entire tool surface so the column reads as one visual
/// language instead of an icon zoo. The verb (`read`, `list`,
/// `shell`, …) carries the differentiation; the bolt just signals
/// "tool call" the way a play triangle signals "video".
///
/// The legacy per-tool geometric glyphs (◰ ▤ ◎ ⎇ ✎ ◧ $ ◉ → ⌕ ↧) read
/// as 90s-Unix and competed with the text for attention. Lightning
/// reads as fast/decisive/action — the vibe the agent surface wants.
/// Keep the function around (rather than inlining "⚡" at call sites)
/// so a future contributor has one knob to flip if they want to bring
/// back per-tool icons.
#[allow(dead_code)]
pub(super) fn tool_icon(_name: &str) -> &'static str {
    "⚡"
}

/// Pulsing brightness modifier for the lightning bolt on an active
/// tool. Cycles every `period` ticks between two intensities so the
/// bolt visibly breathes while the tool runs — reads as "alive",
/// unlike a static glyph that looks frozen.
///
/// Pass the renderer's `anim_tick` and a period in ticks (~120 ms
/// each). Returns a `Modifier` to OR into the bolt's `Style` (Dim
/// on the off-beat, nothing on the on-beat so the bold peach stays
/// full-intensity half the time).
pub(super) fn pulse_modifier(tick: u64, period: u64) -> ratatui::style::Modifier {
    if (tick / period.max(1)) % 2 == 1 {
        ratatui::style::Modifier::DIM
    } else {
        ratatui::style::Modifier::empty()
    }
}

/// Animated trailing dots for "still running" copy. Cycles
/// `.` → `..` → `...` → (space) every `period` ticks. Used as a
/// trailing flicker after an in-flight verb (`reading…` → `reading… .`)
/// to signal liveness without spinning a separate glyph.
pub(super) fn animated_ellipsis(tick: u64, period: u64) -> &'static str {
    let phase = (tick / period.max(1)) % 4;
    match phase {
        0 => ".",
        1 => "..",
        2 => "...",
        _ => " ",
    }
}

/// Bright-count badge that subtly breathes while a batch is in
/// flight. Returns the rendered string ("3", "2/3", etc.) — caller
/// styles it. Phase-shifted vs `pulse_modifier` so two pulsing things
/// on the same row don't beat in sync.
pub(super) fn breathing_count(completed: usize, total: usize, tick: u64) -> String {
    let _ = tick; // reserved for future per-frame width jitter
    if total > 0 && completed < total {
        format!("{completed}/{total}")
    } else {
        format!("{total}")
    }
}

/// Short label shown after the icon. Strips the `_file` / `_dir`
/// suffix for the common cases so the tile doesn't read like
/// `read_file · src/main.rs` (redundant). Falls back to the raw tool
/// name for unknown tools so a new tool never renders as blank.
pub(super) fn tool_label(name: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match name {
        "read_file" => Cow::Borrowed("read"),
        "list_dir" => Cow::Borrowed("list"),
        "find_files" => Cow::Borrowed("find"),
        "git_status" => Cow::Borrowed("git status"),
        "git_log" => Cow::Borrowed("git log"),
        "git_diff" => Cow::Borrowed("git diff"),
        "git_show" => Cow::Borrowed("git show"),
        "edit_file" => Cow::Borrowed("edit"),
        "multi_edit" => Cow::Borrowed("multi-edit"),
        "apply_patch" => Cow::Borrowed("patch"),
        "move_lines" => Cow::Borrowed("move"),
        "delete_lines" => Cow::Borrowed("delete"),
        "insert_at" => Cow::Borrowed("insert"),
        "write_file" => Cow::Borrowed("write"),
        "run_command" => Cow::Borrowed("shell"),
        "save_memory" => Cow::Borrowed("remember"),
        "read_memory" => Cow::Borrowed("recall"),
        "forget_memory" => Cow::Borrowed("forget"),
        "consult_specialist" => Cow::Borrowed("consult"),
        "web_search" => Cow::Borrowed("search"),
        "web_fetch" => Cow::Borrowed("fetch"),
        other => Cow::Borrowed(other),
    }
}

/// Pull the most informative field out of a tool's `args` JSON. The
/// goal is a one-line summary that tells the user *what* the tool
/// targeted without re-reading the full args blob. Falls back to a
/// compact JSON preview for tools we haven't specialised.
///
/// Reads and edits get the path; `read_file` additionally annotates
/// the line range so a 5,000-line file scan reads as
/// `src/main.rs · 312 lines` instead of just `src/main.rs`. Commands
/// are truncated to 40 chars so a long `cargo test …` doesn't
/// overflow the tile.
pub(super) fn tool_arg_summary(name: &str, args: &serde_json::Value) -> String {
    use serde_json::Value;
    let s = |k: &str| args.get(k).and_then(Value::as_str);
    let n = |k: &str| args.get(k).and_then(Value::as_u64);

    match name {
        "read_file" => match (s("path"), n("limit"), n("start_line")) {
            (Some(p), Some(limit), Some(start)) => {
                format!("{p} · {}–{}", start, start + limit)
            }
            (Some(p), Some(limit), None) => format!("{p} · {limit} lines"),
            (Some(p), None, _) => p.to_string(),
            _ => fallback_json(args),
        },
        "list_dir" | "edit_file" | "multi_edit" | "apply_patch" | "move_lines" | "delete_lines"
        | "insert_at" | "write_file" => s("path")
            .map(str::to_string)
            .unwrap_or_else(|| fallback_json(args)),
        "find_files" => s("pattern")
            .map(str::to_string)
            .unwrap_or_else(|| fallback_json(args)),
        "run_command" => s("command")
            .map(|c| truncate(c, 40))
            .unwrap_or_else(|| fallback_json(args)),
        "save_memory" | "read_memory" | "forget_memory" => s("key")
            .map(str::to_string)
            .unwrap_or_else(|| fallback_json(args)),
        "consult_specialist" => s("name")
            .map(str::to_string)
            .unwrap_or_else(|| fallback_json(args)),
        "web_search" => s("query")
            .map(str::to_string)
            .unwrap_or_else(|| fallback_json(args)),
        "web_fetch" => s("url")
            .map(str::to_string)
            .unwrap_or_else(|| fallback_json(args)),
        n if n.starts_with("git_") => match n {
            "git_status" => ".".into(),
            "git_log" => s("max_count")
                .map(|c| format!("last {c}"))
                .unwrap_or_else(|| "log".into()),
            "git_diff" | "git_show" => s("path")
                .or_else(|| s("ref"))
                .map(str::to_string)
                .unwrap_or_else(|| ".".into()),
            _ => fallback_json(args),
        },
        _ => fallback_json(args),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn fallback_json(args: &serde_json::Value) -> String {
    let s = args.to_string();
    truncate(&s, 60)
}

/// Render the running-tool indicator. Five segments, fills linearly
/// over `FULL_BAR_SECS` seconds and then sits at full. `elapsed_secs`
/// is the wall-clock time since the tool started (from
/// `App::active_tool_started_at`).
///
/// Six seconds reads as "this is taking a moment" without making the
/// bar feel sluggish on the common short-running tools (read_file,
/// git_status, list_dir all finish in well under a second).
pub(super) fn progress_bar(elapsed_secs: f32) -> String {
    const SEGMENTS: usize = 5;
    const FULL_BAR_SECS: f32 = 6.0;
    let filled = ((elapsed_secs / FULL_BAR_SECS) * SEGMENTS as f32)
        .round()
        .clamp(0.0, SEGMENTS as f32) as usize;
    let bar: String = (0..SEGMENTS)
        .map(|i| if i < filled { '▰' } else { '▱' })
        .collect();
    format!("{bar} {elapsed_secs:.1}s")
}

// ─── Multi-tool grouping (collapsed-batch tile) ────────────────────────
//
// When an assistant turn emits N tool calls, the agent loop runs them
// sequentially and pushes one `role: "tool"` message per call. Without
// grouping, that floods the chat with N near-identical rows:
//
//   ▎ ◰ read · src/main.rs · 312 lines   ⌄
//   ▎ ◰ read · src/lib.rs  · 145 lines   ⌄
//   ▎ ◰ read · src/mod.rs  · 89 lines    ⌄
//
// `compute_tool_groups` finds maximal runs of consecutive tool messages
// (a `permission` / `assistant` / `info` / hidden message naturally
// breaks a run — a confirm popup between tool calls is an intentional
// interruption). `summarize_group` then collapses the run into one
// header: same-verb batches unify to `◰×3 read 3 files · …`; mixed
// batches fall back to `3 calls · read 2 · list 1`. The header is
// clickable — expand reveals the individual tiles underneath with no
// loss of detail.

/// One maximal run of consecutive non-hidden tool messages. Built fresh
/// every frame from `app.messages`. A group of size 1 takes the
/// existing single-tile render path — only groups of size ≥ 2 hit the
/// consolidated tile.
#[derive(Clone, Copy, Debug)]
pub(super) struct ToolGroup {
    /// Inclusive start index into `app.messages`.
    pub start: usize,
    /// Inclusive end index into `app.messages`.
    pub end: usize,
}

impl ToolGroup {
    pub fn count(&self) -> usize {
        self.end.saturating_sub(self.start) + 1
    }
    /// True if `idx` lies within `[start, end]` (inclusive).
    pub fn contains(&self, idx: usize) -> bool {
        idx >= self.start && idx <= self.end
    }
}

/// Scan the message list for maximal runs of consecutive non-hidden
/// tool messages. Returned in order, no gaps. Each non-tool message
/// (or hidden one) naturally breaks a run.
pub(super) fn compute_tool_groups(messages: &[ChatMessage]) -> Vec<ToolGroup> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        if messages[i].role == "tool" && !messages[i].hidden {
            let start = i;
            while i < messages.len() && messages[i].role == "tool" && !messages[i].hidden {
                i += 1;
            }
            out.push(ToolGroup { start, end: i - 1 });
        } else {
            i += 1;
        }
    }
    out
}

/// Aggregated view of a tool group used by the collapsed tile. Tells
/// the renderer the dominant verb (so `read 3 files` reads cleaner
/// than `3 calls · read, read, read`), per-tool short summaries for
/// the trailing arg list, error stats for the icon color, and total
/// diff counts for write/edit batches.
pub(super) struct GroupSummary {
    /// True iff every tool in the group canonicalises to the same name.
    /// When true, the renderer can use the unified header
    /// (`◰×3 read 3 files`); when false, it falls back to a per-verb
    /// tally (`3 calls · read 2 · list 1`).
    pub uniform: bool,
    /// Canonical name shared by all members when `uniform`, else the
    /// first member's name (used only for icon fallback).
    pub canonical: String,
    /// Distinct canonical names present in the group, in first-seen
    /// order, with their counts. Drives the mixed-batch breakdown.
    pub verb_counts: Vec<(String, usize)>,
    /// Per-tool short summary (path, command, query, …) in group
    /// order. Empty string when the tool had no recognised arg.
    pub args: Vec<String>,
    /// Count of tools whose result content starts with `error:`.
    pub errors: usize,
    /// Total added lines across all write/edit tools in the group.
    pub total_adds: usize,
    /// Total removed lines.
    pub total_dels: usize,
}

pub(super) fn summarize_group(messages: &[ChatMessage], group: ToolGroup) -> GroupSummary {
    use crate::tools::DiffLineKind;

    let mut args = Vec::with_capacity(group.count());
    let mut errors = 0usize;
    let mut total_adds = 0usize;
    let mut total_dels = 0usize;
    // First-seen-order dedup of canonical names → count.
    let mut verb_counts: Vec<(String, usize)> = Vec::new();
    let mut first_canonical: Option<String> = None;

    for i in group.start..=group.end {
        let msg = &messages[i];
        let raw_name = msg.name.as_deref().unwrap_or("");
        let canonical = crate::tools::resolve_tool_alias(raw_name).to_string();
        if first_canonical.is_none() {
            first_canonical = Some(canonical.clone());
        }
        match verb_counts.iter_mut().find(|(n, _)| n == &canonical) {
            Some((_, c)) => *c += 1,
            None => verb_counts.push((canonical.clone(), 1)),
        }
        let arg_text = match msg.tool_args.as_ref() {
            Some(v) => tool_arg_summary(&canonical, v),
            None => msg.content.lines().next().unwrap_or("").to_string(),
        };
        args.push(arg_text);
        if msg.content.trim_start().starts_with("error:") {
            errors += 1;
        }
        if let Some(diff) = msg.diff.as_ref() {
            for line in diff {
                match line.kind {
                    DiffLineKind::Added => total_adds += 1,
                    DiffLineKind::Removed => total_dels += 1,
                    DiffLineKind::Context | DiffLineKind::Summary => {}
                }
            }
        }
    }

    let canonical = first_canonical.unwrap_or_default();
    let uniform = verb_counts.len() == 1;

    GroupSummary {
        uniform,
        canonical,
        verb_counts,
        args,
        errors,
        total_adds,
        total_dels,
    }
}

/// Plural noun for a same-verb batch's header line. "files" for reads,
/// "dirs" for listings, "edits" for write/edit, etc. Falls back to
/// "calls" for anything unmatched so the tile always reads grammatically
/// (`3 calls` rather than `3 read_files`).
pub(super) fn group_noun(canonical: &str) -> &'static str {
    match canonical {
        "read_file" => "files",
        "list_dir" => "dirs",
        "find_files" => "matches",
        "edit_file" | "multi_edit" | "apply_patch" | "move_lines" | "delete_lines"
        | "insert_at" | "write_file" => "edits",
        "run_command" => "commands",
        "save_memory" | "read_memory" | "forget_memory" => "memories",
        "git_status" | "git_log" | "git_diff" | "git_show" => "queries",
        "consult_specialist" => "consults",
        "web_search" => "searches",
        "web_fetch" => "fetches",
        _ => "calls",
    }
}

/// Live progress for an in-flight group. `completed` counts tool
/// messages whose result has landed (i.e. content is no longer the
/// `(running…` placeholder); `total` is the planned batch size pulled
/// from the preceding assistant message's `tool_calls`, falling back
/// to the current group size when the plan isn't available (e.g.
/// reloaded from a saved session without the trailing assistant).
#[derive(Clone, Copy, Debug)]
pub(super) struct GroupProgress {
    pub completed: usize,
    pub total: usize,
    /// True when `active_tool_msg_idx` is inside this group — i.e. one
    /// of the batch's tools is running right now.
    pub in_flight: bool,
}

pub(super) fn group_progress(
    messages: &[ChatMessage],
    group: ToolGroup,
    active_tool_msg_idx: Option<usize>,
) -> GroupProgress {
    let mut completed = 0usize;
    for i in group.start..=group.end {
        if !messages[i].content.trim_start().starts_with("(running") {
            completed += 1;
        }
    }
    let in_flight = matches!(active_tool_msg_idx, Some(idx) if group.contains(idx));

    // Walk back from the group's start to find the preceding non-tool
    // message. If it's an assistant with `tool_calls`, that's the
    // planned batch size. Otherwise we can't know — fall back to the
    // current count so the ratio never reads > 1.
    let planned = (0..group.start)
        .rev()
        .find_map(|i| {
            let m = &messages[i];
            if m.role == "tool" {
                None
            } else {
                Some(m)
            }
        })
        .and_then(|m| {
            if m.role == "assistant" {
                m.tool_calls.as_ref().map(|c| c.len())
            } else {
                None
            }
        })
        .unwrap_or_else(|| group.count());

    GroupProgress {
        completed,
        // If the plan named more tools than we've seen so far, trust
        // the plan. If we've somehow seen more than the plan (e.g.
        // plan was lost on session reload), trust the observed count.
        total: planned.max(group.count()),
        in_flight,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        animated_ellipsis, breathing_count, compute_tool_groups, group_noun, group_progress,
        progress_bar, pulse_modifier, split_thinking, summarize_group, tool_arg_summary, tool_icon,
        tool_label, ToolGroup,
    };
    use crate::ollama::ChatMessage;

    #[test]
    fn basic_split() {
        let (think, vis) = split_thinking("hello</think>world", false);
        assert_eq!(think, Some("hello"));
        assert_eq!(vis, "world");
    }

    #[test]
    fn leading_open_tag_stripped() {
        // Some templates include the opening tag in the streamed
        // content rather than the prompt.
        let (think, vis) = split_thinking("<think>reasoning</think>answer", false);
        assert_eq!(think, Some("reasoning"));
        assert_eq!(vis, "answer");
    }

    #[test]
    fn duplicate_close_tag_stripped_from_visible() {
        // MiniMax-M3 (and a few others) sometimes emit the closing
        // tag twice — once to end the thinking block, once again
        // before the actual answer (likely a chat-template artifact).
        // The first one is consumed by `s.find(CLOSE)`; the second
        // would otherwise leak into the visible slice as a stray
        // `</think>` line.
        let (think, vis) = split_thinking("thinking</think></think>answer", false);
        assert_eq!(think, Some("thinking"));
        assert_eq!(vis, "answer");
    }

    #[test]
    fn duplicate_close_tag_with_newline_leak() {
        // The actual bug from the screenshot — model emits the close
        // tag, then a newline, then a STRAY close tag, then the
        // answer. The first `</think>` is consumed by `s.find`;
        // `after` starts with `\n</think>\nanswer`. The fix has to
        // interleave whitespace stripping with tag stripping (a
        // single ordered pass leaves the second `</think>` exposed
        // because the leading `\n` blocks the tag-strip on pass 1).
        let (think, vis) = split_thinking("thinking</think>\n</think>\nactual answer", false);
        assert_eq!(think, Some("thinking"));
        assert_eq!(vis, "actual answer");
    }

    #[test]
    fn multiple_stray_close_tags_with_whitespace() {
        // Pathological case: model emits the close tag, then several
        // stray close tags separated by whitespace. The loop should
        // chew through all of them.
        let (think, vis) = split_thinking("reasoning</think>\n</think>\n  </think>\nanswer", false);
        assert_eq!(think, Some("reasoning"));
        assert_eq!(vis, "answer");
    }

    #[test]
    fn stragglers_only_kept_during_streaming() {
        // Mid-stream: model has emitted `<think>reasoning` but no
        // closing tag yet. Return empty visible so the generating
        // spinner takes over instead of leaking the partial tag.
        let (think, vis) = split_thinking("<think>reasoning in progress", true);
        assert_eq!(think, None);
        assert_eq!(vis, "");
    }

    #[test]
    fn no_tags_means_legacy_visible() {
        // Non-reasoning model finished without ever emitting a tag.
        let (think, vis) = split_thinking("just a plain answer", false);
        assert_eq!(think, None);
        assert_eq!(vis, "just a plain answer");
    }

    // ── tool_icon / tool_label ────────────────────────────────────
    #[test]
    fn icon_is_always_lightning() {
        // Single-icon design language — every tool gets ⚡. The verb
        // (`read`, `list`, `shell`) carries the differentiation; the
        // bolt just signals "tool call". Unknown tools get ⚡ too.
        assert_eq!(tool_icon("read_file"), "⚡");
        assert_eq!(tool_icon("run_command"), "⚡");
        assert_eq!(tool_icon("git_status"), "⚡");
        assert_eq!(tool_icon("edit_file"), "⚡");
        assert_eq!(tool_icon("write_file"), "⚡");
        assert_eq!(tool_icon("future_tool"), "⚡");
    }

    #[test]
    fn label_strips_redundant_suffix() {
        assert_eq!(tool_label("read_file"), "read");
        assert_eq!(tool_label("list_dir"), "list");
        // Multi-word tools keep their descriptive suffix.
        assert_eq!(tool_label("git_status"), "git status");
        // Unknown tool name round-trips verbatim.
        assert_eq!(tool_label("future_tool"), "future_tool");
    }

    // ── tool_arg_summary ──────────────────────────────────────────
    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn read_file_summary_includes_line_range() {
        assert_eq!(
            tool_arg_summary("read_file", &json(r#"{"path":"src/main.rs"}"#)),
            "src/main.rs"
        );
        assert_eq!(
            tool_arg_summary("read_file", &json(r#"{"path":"x.rs","limit":50}"#)),
            "x.rs · 50 lines"
        );
        assert_eq!(
            tool_arg_summary(
                "read_file",
                &json(r#"{"path":"x.rs","limit":50,"start_line":100}"#)
            ),
            "x.rs · 100–150"
        );
    }

    #[test]
    fn edit_and_write_summaries_show_path() {
        assert_eq!(
            tool_arg_summary("edit_file", &json(r#"{"path":"a.rs"}"#)),
            "a.rs"
        );
        assert_eq!(
            tool_arg_summary("write_file", &json(r#"{"path":"b.rs"}"#)),
            "b.rs"
        );
    }

    #[test]
    fn run_command_truncates_long_commands() {
        let long = "a".repeat(60);
        let v = json(&format!(r#"{{"command":"{long}"}}"#));
        let summary = tool_arg_summary("run_command", &v);
        // 39 chars + the ellipsis.
        assert_eq!(summary.chars().count(), 40);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn memory_tools_summarise_by_key() {
        assert_eq!(
            tool_arg_summary("save_memory", &json(r#"{"key":"foo"}"#)),
            "foo"
        );
    }

    #[test]
    fn unknown_tool_falls_back_to_json() {
        let v = json(r#"{"something":"else"}"#);
        assert_eq!(
            tool_arg_summary("future_tool", &v),
            r#"{"something":"else"}"#
        );
    }

    #[test]
    fn git_diff_picks_path_or_ref() {
        assert_eq!(
            tool_arg_summary("git_diff", &json(r#"{"path":"src/main.rs"}"#)),
            "src/main.rs"
        );
        assert_eq!(
            tool_arg_summary("git_diff", &json(r#"{"ref":"HEAD~1"}"#)),
            "HEAD~1"
        );
    }

    // ── progress_bar ──────────────────────────────────────────────
    #[test]
    fn progress_bar_fills_over_time() {
        assert_eq!(progress_bar(0.0), "▱▱▱▱▱ 0.0s");
        // 6s = full bar; 3s rounds to 3/5 (f32::round ties away from zero).
        assert_eq!(progress_bar(3.0), "▰▰▰▱▱ 3.0s");
        // Past 6s, the bar caps at full.
        assert_eq!(progress_bar(120.0), "▰▰▰▰▰ 120.0s");
    }

    // ── compute_tool_groups ───────────────────────────────────────
    fn tool_msg(name: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".into(),
            name: Some(name.into()),
            content: content.into(),
            ..Default::default()
        }
    }
    fn assistant_with_n_tool_calls(n: usize) -> ChatMessage {
        let calls: Vec<_> = (0..n)
            .map(|_| crate::ollama::ToolCall {
                function: crate::ollama::ToolCallFunction {
                    name: "read_file".into(),
                    arguments: serde_json::Value::Null,
                },
            })
            .collect();
        ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(calls),
            ..Default::default()
        }
    }

    #[test]
    fn groups_maximal_runs_of_tool_messages() {
        let msgs = vec![
            ChatMessage {
                role: "assistant".into(),
                ..Default::default()
            },
            tool_msg("read_file", "ok"),
            tool_msg("read_file", "ok"),
            tool_msg("list_dir", "ok"),
            ChatMessage {
                role: "assistant".into(),
                ..Default::default()
            },
            tool_msg("run_command", "done"),
        ];
        let groups = compute_tool_groups(&msgs);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].start, 1);
        assert_eq!(groups[0].end, 3);
        assert_eq!(groups[0].count(), 3);
        assert_eq!(groups[1].start, 5);
        assert_eq!(groups[1].count(), 1);
    }

    #[test]
    fn hidden_messages_break_a_run() {
        // A hidden tool message between two visible ones should split
        // the group rather than fuse them — hidden msgs are typically
        // system-injected Y/N quick-replies that don't belong to the
        // user-visible tool flow.
        let mut hidden = tool_msg("read_file", "ok");
        hidden.hidden = true;
        let msgs = vec![
            tool_msg("read_file", "ok"),
            hidden,
            tool_msg("read_file", "ok"),
        ];
        let groups = compute_tool_groups(&msgs);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|g| g.count() == 1));
    }

    #[test]
    fn permission_card_breaks_a_run() {
        // Even though a permission card is conceptually "inside" a
        // tool's execution, the message itself sits between tool
        // messages in chat history. It should break the group so the
        // card stays a visible interruption.
        let msgs = vec![
            tool_msg("edit_file", "done"),
            ChatMessage {
                role: "permission".into(),
                ..Default::default()
            },
            tool_msg("write_file", "done"),
        ];
        let groups = compute_tool_groups(&msgs);
        assert_eq!(groups.len(), 2);
    }

    // ── summarize_group ───────────────────────────────────────────
    #[test]
    fn uniform_batch_reports_single_verb() {
        let msgs = vec![
            tool_msg("read_file", "ok"),
            tool_msg("read_file", "ok"),
            tool_msg("read_file", "ok"),
        ];
        let g = ToolGroup { start: 0, end: 2 };
        let s = summarize_group(&msgs, g);
        assert!(s.uniform);
        assert_eq!(s.canonical, "read_file");
        assert_eq!(s.verb_counts, vec![("read_file".into(), 3)]);
        assert_eq!(s.errors, 0);
    }

    #[test]
    fn mixed_batch_breaks_down_by_verb() {
        let msgs = vec![
            tool_msg("read_file", "ok"),
            tool_msg("list_dir", "ok"),
            tool_msg("read_file", "ok"),
        ];
        let g = ToolGroup { start: 0, end: 2 };
        let s = summarize_group(&msgs, g);
        assert!(!s.uniform);
        // First-seen order: read_file (idx 0), list_dir (idx 1), then
        // read_file count increments the existing entry rather than
        // appending.
        assert_eq!(s.verb_counts.len(), 2);
        assert_eq!(s.verb_counts[0], ("read_file".into(), 2));
        assert_eq!(s.verb_counts[1], ("list_dir".into(), 1));
    }

    #[test]
    fn titlecase_aliases_canonicalise_before_grouping() {
        // Fine-tuned models emit Claude-Code-style names (`Read`,
        // `Bash`, ...). Two `Read` calls and one `read_file` should
        // canonicalise together as a uniform `read_file` batch.
        let msgs = vec![
            tool_msg("Read", "ok"),
            tool_msg("read_file", "ok"),
            tool_msg("Read", "ok"),
        ];
        let g = ToolGroup { start: 0, end: 2 };
        let s = summarize_group(&msgs, g);
        assert!(s.uniform, "expected uniform after alias resolution");
        assert_eq!(s.canonical, "read_file");
    }

    #[test]
    fn errors_are_counted_from_content_prefix() {
        let msgs = vec![
            tool_msg("read_file", "ok"),
            tool_msg("read_file", "error: file not found"),
            tool_msg("read_file", "ok"),
        ];
        let g = ToolGroup { start: 0, end: 2 };
        let s = summarize_group(&msgs, g);
        assert_eq!(s.errors, 1);
    }

    // ── group_progress ────────────────────────────────────────────
    #[test]
    fn progress_reads_total_from_preceding_assistant_tool_calls() {
        let msgs = vec![
            assistant_with_n_tool_calls(5),
            tool_msg("read_file", "done"),
            tool_msg("read_file", "done"),
        ];
        let g = ToolGroup { start: 1, end: 2 };
        let p = group_progress(&msgs, g, None);
        assert_eq!(p.completed, 2);
        assert_eq!(p.total, 5);
        assert!(!p.in_flight);
    }

    #[test]
    fn progress_marks_in_flight_when_active_idx_inside_group() {
        let msgs = vec![
            assistant_with_n_tool_calls(3),
            tool_msg("read_file", "done"),
            tool_msg("read_file", "(running…)"),
        ];
        let g = ToolGroup { start: 1, end: 2 };
        let p = group_progress(&msgs, g, Some(2));
        assert_eq!(p.completed, 1);
        assert_eq!(p.total, 3);
        assert!(p.in_flight);
    }

    #[test]
    fn progress_falls_back_to_observed_count_without_plan() {
        // Session reload case: tool messages exist but no preceding
        // assistant with `tool_calls`. Don't divide-by-zero — just
        // treat the observed count as the total.
        let msgs = vec![
            ChatMessage {
                role: "user".into(),
                content: "hi".into(),
                ..Default::default()
            },
            tool_msg("read_file", "done"),
            tool_msg("read_file", "done"),
        ];
        let g = ToolGroup { start: 1, end: 2 };
        let p = group_progress(&msgs, g, None);
        assert_eq!(p.total, 2);
        assert_eq!(p.completed, 2);
    }

    // ── group_noun ────────────────────────────────────────────────
    #[test]
    fn noun_picks_the_right_plural() {
        assert_eq!(group_noun("read_file"), "files");
        assert_eq!(group_noun("list_dir"), "dirs");
        assert_eq!(group_noun("run_command"), "commands");
        assert_eq!(group_noun("edit_file"), "edits");
        // Unknown tools fall back to "calls" so the tile still reads
        // grammatically (`3 calls`) rather than leaking the snake_case
        // internal name.
        assert_eq!(group_noun("future_tool"), "calls");
    }

    // ── pulse_modifier / animated_ellipsis / breathing_count ─────
    #[test]
    fn pulse_alternates_between_dim_and_clear() {
        use ratatui::style::Modifier;
        // Period 4: ticks 0-3 = clear (on-beat), 4-7 = dim (off-beat).
        assert_eq!(pulse_modifier(0, 4), Modifier::empty());
        assert_eq!(pulse_modifier(3, 4), Modifier::empty());
        assert_eq!(pulse_modifier(4, 4), Modifier::DIM);
        assert_eq!(pulse_modifier(7, 4), Modifier::DIM);
        assert_eq!(pulse_modifier(8, 4), Modifier::empty());
    }

    #[test]
    fn pulse_handles_zero_period_gracefully() {
        // A zero period must not divide-by-zero — fall through to the
        // "on" phase rather than panicking.
        use ratatui::style::Modifier;
        assert_eq!(pulse_modifier(0, 0), Modifier::empty());
    }

    #[test]
    fn ellipsis_cycles_four_phases() {
        // The four-phase cycle reads as a slow rightward shimmer.
        assert_eq!(animated_ellipsis(0, 3), ".");
        assert_eq!(animated_ellipsis(3, 3), "..");
        assert_eq!(animated_ellipsis(6, 3), "...");
        assert_eq!(animated_ellipsis(9, 3), " ");
        assert_eq!(animated_ellipsis(12, 3), ".");
    }

    #[test]
    fn breathing_count_shows_ratio_while_pending() {
        // While a batch is mid-flight: "completed/total" so the user
        // sees progress at a glance. Once done: just the count.
        assert_eq!(breathing_count(2, 3, 0), "2/3");
        assert_eq!(breathing_count(0, 5, 0), "0/5");
        // All done — drop the ratio, it's noise.
        assert_eq!(breathing_count(3, 3, 0), "3");
        assert_eq!(breathing_count(5, 5, 0), "5");
    }
}
