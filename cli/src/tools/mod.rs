//! Agent tool surface.
//!
//! The model talks to this module via two entry points:
//!   - `tool_definitions()` — JSON schemas Ollama serialises into its tool API
//!   - `execute_tool(name, args, ctx)` — dispatch into the right `tool_*` impl
//!
//! Each implementation lives in a subfile grouped by intent (read, git, write,
//! shell). Confirmation flow, diff preview, and workspace-path safety are
//! shared utilities under the same tree.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

use crate::app::StreamMsg;

mod definitions;
mod diff;
mod git;
mod lines;
mod matchers;
mod memory_tools;
mod patch;
mod read;
mod shell;
mod websearch;
mod workspace;
mod write;

pub use definitions::{system_prompt, tool_definitions, tool_definitions_with};
pub use diff::{DiffLine, DiffLineKind};

/// Request the user confirm before running something risky.
///
/// `prompt` is a short, plain-text headline (still used by run_command).
/// `diff`, when non-empty, is rendered as a coloured diff under the
/// headline — that's how edit_file / write_file show what they'd change.
pub struct ConfirmRequest {
    pub prompt: String,
    pub diff: Vec<DiffLine>,
    pub responder: oneshot::Sender<bool>,
}

/// Per-invocation context handed to each tool. Carries one mpsc
/// channel back to App — confirm requests and the phase-3 specialist
/// dispatch both ride on the same stream as plain `StreamMsg` variants
/// (no extra task to bridge a per-purpose channel).
pub struct ToolContext {
    pub workspace: PathBuf,
    /// Single channel back to the UI. `confirm` wraps requests as
    /// `StreamMsg::ConfirmRequest`; `tool_consult_specialist` wraps
    /// dispatches as `StreamMsg::SpecialistRequest`. Inside a nested
    /// specialist sub-loop this is the sub-loop's local channel, so
    /// any confirm requests (which can't happen anyway — read-only
    /// tools only) get silently dropped by the drain loop.
    pub stream_tx: mpsc::UnboundedSender<StreamMsg>,
    /// Pre-built specialist runners visible to this turn's tool surface
    /// (phase 2). Empty when `consult_specialist` shouldn't reach
    /// anyone — either agents are session-disabled, or this is a nested
    /// specialist call (no chaining, no recursion).
    pub specialists: Vec<crate::agent::SpecialistRunner>,
    /// Active MCP web-search provider id ("brave", "exa", "tavily",
    /// "parallel"). `None` if web-search is not configured — in that
    /// case `web_search` / `web_fetch` won't appear in the tool surface
    /// so the model can't call them, but we still guard here defensively.
    pub mcp_active_provider: Option<String>,
    /// API keys for MCP providers that require them (keyed by provider id).
    /// Parallel and Exa (basic tier) will never be present here.
    pub mcp_keys: std::collections::HashMap<String, String>,
}

/// Send a confirmation request to the UI and wait for the user's y/n.
/// Returns `Ok(false)` on both "deny" and "UI channel dropped".
pub(super) async fn confirm(
    ctx: &ToolContext,
    prompt: String,
    diff: Vec<DiffLine>,
) -> Result<bool> {
    let (tx, rx) = oneshot::channel::<bool>();
    ctx.stream_tx
        .send(StreamMsg::ConfirmRequest(ConfirmRequest {
            prompt,
            diff,
            responder: tx,
        }))
        .map_err(|_| anyhow!("UI channel closed before confirmation"))?;
    Ok(rx.await.unwrap_or(false))
}

/// Resolve a model-emitted tool name to a canonical hmanlab tool name.
///
/// Public v0.1 was fine-tuned on agentic traces that use Claude-Code-style
/// TitleCase names (`Read`, `Glob`, `Bash`, ...), not hmanlab's snake_case
/// surface. When such a model emits its trained name, route it to the matching
/// hmanlab handler instead of failing with "unknown tool". Snake_case names
/// pass through unchanged.
///
/// Exposed as `pub` so the chat renderer can canonicalise a tool name
/// before looking up its icon / label / grouping noun — otherwise a
/// `Read` call and a `read_file` call in the same turn would render as
/// two different verbs and miss the consolidation tile.
pub fn resolve_tool_alias(name: &str) -> &str {
    match name {
        "Read" => "read_file",
        "LS" | "List" => "list_dir",
        "Glob" => "find_files",
        "Bash" | "Shell" => "run_command",
        "Edit" => "edit_file",
        "MultiEdit" => "multi_edit",
        "Write" => "write_file",
        "ApplyPatch" => "apply_patch",
        "WebSearch" => "web_search",
        "WebFetch" => "web_fetch",
        other => other,
    }
}

/// True if `name` (model-emitted alias OK) is a tool that only reads
/// state — never mutates the filesystem, never runs a shell command,
/// never writes memory. Used by the chat renderer to consolidate runs
/// of consecutive read-only tool calls into a single "reading N files"
/// card so file-heavy turns don't pile up as N standalone rows.
pub fn is_readonly_tool(name: &str) -> bool {
    matches!(
        resolve_tool_alias(name),
        "read_file"
            | "list_dir"
            | "find_files"
            | "git_status"
            | "git_log"
            | "git_diff"
            | "git_show"
            | "read_memory"
            | "web_search"
            | "web_fetch"
    )
}

