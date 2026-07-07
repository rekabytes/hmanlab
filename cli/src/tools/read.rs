//! Read-only filesystem tools: `read_file`, `list_dir`, `find_files`.
//!
//! None of these need user approval — they don't mutate anything. They DO
//! refuse to leave the workspace via `resolve_in_workspace`.

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::workspace::{resolve_in_workspace, truncate_utf8};
use super::ToolContext;

const MAX_FILE_BYTES: usize = 50_000;

/// Directories we never return from find_files. These are build caches, vendored
/// deps, and VCS metadata — useless noise that fills the context window.
const IGNORED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
    "__pycache__",
    ".venv",
    "venv",
    ".pytest_cache",
    ".mypy_cache",
    ".ruff_cache",
    "vendor",
    ".idea",
    ".vscode",
];

fn is_in_ignored_dir(rel: &std::path::Path) -> bool {
    rel.components().any(|c| {
        if let std::path::Component::Normal(name) = c {
            if let Some(s) = name.to_str() {
                return IGNORED_DIRS.contains(&s);
            }
        }
        false
    })
}

pub(super) async fn tool_read_file(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("read_file requires 'path'"))?;
    let resolved = resolve_in_workspace(&ctx.workspace, path)?;
    let bytes = tokio::fs::read(&resolved).await?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let truncated = truncate_utf8(text, MAX_FILE_BYTES);
    Ok(number_lines(&truncated))
}

/// Prefix every line with `<line_no>\t` so the model can identify
/// unique regions when forming `edit_file` / `multi_edit` snippets.
/// Mirrors Claude Code's `Read` output format — the model is
/// already trained to strip these prefixes before quoting content
/// into `old_string`, and the line numbers double as a coordinate
/// system the model can refer to in chat. The numbering reflects
/// what's actually shown (i.e. after truncation), not the
/// original file's line count, so the model never sees a number
/// that points past the visible content.
fn number_lines(s: &str) -> String {
    use std::fmt::Write;
    if s.is_empty() {
        return String::new();
    }
    // Estimate output size: original bytes + ~6 bytes/line for "NNNN\t".
    // `lines()` is O(n); reserving once avoids repeated re-allocs on big files.
    let line_count = s.lines().count();
    let mut out = String::with_capacity(s.len() + line_count * 6);
    for (i, line) in s.lines().enumerate() {
        let _ = writeln!(out, "{}\t{}", i + 1, line);
    }
    out
}

pub(super) async fn tool_list_dir(args: &Value, ctx: &ToolContext) -> Result<String> {
    let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let resolved = resolve_in_workspace(&ctx.workspace, path)?;
    let mut entries: Vec<String> = Vec::new();
    let mut dir = tokio::fs::read_dir(&resolved).await?;
    while let Some(e) = dir.next_entry().await? {
        let ft = e.file_type().await.ok();
        let name = e.file_name().to_string_lossy().to_string();
        let suffix = match ft {
            Some(t) if t.is_dir() => "/",
            Some(t) if t.is_symlink() => "@",
            _ => "",
        };
        entries.push(format!("{name}{suffix}"));
    }
    entries.sort();
    if entries.is_empty() {
        Ok("(empty directory)".into())
    } else {
        Ok(entries.join("\n"))
    }
}

pub(super) async fn tool_find_files(args: &Value, ctx: &ToolContext) -> Result<String> {
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("find_files requires 'pattern'"))?;
    let ws_canon = ctx
        .workspace
        .canonicalize()
        .unwrap_or_else(|_| ctx.workspace.clone());
    let full_pattern = if pattern.starts_with('/') {
        pattern.to_string()
    } else {
        format!("{}/{}", ws_canon.display(), pattern)
    };
    let mut found = Vec::new();
    let mut skipped = 0usize;
    for entry in glob::glob(&full_pattern).map_err(|e| anyhow!("bad glob: {e}"))? {
        let Ok(p) = entry else { continue };
        if !p.starts_with(&ws_canon) {
            continue;
        }
        let rel = p.strip_prefix(&ws_canon).unwrap_or(&p);
        if is_in_ignored_dir(rel) {
            skipped += 1;
            continue;
        }
        found.push(rel.display().to_string());
        if found.len() >= 100 {
            found.push("... (more matches truncated — refine the pattern)".into());
            break;
        }
    }
    if found.is_empty() {
        let note = if skipped > 0 {
            format!("(no matches; {skipped} hidden in build/cache dirs)")
        } else {
            "(no matches)".to_string()
        };
        Ok(note)
    } else {
        if skipped > 0 {
            found.push(format!(
                "(skipped {skipped} entries inside target/node_modules/.git/etc.)"
            ));
        }
        Ok(found.join("\n"))
    }
}
