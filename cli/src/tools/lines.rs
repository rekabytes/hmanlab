//! Coordinate-based file ops: `move_lines`, `delete_lines`, `insert_at`.
//!
//! These tools take **line numbers** instead of file content, so the
//! model never has to reproduce existing text as a tool-call argument.
//! That sidesteps the failure pattern where `edit_file` / `multi_edit`
//! / `apply_patch` all break: long verbatim reproduction is unreliable
//! (dropped list markers, normalised whitespace, output-token
//! truncation), and every reproduction error stalls the edit.
//!
//! Pairs with the line-numbered `read_file` output the model already
//! sees — Read tells the model "section X lives on lines 60–72",
//! `move_lines` lets the model act on that with just integers. Best
//! primitive for section reorders, structural cleanups, deletions of
//! known blocks. Worst primitive for content-changing edits — those
//! still want `edit_file` (search-and-replace semantics).
//!
//! All ops:
//!   - 1-indexed, end-inclusive ranges (matches Read's display).
//!   - Validate bounds before touching anything.
//!   - Produce a diff that flows through the existing confirm popup.
//!   - Are atomic in memory before any disk write.
//!   - Preserve the file's trailing-newline shape.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use super::diff::{diff_edit, diff_stats};
use super::workspace::read_text_file;
use super::{confirm, ToolContext};

/// Hard cap on `insert_at` content. Same threshold as `write_file`'s
/// other string caps — past this the model is almost certainly doing
/// something it should have split into smaller calls.
const MAX_INSERT_BYTES: usize = 500_000;

pub(super) async fn tool_move_lines(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = require_str(args, "path", "move_lines")?;
    let from_start = require_u64(args, "from_start", "move_lines")? as usize;
    let from_end = require_u64(args, "from_end", "move_lines")? as usize;
    let to_before = require_u64(args, "to_before", "move_lines")? as usize;

    let (resolved, original) = read_text_file(&ctx.workspace, path, "move_lines").await?;

    let mut parts: Vec<&str> = original.split('\n').collect();
    let lc = logical_line_count(&parts);

    if from_start < 1 || from_end < 1 {
        bail!("move_lines: line numbers are 1-indexed; got from_start={from_start}, from_end={from_end}");
    }
    if from_start > from_end {
        bail!("move_lines: from_start ({from_start}) must be <= from_end ({from_end})");
    }
    if from_end > lc {
        bail!(
            "move_lines: from_end ({from_end}) is past the end of {} (file has {lc} lines). \
             Read the file again — line numbers may have shifted from earlier edits.",
            path
        );
    }
    if to_before < 1 || to_before > lc + 1 {
        bail!(
            "move_lines: to_before ({to_before}) must be between 1 and {} (line_count + 1). \
             Use {} to append after the last line.",
            lc + 1,
            lc + 1
        );
    }
    if to_before >= from_start && to_before <= from_end + 1 {
        // Moving the block onto itself (or immediately after itself)
        // is a no-op; tell the model so it knows the move didn't
        // need to happen rather than failing silently.
        bail!(
            "move_lines: to_before ({to_before}) is inside or immediately after the moved block \
             ({from_start}..={from_end}) — that's a no-op. Pick a target outside the source range."
        );
    }

    // 0-indexed slice positions in `parts`.
    let block_start = from_start - 1;
    let block_end_excl = from_end; // half-open
    let block: Vec<String> = parts[block_start..block_end_excl]
        .iter()
        .map(|s| s.to_string())
        .collect();

    // Position-shift logic: when the insertion target sits BEFORE the
    // source block, the post-removal coordinate is unchanged; when it
    // sits AFTER, it shifts left by the block size.
    let target_0idx = to_before - 1;
    let insert_at = if target_0idx <= block_start {
        target_0idx
    } else {
        // We already rejected the inside-block case above, so this
        // branch only fires when target_0idx > block_end_excl - 1.
        target_0idx - block.len()
    };

    parts.drain(block_start..block_end_excl);
    for (i, line) in block.iter().enumerate() {
        parts.insert(insert_at + i, line);
    }
    let updated = parts.join("\n");

    apply_with_confirm(
        ctx,
        &resolved,
        &original,
        &updated,
        format!(
            "move_lines {} (lines {from_start}–{from_end} → before line {to_before})",
            resolved.display()
        ),
    )
    .await
    .map(|r| {
        r.unwrap_or_else(|| {
            format!(
                "moved lines {from_start}..={from_end} before line {to_before} in {}",
                path
            )
        })
    })
}

