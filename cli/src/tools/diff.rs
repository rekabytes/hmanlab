//! Coloured diff preview for the confirm popup.
//!
//! Used by `write::tool_edit_file` (diff_edit) and `write::tool_write_file`
//! (diff_write). Output is consumed by `ui::render_confirm` which maps
//! `DiffLineKind` to ratatui colours.

use similar::{ChangeTag, TextDiff};

/// What kind of marker each line in `ConfirmRequest::diff` carries.
/// The UI maps these to colors (Added → green, Removed → red, etc.).
#[derive(Clone, Copy, Debug)]
pub enum DiffLineKind {
    /// Context line surrounding the changes (rendered dim).
    Context,
    /// Inserted line.
    Added,
    /// Deleted line.
    Removed,
    /// Summary line (e.g. "+12 -3 lines").
    Summary,
}

/// One line of the styled diff preview shown in the confirm popup.
#[derive(Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// Cap on lines per side of the diff preview. Anything over this is
/// summarised so the popup never blows past one screen.
const MAX_DIFF_LINES: usize = 60;

/// Build a coloured diff for edit_file's old_string → new_string replacement.
///
/// We diff the two strings directly (not the surrounding file) because
/// edit_file's contract is "exactly these characters become exactly those" —
/// showing the broader file context would just dilute the signal.
pub(super) fn diff_edit(old: &str, new: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(old, new);
    let mut lines: Vec<DiffLine> = Vec::new();
    let (mut added, mut removed) = (0usize, 0usize);
    for change in diff.iter_all_changes() {
        let kind = match change.tag() {
            ChangeTag::Insert => {
                added += 1;
                DiffLineKind::Added
            }
            ChangeTag::Delete => {
                removed += 1;
                DiffLineKind::Removed
            }
            ChangeTag::Equal => DiffLineKind::Context,
        };
        let prefix = match kind {
            DiffLineKind::Added => "+ ",
            DiffLineKind::Removed => "- ",
            _ => "  ",
        };
        let mut text = change.to_string();
        // similar keeps the trailing newline on each change; the UI wraps
        // lines on its own, so strip it.
        if text.ends_with('\n') {
            text.pop();
        }
        lines.push(DiffLine {
            kind,
            text: format!("{prefix}{text}"),
        });
    }

    truncate_diff(lines, added, removed)
}

/// Build a coloured diff for write_file. CREATE = every line added; OVERWRITE
/// = real line-level diff between previous file contents and the new content.
pub(super) fn diff_write(prev: Option<&str>, new: &str) -> Vec<DiffLine> {
    let (added, removed, mut lines) = match prev {
        None => {
            // CREATE — render every line as added.
            let mut lines: Vec<DiffLine> = Vec::new();
            let mut added = 0usize;
            for raw in new.split_inclusive('\n') {
                let mut line = raw.to_string();
                if line.ends_with('\n') {
                    line.pop();
                }
                added += 1;
                lines.push(DiffLine {
                    kind: DiffLineKind::Added,
                    text: format!("+ {line}"),
                });
            }
            (added, 0, lines)
        }
        Some(prev_text) => {
            let diff = TextDiff::from_lines(prev_text, new);
            let mut lines: Vec<DiffLine> = Vec::new();
            let (mut added, mut removed) = (0usize, 0usize);
            for change in diff.iter_all_changes() {
                let kind = match change.tag() {
                    ChangeTag::Insert => {
                        added += 1;
                        DiffLineKind::Added
                    }
                    ChangeTag::Delete => {
                        removed += 1;
                        DiffLineKind::Removed
                    }
                    ChangeTag::Equal => DiffLineKind::Context,
                };
                let prefix = match kind {
                    DiffLineKind::Added => "+ ",
                    DiffLineKind::Removed => "- ",
                    _ => "  ",
                };
                let mut text = change.to_string();
                if text.ends_with('\n') {
                    text.pop();
                }
                lines.push(DiffLine {
                    kind,
                    text: format!("{prefix}{text}"),
                });
            }
            (added, removed, lines)
        }
    };
    // Prepend a single-line summary so the totals are always at the top
    // regardless of whether the diff body gets truncated below.
    let summary = DiffLine {
        kind: DiffLineKind::Summary,
        text: format!("+{added} -{removed} lines"),
    };
    let mut out = vec![summary];
    out.append(&mut lines);
    out = truncate_diff(out, added, removed);
    out
}

/// Count `(added, removed)` lines in a built diff. Used by the write
/// tools to put `+NL -NL` line totals in the confirm prompt instead of
/// raw byte counts — much more legible at a glance.
pub(super) fn diff_stats(diff: &[DiffLine]) -> (usize, usize) {
    let (mut added, mut removed) = (0usize, 0usize);
    for d in diff {
        match d.kind {
            DiffLineKind::Added => added += 1,
            DiffLineKind::Removed => removed += 1,
            DiffLineKind::Context | DiffLineKind::Summary => {}
        }
    }
    (added, removed)
}

/// Limit the diff to MAX_DIFF_LINES so the popup stays readable; replace the
/// overflow with a single "…(N more)" marker.
fn truncate_diff(lines: Vec<DiffLine>, added: usize, removed: usize) -> Vec<DiffLine> {
    if lines.len() <= MAX_DIFF_LINES {
        return lines;
    }
    let mut head: Vec<DiffLine> = lines.into_iter().take(MAX_DIFF_LINES).collect();
    head.push(DiffLine {
        kind: DiffLineKind::Summary,
        text: format!("…diff truncated — full change is +{added} -{removed} lines",),
    });
    head
}
