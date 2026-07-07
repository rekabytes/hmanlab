//! `apply_patch` — V4A (Codex) envelope-style patches.
//!
//! Lets the model express multi-region rearrangements as a single
//! atomic operation instead of forcing the deletion-here +
//! insertion-there shape through `edit_file` / `multi_edit`. The
//! model emits a textual envelope:
//!
//! ```text
//! *** Begin Patch
//! *** Update File: README.md
//! @@ ## Features
//! -## Features
//! - ...
//! -## Install
//! - ...
//! +## Install
//! + ...
//! +## Features
//! + ...
//! *** End Patch
//! ```
//!
//! Each `@@` hunk is reduced to a find-and-replace against the
//! running file buffer — context+remove lines form the search
//! string, context+add lines form the replacement. We hand that
//! through the existing matcher cascade from `matchers.rs`, so all
//! the whitespace-drift tolerances `edit_file` got in phase A
//! apply here too.
//!
//! Operations supported: `Add File`, `Update File`, `Delete File`.
//! `*** Move to:` (rename during update) is parsed but rejected at
//! apply time for v1 — getting rename right with the confirm/diff
//! popup needs a separate UX pass.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use super::diff::{diff_edit, diff_stats, DiffLine, DiffLineKind};
use super::matchers::{find_match, MatchResult};
use super::workspace::{read_text_file, resolve_in_workspace};
use super::{confirm, ToolContext};

/// One top-level operation parsed out of a patch envelope.
#[derive(Debug, Clone)]
enum FileOp {
    Add {
        path: String,
        content: String,
    },
    Update {
        path: String,
        hunks: Vec<Hunk>,
        move_to: Option<String>,
    },
    Delete {
        path: String,
    },
}

#[derive(Debug, Clone, Default)]
struct Hunk {
    /// Optional anchor (the text after `@@ `). Currently advisory
    /// only — used in the error message for failed hunks so the
    /// model can locate which one was rejected, but not consulted
    /// for matching.
    anchor: Option<String>,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

/// Tool entry. Parses the patch, validates and applies all ops in
/// memory first, builds one cumulative diff for the confirm popup,
/// and only writes to disk after the user approves.
pub(super) async fn tool_apply_patch(args: &Value, ctx: &ToolContext) -> Result<String> {
    let patch_text = args
        .get("patch")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("apply_patch requires 'patch'"))?;

    let ops = parse_patch(patch_text)?;
    if ops.is_empty() {
        bail!("apply_patch: patch contained no file operations");
    }

    // Stage every effect in memory. `new_files` carries the
    // post-apply content for Adds / Updates; `deletes` collects
    // paths to remove on the write pass.
    let mut new_files: HashMap<PathBuf, String> = HashMap::new();
    let mut deletes: Vec<PathBuf> = Vec::new();
    let mut diff_acc: Vec<DiffLine> = Vec::new();
    let mut summary_paths: Vec<String> = Vec::new();