pub(super) async fn tool_delete_lines(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = require_str(args, "path", "delete_lines")?;
    let start = require_u64(args, "start", "delete_lines")? as usize;
    let end = require_u64(args, "end", "delete_lines")? as usize;

    let (resolved, original) = read_text_file(&ctx.workspace, path, "delete_lines").await?;

    let mut parts: Vec<&str> = original.split('\n').collect();
    let lc = logical_line_count(&parts);

    if start < 1 || end < 1 {
        bail!("delete_lines: line numbers are 1-indexed; got start={start}, end={end}");
    }
    if start > end {
        bail!("delete_lines: start ({start}) must be <= end ({end})");
    }
    if end > lc {
        bail!(
            "delete_lines: end ({end}) is past the end of {} (file has {lc} lines).",
            path
        );
    }
    parts.drain(start - 1..end);
    let updated = parts.join("\n");

    apply_with_confirm(
        ctx,
        &resolved,
        &original,
        &updated,
        format!("delete_lines {} (lines {start}–{end})", resolved.display()),
    )
    .await
    .map(|r| {
        r.unwrap_or_else(|| {
            let removed = end - start + 1;
            format!(
                "deleted {removed} line{} from {}",
                if removed == 1 { "" } else { "s" },
                path
            )
        })
    })
}

pub(super) async fn tool_insert_at(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = require_str(args, "path", "insert_at")?;
    let before_line = require_u64(args, "before_line", "insert_at")? as usize;
    let content = require_str(args, "content", "insert_at")?;

    if content.is_empty() {
        bail!("insert_at: 'content' is empty — nothing to insert");
    }
    if content.len() > MAX_INSERT_BYTES {
        bail!(
            "insert_at: content is {} bytes; cap is {MAX_INSERT_BYTES}",
            content.len()
        );
    }

    let (resolved, original) = read_text_file(&ctx.workspace, path, "insert_at").await?;

    let mut parts: Vec<&str> = original.split('\n').collect();
    let lc = logical_line_count(&parts);

    if before_line < 1 || before_line > lc + 1 {
        bail!(
            "insert_at: before_line ({before_line}) must be between 1 and {} (line_count + 1). \
             Use {} to append after the last line.",
            lc + 1,
            lc + 1
        );
    }

    // Split `content` into lines. Drop one trailing empty (so callers
    // can pass either "x\ny" or "x\ny\n" and get the same logical
    // result — a single trailing newline on `content` is conventional
    // and shouldn't smuggle in a blank line).
    let mut new_lines: Vec<&str> = content.split('\n').collect();
    if new_lines.last() == Some(&"") {
        new_lines.pop();
    }
    if new_lines.is_empty() {
        bail!("insert_at: 'content' had only a newline — nothing to insert");
    }

    let pos = before_line - 1;
    for (i, line) in new_lines.iter().enumerate() {
        parts.insert(pos + i, line);
    }
    let updated = parts.join("\n");

    apply_with_confirm(
        ctx,
        &resolved,
        &original,
        &updated,
        format!(
            "insert_at {} (before line {before_line}, +{} line{})",
            resolved.display(),
            new_lines.len(),
            if new_lines.len() == 1 { "" } else { "s" }
        ),
    )
    .await
    .map(|r| {
        r.unwrap_or_else(|| {
            format!(
                "inserted {} line{} into {} before line {before_line}",
                new_lines.len(),
                if new_lines.len() == 1 { "" } else { "s" },
                path
            )
        })
    })
}

/// Shared diff-and-confirm path for the three ops. Builds a coloured
/// diff between `original` and `updated`, drives it through the
/// existing confirm popup, and writes on approval. Returns `Ok(None)`
/// when there's nothing to do (no-op) so the caller can compose a
/// natural success message instead.
async fn apply_with_confirm(
    ctx: &ToolContext,
    resolved: &std::path::Path,
    original: &str,
    updated: &str,
    prompt_prefix: String,
) -> Result<Option<String>> {
    if updated == original {
        return Ok(Some("(no-op — file already in the requested state)".into()));
    }
    let diff = diff_edit(original, updated);
    let (added, removed) = diff_stats(&diff);
    let prompt = format!("{prompt_prefix} +{added}L -{removed}L");
    if !confirm(ctx, prompt, diff).await? {
        return Ok(Some("(user denied this line edit)".into()));
    }
    tokio::fs::write(resolved, updated.as_bytes())
        .await
        .map_err(|e| anyhow!("write failed for {}: {e}", resolved.display()))?;
    Ok(None)
}

