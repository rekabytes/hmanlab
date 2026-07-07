//! Git introspection tools — read-only, no confirm step.
//!
//! `tool_git_diff` is the schema-driven entry point; `run_git` / `run_git_owned`
//! are the dispatch helpers used by `git_status`, `git_log`, `git_show` from
//! the top-level `execute_tool` match.

use anyhow::Result;
use serde_json::Value;
use tokio::process::Command as TokioCmd;

use super::workspace::{truncate_utf8, MAX_CMD_BYTES};
use super::ToolContext;

pub(super) async fn tool_git_diff(args: &Value, ctx: &ToolContext) -> Result<String> {
    let mut argv: Vec<String> = vec!["diff".into()];
    if let Some(range) = args.get("range").and_then(Value::as_str) {
        if !range.is_empty() {
            argv.push(range.to_string());
        }
    }
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        if !path.is_empty() {
            argv.push("--".into());
            argv.push(path.to_string());
        }
    }
    run_git_owned(ctx, argv).await
}

pub(super) async fn run_git(ctx: &ToolContext, args: &[&str]) -> Result<String> {
    let output = TokioCmd::new("git")
        .args(args)
        .current_dir(&ctx.workspace)
        .output()
        .await?;
    git_output_to_string(output)
}

pub(super) async fn run_git_owned(ctx: &ToolContext, args: Vec<String>) -> Result<String> {
    let output = TokioCmd::new("git")
        .args(&args)
        .current_dir(&ctx.workspace)
        .output()
        .await?;
    git_output_to_string(output)
}

fn git_output_to_string(output: std::process::Output) -> Result<String> {
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if !output.status.success() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("[stderr]\n");
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if text.trim().is_empty() {
        text = "(no output)".into();
    }
    Ok(truncate_utf8(text, MAX_CMD_BYTES))
}