    for op in &ops {
        match op {
            FileOp::Add { path, content } => {
                let resolved = resolve_in_workspace(&ctx.workspace, path)?;
                if tokio::fs::try_exists(&resolved).await.unwrap_or(false) {
                    bail!(
                        "apply_patch: Add File '{}' rejected — target already exists. \
                         Use *** Update File: ... instead.",
                        path
                    );
                }
                push_file_header(&mut diff_acc, &format!("+++ {} (added)", path));
                for line in content.lines() {
                    diff_acc.push(DiffLine {
                        kind: DiffLineKind::Added,
                        text: format!("+{line}"),
                    });
                }
                new_files.insert(resolved, content.clone());
                summary_paths.push(format!("+{path}"));
            }
            FileOp::Delete { path } => {
                let resolved = resolve_in_workspace(&ctx.workspace, path)?;
                if !tokio::fs::try_exists(&resolved).await.unwrap_or(false) {
                    bail!(
                        "apply_patch: Delete File '{}' rejected — target does not exist.",
                        path
                    );
                }
                push_file_header(&mut diff_acc, &format!("--- {} (deleted)", path));
                deletes.push(resolved);
                summary_paths.push(format!("-{path}"));
            }
            FileOp::Update {
                path,
                hunks,
                move_to,
            } => {
                if move_to.is_some() {
                    bail!(
                        "apply_patch: '*** Move to: ...' is not yet supported. Issue a separate \
                         Add + Delete pair, or use run_command to rename."
                    );
                }
                let (resolved, original) =
                    read_text_file(&ctx.workspace, path, "apply_patch").await?;
                // Each hunk reduces to a find-and-replace against the
                // running buffer. The matcher cascade does the heavy
                // lifting — whitespace-drift tolerance comes for free.
                let mut current = original.clone();
                for (h_idx, hunk) in hunks.iter().enumerate() {
                    let (old_text, new_text) = hunk_to_replace_pair(hunk);
                    if old_text.is_empty() && new_text.is_empty() {
                        // Empty hunk — patch bug, but tolerate it.
                        continue;
                    }
                    if old_text.is_empty() {
                        bail!(
                            "apply_patch: hunk #{h_idx} in '{}' has only additions and no \
                             context — anchor your additions with at least one nearby context \
                             line so we can locate the insertion point.",
                            path
                        );
                    }
                    match find_match(&current, &old_text) {
                        MatchResult::Unique(r) => {
                            let mut out = String::with_capacity(current.len() + new_text.len());
                            out.push_str(&current[..r.start]);
                            out.push_str(&new_text);
                            out.push_str(&current[r.end..]);
                            current = out;
                        }
                        MatchResult::Ambiguous { count, .. } => {
                            let anchor_hint = hunk
                                .anchor
                                .as_deref()
                                .map(|a| format!(" (anchor: '{a}')"))
                                .unwrap_or_default();
                            bail!(
                                "apply_patch: hunk #{h_idx} in '{}'{anchor_hint} matched \
                                 {count} locations. Add more context lines (unchanged ' ' \
                                 prefixed lines) around the change so the hunk is unique.",
                                path
                            );
                        }
                        MatchResult::NotFound => {
                            let anchor_hint = hunk
                                .anchor
                                .as_deref()
                                .map(|a| format!(" (anchor: '{a}')"))
                                .unwrap_or_default();
                            bail!(
                                "apply_patch: hunk #{h_idx} in '{}'{anchor_hint} did not match \
                                 the current file state. Re-read the file and rebuild the \
                                 patch — the context+remove lines must appear together in the \
                                 file.",
                                path
                            );
                        }
                    }
                }
                if current == original {
                    // No change after applying — model gave a no-op patch.
                    continue;
                }
                push_file_header(&mut diff_acc, &format!("~~~ {} (updated)", path));
                let file_diff = diff_edit(&original, &current);
                diff_acc.extend(file_diff);
                new_files.insert(resolved, current);
                summary_paths.push(format!("~{path}"));
            }
        }
    }

    if new_files.is_empty() && deletes.is_empty() {
        return Ok("apply_patch: patch produced no changes".into());
    }

    let (added, removed) = diff_stats(&diff_acc);
    let prompt = format!(
        "Apply patch ({}) +{added}L -{removed}L",
        summary_paths.join(", ")
    );
    if !confirm(ctx, prompt, diff_acc).await? {
        return Ok("(user denied this patch)".into());
    }

    // Commit: writes first, then deletes. If a write fails partway,
    // earlier files are already on disk — the patch isn't truly
    // atomic at the filesystem level. Worth flagging if we ever
    // care; for now the validation pass catches the vast majority
    // of failures before any write happens.
    for (path, content) in &new_files {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    anyhow!(
                        "apply_patch: failed to create parent dir of {}: {e}",
                        path.display()
                    )
                })?;
            }
        }
        tokio::fs::write(path, content.as_bytes())
            .await
            .map_err(|e| anyhow!("apply_patch: write failed for {}: {e}", path.display()))?;
    }
    for path in &deletes {
        tokio::fs::remove_file(path)
            .await
            .map_err(|e| anyhow!("apply_patch: delete failed for {}: {e}", path.display()))?;
    }

    Ok(format!(
        "applied patch: {} file{}",
        summary_paths.len(),
        if summary_paths.len() == 1 { "" } else { "s" }
    ))
}