pub async fn execute_tool(name: &str, args: &Value, ctx: &ToolContext) -> Result<String> {
    let canonical = resolve_tool_alias(name);
    match canonical {
        "read_file" => read::tool_read_file(args, ctx).await,
        "list_dir" => read::tool_list_dir(args, ctx).await,
        "find_files" => read::tool_find_files(args, ctx).await,
        "git_status" => git::run_git(ctx, &["status", "--porcelain=v1", "-b"]).await,
        "git_log" => {
            let limit = args
                .get("limit")
                .and_then(Value::as_i64)
                .unwrap_or(10)
                .clamp(1, 100);
            git::run_git_owned(
                ctx,
                vec![
                    "log".into(),
                    "--oneline".into(),
                    "-n".into(),
                    limit.to_string(),
                ],
            )
            .await
        }
        "git_diff" => git::tool_git_diff(args, ctx).await,
        "git_show" => {
            let rev = args
                .get("rev")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("git_show requires 'rev'"))?;
            if !rev.chars().all(|c| {
                c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.' | '~' | '^')
            }) {
                bail!("rev contains disallowed characters");
            }
            // Drop `--stat`: the previous `--stat`-only output gave file
            // counts but no actual hunks, so "read the latest commit"
            // needed a second `git_diff HEAD~1..HEAD` call. Now one call
            // returns the full picture — message, author, date, AND the
            // line-level diff. Big commits still get tail-truncated by
            // `truncate_utf8`, in which case the model can fall back to
            // `git_diff` with a `path:` filter to drill into one file.
            git::run_git(ctx, &["show", rev]).await
        }
        "edit_file" => write::tool_edit_file(args, ctx).await,
        "multi_edit" => write::tool_multi_edit(args, ctx).await,
        "write_file" => write::tool_write_file(args, ctx).await,
        "apply_patch" => patch::tool_apply_patch(args, ctx).await,
        "move_lines" => lines::tool_move_lines(args, ctx).await,
        "delete_lines" => lines::tool_delete_lines(args, ctx).await,
        "insert_at" => lines::tool_insert_at(args, ctx).await,
        "run_command" => shell::tool_run_command(args, ctx).await,
        "save_memory" => memory_tools::tool_save_memory(args, ctx).await,
        "read_memory" => memory_tools::tool_read_memory(args, ctx).await,
        "forget_memory" => memory_tools::tool_forget_memory(args, ctx).await,
        "web_search" => websearch::tool_web_search(args, ctx).await,
        "web_fetch" => websearch::tool_web_fetch(args, ctx).await,
        "consult_specialist" => tool_consult_specialist(args, ctx).await,
        other => bail!("unknown tool: {other}"),
    }
}

/// `consult_specialist` — supervisor entry point. The main agent
/// picks a specialist by name; we look it up in `ctx.specialists`,
/// hand the runner off to App via `StreamMsg::SpecialistRequest`, and
/// await the consolidated reply through a `oneshot` channel. App runs
/// the sub-agent on its own Tokio task (see
/// `crate::agent::run_specialist_consult`) — no recursive future, no
/// `Box::pin`, no `tokio::join!`. Cancellation chains naturally: if
/// the parent agent task gets aborted, the oneshot receiver here
/// drops, App's spawn task sees `reply_tx.closed()` fire and aborts
/// the sub-agent.
async fn tool_consult_specialist(args: &Value, ctx: &ToolContext) -> Result<String> {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("consult_specialist requires 'name'"))?
        .trim()
        .to_string();
    let query = args
        .get("query")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("consult_specialist requires 'query'"))?
        .trim()
        .to_string();
    if name.is_empty() || query.is_empty() {
        bail!("consult_specialist needs both 'name' and 'query' (non-empty)");
    }

    let runner = ctx
        .specialists
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(&name))
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "no specialist named '{name}' is available. Known: {}",
                if ctx.specialists.is_empty() {
                    "(none — agents not enabled this session)".to_string()
                } else {
                    ctx.specialists
                        .iter()
                        .map(|s| s.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            )
        })?;

    let (reply_tx, reply_rx) = oneshot::channel::<Result<String, String>>();
    ctx.stream_tx
        .send(StreamMsg::SpecialistRequest {
            runner,
            query,
            reply_tx,
        })
        .map_err(|_| anyhow!("UI channel closed before specialist could be dispatched"))?;

    match reply_rx.await {
        Ok(Ok(reply)) => Ok(reply),
        Ok(Err(e)) => bail!("{e}"),
        Err(_) => bail!("specialist task ended without sending a reply"),
    }
}

/// Read-only tool surface for nested specialist calls — same subset
/// `/ask` uses (see `app::commands::agents::ask_tool_defs`). Kept
/// inline here so the tools module doesn't have to reach back into
/// `app::commands` for what is really a tool-module concern.
pub fn ask_specialist_tool_defs() -> Vec<crate::ollama::Tool> {
    const READONLY: &[&str] = &[
        "read_file",
        "list_dir",
        "find_files",
        "git_status",
        "git_log",
        "git_diff",
        "git_show",
        "read_memory",
    ];
    tool_definitions()
        .into_iter()
        .filter(|t| READONLY.iter().any(|name| t.function.name == *name))
        .collect()
}