/// Number of "logical" lines — what the user / model would call line
/// 1 through line N. The trailing empty string `split('\n')` produces
/// for a file ending in `\n` is NOT counted; that's the file's
/// trailing-newline marker, not an addressable line.
fn logical_line_count(parts: &[&str]) -> usize {
    if parts.last() == Some(&"") {
        parts.len() - 1
    } else {
        parts.len()
    }
}

fn require_str<'a>(args: &'a Value, key: &str, tool: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("{tool} requires '{key}' (string)"))
}

fn require_u64(args: &Value, key: &str, tool: &str) -> Result<u64> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("{tool} requires '{key}' (positive integer, 1-indexed)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(s: &str) -> Vec<&str> {
        s.split('\n').collect()
    }

    #[test]
    fn line_count_with_trailing_newline() {
        assert_eq!(logical_line_count(&split("a\nb\nc\n")), 3);
        assert_eq!(logical_line_count(&split("a\nb\nc")), 3);
        // `"".split('\n')` yields a single empty part — the trailing-empty
        // rule treats that as 0 logical lines, matching how the model would
        // see it ("the file has nothing in it").
        assert_eq!(logical_line_count(&split("")), 0);
        assert_eq!(logical_line_count(&split("only\n")), 1);
        assert_eq!(logical_line_count(&split("only")), 1);
    }

    /// Manually re-implement the move logic to spec-check it against
    /// known cases. Mirrors the production path in `tool_move_lines`
    /// (without the I/O and confirm) so the math is testable.
    fn move_pure(original: &str, from_start: usize, from_end: usize, to_before: usize) -> String {
        let mut parts: Vec<&str> = original.split('\n').collect();
        let block_start = from_start - 1;
        let block_end_excl = from_end;
        let block: Vec<String> = parts[block_start..block_end_excl]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let target_0idx = to_before - 1;
        let insert_at = if target_0idx <= block_start {
            target_0idx
        } else {
            target_0idx - block.len()
        };
        parts.drain(block_start..block_end_excl);
        for (i, line) in block.iter().enumerate() {
            parts.insert(insert_at + i, line);
        }
        parts.join("\n")
    }

    #[test]
    fn move_section_to_top() {
        // Real-world case: "swap Install above Features" on a tiny
        // file. Install lives on lines 5-6, Features on lines 2-3.
        let file = "title\nFeatures\nbody1\n\nInstall\nbody2\n";
        // Move lines 5-6 (Install + body2) before line 2 (Features).
        let out = move_pure(file, 5, 6, 2);
        assert_eq!(out, "title\nInstall\nbody2\nFeatures\nbody1\n\n");
    }

    #[test]
    fn move_last_line_to_front() {
        let file = "a\nb\nc";
        let out = move_pure(file, 3, 3, 1);
        assert_eq!(out, "c\na\nb");
    }

    #[test]
    fn move_preserves_trailing_newline() {
        let file = "a\nb\nc\n";
        let out = move_pure(file, 1, 1, 4); // move "a" to end
        assert_eq!(out, "b\nc\na\n");
    }

    #[test]
    fn delete_middle_lines() {
        let mut parts: Vec<&str> = "a\nb\nc\nd\ne\n".split('\n').collect();
        parts.drain(1..3); // lines 2-3
        assert_eq!(parts.join("\n"), "a\nd\ne\n");
    }

    #[test]
    fn insert_at_start() {
        let mut parts: Vec<&str> = "a\nb\nc\n".split('\n').collect();
        let new = ["x", "y"];
        for (i, l) in new.iter().enumerate() {
            parts.insert(i, l);
        }
        assert_eq!(parts.join("\n"), "x\ny\na\nb\nc\n");
    }

    #[test]
    fn insert_at_end_appends() {
        // before_line = line_count + 1 means "append after last".
        let mut parts: Vec<&str> = "a\nb\nc\n".split('\n').collect();
        let lc = logical_line_count(&parts);
        let pos = (lc + 1) - 1; // = 3, the trailing "" position
        parts.insert(pos, "d");
        assert_eq!(parts.join("\n"), "a\nb\nc\nd\n");
    }
}