/// Tag the diff with a one-line header so the confirm popup shows a
/// readable separator between files in a multi-file patch.
fn push_file_header(diff: &mut Vec<DiffLine>, header: &str) {
    if !diff.is_empty() {
        diff.push(DiffLine {
            kind: DiffLineKind::Context,
            text: String::new(),
        });
    }
    diff.push(DiffLine {
        kind: DiffLineKind::Summary,
        text: header.into(),
    });
}

/// Reduce a hunk to the `(old_text, new_text)` pair an edit_file-shape
/// matcher needs. Context lines appear in both; removes appear only
/// in old; adds appear only in new. Order is preserved exactly as
/// the hunk specified.
fn hunk_to_replace_pair(hunk: &Hunk) -> (String, String) {
    let mut old = String::new();
    let mut new = String::new();
    for (i, line) in hunk.lines.iter().enumerate() {
        let nl = if i > 0 { "\n" } else { "" };
        match line {
            HunkLine::Context(s) => {
                old.push_str(nl);
                old.push_str(s);
                new.push_str(nl);
                new.push_str(s);
            }
            HunkLine::Remove(s) => {
                old.push_str(nl);
                old.push_str(s);
            }
            HunkLine::Add(s) => {
                new.push_str(nl);
                new.push_str(s);
            }
        }
    }
    // The leading-nl logic above leaves both buffers without a
    // trailing newline. The matcher cascade compares against raw
    // file content so this is the right shape.
    (old, new)
}

