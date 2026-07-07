//! Mutating filesystem tools: `edit_file` (surgical) + `write_file` (whole-file).
//!
//! Both require user approval. The approval popup carries a coloured diff
//! preview built by `super::diff::{diff_edit, diff_write}`.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use super::diff::{diff_edit, diff_stats, diff_write};
use super::matchers::{find_all_matches, find_match, MatchRange, MatchResult};
use super::workspace::{read_text_file, resolve_in_workspace};
use super::{confirm, ToolContext};

/// Hard cap on `write_file` content + `edit_file` strings. Above this the
/// prompt and history get unwieldy and the model is almost certainly going
/// wrong anyway. 500 KB is enough for any single source file we'd realistically
/// edit; bigger pastes belong in a separate file-management workflow.
const MAX_WRITE_BYTES: usize = 500_000;

pub(super) async fn tool_edit_file(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("edit_file requires 'path'"))?;
    let old_string = args
        .get("old_string")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("edit_file requires 'old_string'"))?;
    let new_string = args
        .get("new_string")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("edit_file requires 'new_string'"))?;
    // Claude-Code-style escape hatch. Default off so a careless model can't
    // accidentally rewrite every `---` separator or every `## Heading` in
    // a markdown file — when the snippet is ambiguous the safe answer is
    // "expand context", not "replace everything".
    let replace_all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if old_string.is_empty() {
        bail!("edit_file: 'old_string' cannot be empty — use write_file to create a file");
    }
    if old_string == new_string {
        bail!("edit_file: 'old_string' and 'new_string' are identical");
    }
    if old_string.len() > MAX_WRITE_BYTES || new_string.len() > MAX_WRITE_BYTES {
        bail!("edit_file: strings exceed {MAX_WRITE_BYTES} byte cap");
    }

    let (resolved, content) = read_text_file(&ctx.workspace, path, "edit_file").await?;

    // Two paths through the matcher cascade:
    //   - replace_all: gather every match the first-hitting strategy
    //     returns, swap them all.
    //   - default: demand exactly one match across the cascade. A
    //     fuzzy strategy (line-trimmed, block-anchor) may rescue a
    //     snippet whose whitespace drifted from the source; the
    //     final span we replace is still byte-precise on the file.
    let (updated, replacements) = if replace_all {
        let ranges = find_all_matches(&content, old_string);
        if ranges.is_empty() {
            bail!(
                "edit_file: 'old_string' not found in {}. Read the file first to confirm exact \
                 text (whitespace, tabs vs spaces, trailing newlines all count).",
                path
            );
        }
        (replace_ranges(&content, &ranges, new_string), ranges.len())
    } else {
        match find_match(&content, old_string) {
            MatchResult::Unique(r) => {
                let mut out = String::with_capacity(content.len() + new_string.len());
                out.push_str(&content[..r.start]);
                out.push_str(new_string);
                out.push_str(&content[r.end..]);
                (out, 1)
            }
            MatchResult::Ambiguous { count, .. } => bail!(
                "edit_file: 'old_string' appears {count} times in {}. Either expand the snippet \
                 with surrounding context until it's unique, or set `replace_all: true` to \
                 change every instance (useful for renaming a variable across the file).",
                path
            ),
            MatchResult::NotFound => bail!(
                "edit_file: 'old_string' not found in {}. Read the file first to confirm exact \
                 text (whitespace, tabs vs spaces, trailing newlines all count). If the snippet \
                 spans multiple regions of the file, prefer apply_patch.",
                path
            ),
        }
    };

    let diff = diff_edit(old_string, new_string);
    let (added, removed) = diff_stats(&diff);
    let count_suffix = if replacements > 1 {
        format!(" · {replacements} replacements")
    } else {
        String::new()
    };
    let prompt = format!(
        "Edit file {} (+{added}L -{removed}L{count_suffix})",
        resolved.display(),
    );
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this edit)".into());
    }

    tokio::fs::write(&resolved, updated.as_bytes()).await?;
    Ok(format!(
        "edited {} ({} replacement{})",
        path,
        replacements,
        if replacements > 1 { "s" } else { "" }
    ))
}

/// Apply `new` at every range in `ranges` to `content`, returning the
/// rewritten string. Ranges must be non-overlapping and sorted by
/// start position — guaranteed by [`find_all_matches`], which scans
/// the content in order and never returns overlaps within a single
/// strategy's output.
fn replace_ranges(content: &str, ranges: &[MatchRange], new: &str) -> String {
    let mut out = String::with_capacity(content.len() + ranges.len() * new.len());
    let mut cursor = 0usize;
    for r in ranges {
        out.push_str(&content[cursor..r.start]);
        out.push_str(new);
        cursor = r.end;
    }
    out.push_str(&content[cursor..]);
    out
}

