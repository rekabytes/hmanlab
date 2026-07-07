//! Tool-surface adapters for the memory store. Maps `save_memory`,
//! `read_memory`, `forget_memory` JSON calls into the operations exposed by
//! `crate::memory`. Save and forget go through the same confirmation flow as
//! `edit_file` / `write_file` so writes never happen behind the user's back.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;

use crate::memory::{self, build_memory_file, MemoryScope};

use super::diff::{diff_write, DiffLine, DiffLineKind};
use super::{confirm, ToolContext};

/// Soft cap on memory bodies. Memories should be terse — if the model is
/// trying to stash 100 KB it's misusing the system, and the index line for it
/// will be wrong anyway.
const MAX_MEMORY_BYTES: usize = 16_000;

pub(super) async fn tool_save_memory(args: &Value, ctx: &ToolContext) -> Result<String> {
    let scope = parse_scope(args)?;
    let name = require_str(args, "name")?;
    let kind = require_str(args, "type")?;
    let description = require_str(args, "description")?;
    let body = require_str(args, "body")?;

    if body.len() > MAX_MEMORY_BYTES {
        bail!(
            "save_memory: body is {} bytes; cap is {}. Memories should be terse — split or trim.",
            body.len(),
            MAX_MEMORY_BYTES
        );
    }
    if description.len() > 240 {
        bail!(
            "save_memory: description is {} chars; cap is 240. The description sits in the always-loaded index — keep it tight.",
            description.len()
        );
    }
    match kind {
        "user" | "project" | "feedback" | "reference" => {}
        other => {
            bail!("save_memory: unknown type '{other}' — expected user|project|feedback|reference")
        }
    }

    let slug = memory::sanitize_name(name);
    if slug.is_empty() {
        bail!("save_memory: 'name' must contain at least one alphanumeric character");
    }

    // Show a diff against the prior memory (if any) so the user can review
    // overwrites the same way `write_file` does. New memories show as all-added.
    let dir = memory::scope_dir(scope, &ctx.workspace)
        .ok_or_else(|| anyhow!("$HOME not set; user-scope memories unavailable"))?;
    let target_path = dir.join(format!("{slug}.md"));
    let prev = std::fs::read_to_string(&target_path).ok();
    let new_content = build_memory_file(&slug, kind, description, body);

    let action = if prev.is_some() { "UPDATE" } else { "SAVE" };
    let prompt = format!(
        "{action} memory `{slug}` ({}, scope={}) — {} bytes",
        kind,
        scope.as_str(),
        new_content.len()
    );
    let diff = diff_write(prev.as_deref(), &new_content);
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this memory save)".into());
    }

    let path = memory::save_memory(scope, &slug, kind, description, body, &ctx.workspace)?;
    Ok(format!(
        "saved memory `{slug}` to {} (scope={})",
        path.display(),
        scope.as_str()
    ))
}

pub(super) async fn tool_read_memory(args: &Value, ctx: &ToolContext) -> Result<String> {
    let scope = parse_scope(args)?;
    let name = require_str(args, "name")?;
    let mem = memory::read_memory(scope, name, &ctx.workspace)?;
    // Return the rendered file the same shape it lives on disk — including
    // frontmatter — so the model sees the full structure when reasoning about
    // updates. Cheap and unambiguous.
    Ok(build_memory_file(
        &mem.name,
        &mem.kind,
        &mem.description,
        &mem.body,
    ))
}

pub(super) async fn tool_forget_memory(args: &Value, ctx: &ToolContext) -> Result<String> {
    let scope = parse_scope(args)?;
    let name = require_str(args, "name")?;
    let slug = memory::sanitize_name(name);
    let dir = memory::scope_dir(scope, &ctx.workspace)
        .ok_or_else(|| anyhow!("$HOME not set; user-scope memories unavailable"))?;
    let path = dir.join(format!("{slug}.md"));
    if !path.exists() {
        bail!(
            "forget_memory: '{slug}' not found in {} scope",
            scope.as_str()
        );
    }

    // Build a diff that shows the whole memory as removed lines, so the
    // confirmation popup can read "you're about to delete X".
    let prev = std::fs::read_to_string(&path).unwrap_or_default();
    let diff: Vec<DiffLine> = prev
        .lines()
        .map(|l| DiffLine {
            kind: DiffLineKind::Removed,
            text: format!("- {l}"),
        })
        .collect();
    let prompt = format!(
        "FORGET memory `{slug}` (scope={}) — {} bytes",
        scope.as_str(),
        prev.len()
    );
    if !confirm(ctx, prompt, diff).await? {
        return Ok("(user denied this memory deletion)".into());
    }

    memory::forget_memory(scope, &slug, &ctx.workspace)?;
    Ok(format!("forgot memory `{slug}` (scope={})", scope.as_str()))
}

fn parse_scope(args: &Value) -> Result<MemoryScope> {
    let scope = args
        .get("scope")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing 'scope' (expected 'user' or 'project')"))?;
    MemoryScope::from_str(scope)
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing or non-string argument '{key}'"))
}