/// Parse a V4A envelope into a list of file operations. Tolerant of
/// trailing whitespace on the directive lines and of stray blank
/// lines between hunks — strict on the directive prefixes themselves.
fn parse_patch(text: &str) -> Result<Vec<FileOp>> {
    let mut lines = text.lines().peekable();
    // Skip blank lines before the envelope opener; some models like
    // to lead with a blank line after the tool call.
    loop {
        match lines.peek() {
            Some(l) if l.trim().is_empty() => {
                lines.next();
            }
            Some(_) => break,
            None => bail!("apply_patch: empty patch"),
        }
    }
    let first = lines
        .next()
        .ok_or_else(|| anyhow!("apply_patch: empty patch"))?;
    if first.trim() != "*** Begin Patch" {
        bail!(
            "apply_patch: patch must start with '*** Begin Patch' (got '{}')",
            first
        );
    }

    let mut ops: Vec<FileOp> = Vec::new();

    while let Some(&line) = lines.peek() {
        let trimmed = line.trim_end();
        if trimmed == "*** End Patch" {
            lines.next();
            // Allow trailing blank lines after End Patch.
            while let Some(l) = lines.peek() {
                if l.trim().is_empty() {
                    lines.next();
                } else {
                    bail!(
                        "apply_patch: unexpected content after '*** End Patch': '{}'",
                        l
                    );
                }
            }
            return Ok(ops);
        }
        if let Some(path) = trimmed.strip_prefix("*** Add File: ") {
            lines.next();
            let mut content = String::new();
            while let Some(&l) = lines.peek() {
                if l.starts_with("*** ") {
                    break;
                }
                lines.next();
                // Add File content uses '+' prefix on every line.
                if let Some(body) = l.strip_prefix('+') {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(body);
                } else if l.trim().is_empty() {
                    // Tolerate stray blank lines inside an Add body.
                    if !content.is_empty() {
                        content.push('\n');
                    }
                } else {
                    bail!(
                        "apply_patch: Add File '{path}' body line must start with '+', got '{l}'"
                    );
                }
            }
            ops.push(FileOp::Add {
                path: path.trim().to_string(),
                content,
            });
            continue;
        }
        if let Some(path) = trimmed.strip_prefix("*** Delete File: ") {
            lines.next();
            ops.push(FileOp::Delete {
                path: path.trim().to_string(),
            });
            continue;
        }
        if let Some(path) = trimmed.strip_prefix("*** Update File: ") {
            lines.next();
            // Optional rename right after the Update directive.
            let mut move_to: Option<String> = None;
            if let Some(&l) = lines.peek() {
                if let Some(new_path) = l.trim_end().strip_prefix("*** Move to: ") {
                    move_to = Some(new_path.trim().to_string());
                    lines.next();
                }
            }
            let mut hunks: Vec<Hunk> = Vec::new();
            let mut current: Option<Hunk> = None;
            while let Some(&l) = lines.peek() {
                if l.starts_with("*** ") {
                    break;
                }
                lines.next();
                if let Some(anchor) = l.strip_prefix("@@") {
                    if let Some(h) = current.take() {
                        hunks.push(h);
                    }
                    let a = anchor.trim();
                    current = Some(Hunk {
                        anchor: if a.is_empty() {
                            None
                        } else {
                            Some(a.to_string())
                        },
                        lines: Vec::new(),
                    });
                    continue;
                }
                let cur = current.get_or_insert_with(Hunk::default);
                if let Some(body) = l.strip_prefix('+') {
                    cur.lines.push(HunkLine::Add(body.to_string()));
                } else if let Some(body) = l.strip_prefix('-') {
                    cur.lines.push(HunkLine::Remove(body.to_string()));
                } else if let Some(body) = l.strip_prefix(' ') {
                    cur.lines.push(HunkLine::Context(body.to_string()));
                } else if l.is_empty() {
                    // Empty line == empty context line. Some models
                    // omit the leading space for empty context.
                    cur.lines.push(HunkLine::Context(String::new()));
                } else {
                    bail!(
                        "apply_patch: Update File '{path}' hunk line must start with '@@', \
                         '+', '-', or ' ', got '{l}'"
                    );
                }
            }
            if let Some(h) = current {
                hunks.push(h);
            }
            ops.push(FileOp::Update {
                path: path.trim().to_string(),
                hunks,
                move_to,
            });
            continue;
        }
        if trimmed.is_empty() {
            lines.next();
            continue;
        }
        bail!(
            "apply_patch: unexpected line outside any file directive: '{}'",
            line
        );
    }
    bail!("apply_patch: patch missing '*** End Patch' terminator");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_add() {
        let p = "*** Begin Patch\n*** Add File: foo.txt\n+hello\n+world\n*** End Patch\n";
        let ops = parse_patch(p).unwrap();
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FileOp::Add { path, content } => {
                assert_eq!(path, "foo.txt");
                assert_eq!(content, "hello\nworld");
            }
            other => panic!("expected Add, got {other:?}"),
        }
    }

    #[test]
    fn parses_update_with_hunk() {
        let p = "\
*** Begin Patch
*** Update File: README.md
@@ ## Heading
-old line
+new line
 context line
*** End Patch
";
        let ops = parse_patch(p).unwrap();
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            FileOp::Update { path, hunks, .. } => {
                assert_eq!(path, "README.md");
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].anchor.as_deref(), Some("## Heading"));
                assert_eq!(hunks[0].lines.len(), 3);
            }
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[test]
    fn parses_delete() {
        let p = "*** Begin Patch\n*** Delete File: junk.txt\n*** End Patch\n";
        let ops = parse_patch(p).unwrap();
        assert_eq!(ops.len(), 1);
        assert!(matches!(&ops[0], FileOp::Delete { path } if path == "junk.txt"));
    }

    #[test]
    fn rejects_missing_envelope() {
        let p = "*** Update File: foo\n@@\n+bar\n";
        assert!(parse_patch(p).is_err());
    }

    #[test]
    fn hunk_to_pair_swaps_correctly() {
        let h = Hunk {
            anchor: None,
            lines: vec![
                HunkLine::Context("line1".into()),
                HunkLine::Remove("oldline".into()),
                HunkLine::Add("newline".into()),
                HunkLine::Context("line3".into()),
            ],
        };
        let (old, new) = hunk_to_replace_pair(&h);
        assert_eq!(old, "line1\noldline\nline3");
        assert_eq!(new, "line1\nnewline\nline3");
    }
}
