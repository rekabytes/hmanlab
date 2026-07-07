//! Workspace path safety + shared output trimming.
//!
//! Anything that touches the filesystem goes through `resolve_in_workspace`
//! first — it canonicalises the input and refuses paths that escape the
//! workspace root. `truncate_utf8` caps tool output at a byte budget while
//! respecting UTF-8 char boundaries.

use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};

/// Shared cap for shell-like output (run_command, git_*). File reads have
/// their own larger cap in `read.rs`.
pub(super) const MAX_CMD_BYTES: usize = 4_000;

pub(super) fn resolve_in_workspace(workspace: &Path, input: &str) -> Result<PathBuf> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf()));
    }
    let p = Path::new(trimmed);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(p)
    };
    let ws_canon = workspace
        .canonicalize()
        .map_err(|e| anyhow!("workspace not accessible: {e}"))?;
    let abs_canon = match abs.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            // Path doesn't exist — resolve parent for the boundary check
            let parent = abs
                .parent()
                .ok_or_else(|| anyhow!("invalid path"))?
                .canonicalize()
                .map_err(|e| anyhow!("parent not accessible: {e}"))?;
            let name = abs
                .file_name()
                .ok_or_else(|| anyhow!("path has no file name component"))?;
            parent.join(name)
        }
    };
    if !abs_canon.starts_with(&ws_canon) {
        bail!(
            "path '{}' escapes the workspace ({})",
            input,
            ws_canon.display()
        );
    }
    Ok(abs_canon)
}

/// Resolve a path inside the workspace, read its bytes, and decode as
/// strict UTF-8. The shared entry point for any tool that needs to
/// load a file before mutating it (`edit_file`, `multi_edit`,
/// `apply_patch`, the `lines` family) — gathers the four steps that
/// previously appeared at every call site:
///   1. Workspace-boundary resolve.
///   2. Read bytes from disk.
///   3. Decode as strict UTF-8 (binary files are rejected — the
///      mutating tools operate on text only).
///   4. Wrap each failure with a `{tool}: ...` prefix so the model's
///      error mentions the tool it called, not a bare std error.
///
/// Returns the resolved absolute path + the file content. Callers
/// keep the path so they can write the result back to the same
/// location after the user approves the change.
pub(super) async fn read_text_file(
    workspace: &Path,
    path: &str,
    tool: &'static str,
) -> Result<(PathBuf, String)> {
    let resolved = resolve_in_workspace(workspace, path)?;
    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|e| anyhow!("{tool}: failed to read '{}': {e}", path))?;
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow!("{tool}: '{}' is not valid UTF-8", path))?;
    Ok((resolved, text))
}

pub(super) fn truncate_utf8(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    // Trim to a char boundary at or before `max`
    let mut cut = max;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let total = s.len();
    s.truncate(cut);
    s.push_str(&format!("\n\n... [truncated, {total} bytes total]"));
    s
}