/// Batched surgical edit: apply N `{old_string, new_string}` pairs to the
/// same file in one call, one approval, one unified diff. Mirrors Claude
/// Code's `MultiEdit` so models trained on those traces reach for it
/// naturally instead of firing N separate `edit_file` calls.
///
/// Apply order matters — each `old_string` is matched against the file
/// state *after* the previous edits in the batch. All-or-nothing: if any
/// edit fails validation (snippet missing, ambiguous, empty, or a no-op),
/// nothing is written and the model gets a clear error pointing at the
/// failing index.
pub(super) async fn tool_multi_edit(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("multi_edit requires 'path'"))?;
    let edits = args
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("multi_edit requires 'edits' array"))?;
    if edits.is_empty() {
        bail!("multi_edit: 'edits' must contain at least one {{old_string, new_string}} pair");
    }

    let (resolved, original) = read_text_file(&ctx.workspace, path, "multi_edit").await?;

    // Apply in-memory first so a mid-batch failure leaves the file on disk
    // untouched. Each edit's `old_string` is re-matched against the running
    // `current` buffer, not the original — that's the contract that lets
    // later edits target text that earlier edits produced.
    let mut current = original.clone();
    for (i, edit) in edits.iter().enumerate() {
        let old = edit
            .get("old_string")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("multi_edit: edit #{i} missing 'old_string'"))?;
        let new = edit
            .get("new_string")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("multi_edit: edit #{i} missing 'new_string'"))?;
        // Per-edit Claude-Code-style escape hatch — `replace_all` lives on
        // each edit object so a single batch can mix unique-target edits
        // with rename-style edits without splitting into two tool calls.
        let replace_all = edit
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if old.is_empty() {
            bail!("multi_edit: edit #{i} has empty 'old_string' — use write_file to create a file");
        }
        if old == new {
            bail!("multi_edit: edit #{i} 'old_string' and 'new_string' are identical (no-op)");
        }
        if old.len() > MAX_WRITE_BYTES || new.len() > MAX_WRITE_BYTES {
            bail!("multi_edit: edit #{i} strings exceed {MAX_WRITE_BYTES} byte cap");
        }
        // Same cascade as `edit_file`. Each edit is independent —
        // the matcher runs against the running buffer state (post
        // prior edits), so a later edit can target text an earlier
        // edit produced.
        current = if replace_all {
            let ranges = find_all_matches(&current, old);
            if ranges.is_empty() {
                bail!(
                    "multi_edit: edit #{i} 'old_string' not found in {} (after prior edits). \
                     Either the snippet doesn't exist or a previous edit in this batch already \
                     rewrote that region — read the file and rebuild the batch.",
                    path
                );
            }
            replace_ranges(&current, &ranges, new)
        } else {
            match find_match(&current, old) {
                MatchResult::Unique(r) => {
                    let mut out = String::with_capacity(current.len() + new.len());
                    out.push_str(&current[..r.start]);
                    out.push_str(new);
                    out.push_str(&current[r.end..]);
                    out
                }
                MatchResult::Ambiguous { count, .. } => bail!(
                    "multi_edit: edit #{i} 'old_string' appears {count} times in {}. Either \
                     expand the snippet with surrounding context until it's unique, or set \
                     `replace_all: true` on this edit to change every instance.",
                    path
                ),
                MatchResult::NotFound => bail!(
                    "multi_edit: edit #{i} 'old_string' not found in {} (after prior edits). \
                     Either the snippet doesn't exist or a previous edit in this batch already \
                     rewrote that region — read the file and rebuild the batch. If the edit \
                     spans multiple regions, prefer apply_patch.",
                    path
                ),
            }
        };
    }

    // One confirm popup, one cumulative diff against the on-disk original.
    let diff = diff_write(Some(&original), &current);
    let (added, removed) = diff_stats(&diff);
    let prompt = format!(
        "Multi-edit {} (+{added}L -{removed}L · {} edits)",
        resolved.display(),
        edits.len()
    );
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this multi-edit)".into());
    }

    tokio::fs::write(&resolved, current.as_bytes()).await?;
    Ok(format!(
        "edited {} ({} replacements applied atomically)",
        path,
        edits.len()
    ))
}

pub(super) async fn tool_write_file(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("write_file requires 'path'"))?;
    let content = args
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("write_file requires 'content'"))?;

    if content.len() > MAX_WRITE_BYTES {
        bail!(
            "write_file: content is {} bytes; cap is {}. Split into multiple files or \
             reduce.",
            content.len(),
            MAX_WRITE_BYTES
        );
    }

    let resolved = resolve_in_workspace(&ctx.workspace, path)?;
    // resolve_in_workspace returns a path even if the file doesn't exist yet
    // (parent must exist). exists() lets us tell create vs overwrite in the
    // approval prompt — useful when the model thinks it's making a new file
    // but is actually about to clobber one.
    let prev_contents: Option<String> = if resolved.exists() {
        match tokio::fs::read(&resolved).await {
            Ok(bytes) => String::from_utf8(bytes).ok(),
            Err(_) => None,
        }
    } else {
        None
    };
    // Compute the diff first so the prompt summary uses line counts — same
    // `+NL -NL` shape as edit_file. For a CREATE the removed count is 0.
    let diff = diff_write(prev_contents.as_deref(), content);
    let (added, removed) = diff_stats(&diff);
    let action = if prev_contents.is_some() {
        "OVERWRITE"
    } else {
        "CREATE"
    };
    let prompt = format!("{} {} (+{added}L -{removed}L)", action, resolved.display());
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this write)".into());
    }

    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(&resolved, content.as_bytes()).await?;
    Ok(format!("wrote {} ({} bytes)", path, content.len()))
}
