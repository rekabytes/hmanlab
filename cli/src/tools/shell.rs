//! `run_command` — shell escape hatch. User-approval required per call.
//!
//! The command is spawned with piped stdout/stderr, and each line is
//! emitted as a `StreamMsg::ShellOutput` so the UI's `ShellMonitor`
//! overlay can render the output live. A oneshot `kill_tx` is handed to
//! the App on `ShellStart`; Ctrl+C in the monitor fires it, which races
//! against `child.wait()` inside the local `tokio::select!` and triggers
//! `child.kill()`. The final assembled string still flows back to the
//! agent as the tool result (same shape as the old bundled output).

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command as TokioCmd;
use tokio::sync::oneshot;

use crate::app::StreamMsg;

use super::workspace::{truncate_utf8, MAX_CMD_BYTES};
use super::{confirm, ToolContext};

/// Hard ceiling on a single shell invocation. Up from the previous 30 s
/// because the monitor overlay lets a user actually *watch* long-running
/// commands now (test suites, builds) — 10 min keeps the runaway-shell
/// safety net while making real workflows usable. Users can kill earlier
/// from the monitor with Ctrl+C.
const CMD_TIMEOUT: Duration = Duration::from_secs(600);

pub(super) async fn tool_run_command(args: &Value, ctx: &ToolContext) -> Result<String> {
    let cmd = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("run_command requires 'command'"))?
        .to_string();

    let prompt = format!(
        "Run shell command in {}:\n  {}",
        ctx.workspace.display(),
        cmd
    );
    if !confirm(ctx, prompt, Vec::new()).await? {
        return Ok("(user denied this command)".into());
    }

    // Pipe stdout/stderr so we can stream them line-by-line. `kill_on_drop`
    // is the belt-and-suspenders fallback: if this task itself is aborted
    // (Esc / Ctrl+C cancelling the whole agent turn), the dropped Child
    // SIGKILLs the process so we don't leak zombies. The explicit kill
    // path below is the primary mechanism for monitor-driven interrupts.
    let mut child = TokioCmd::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(&ctx.workspace)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow!("failed to spawn shell: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("child has no stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("child has no stderr pipe"))?;

    // Kill channel — handed to App via ShellStart, fired by the
    // monitor's Ctrl+C handler. Wraps in Option so it can be consumed
    // (oneshot Senders are single-use).
    let (kill_tx, mut kill_rx) = oneshot::channel::<()>();
    ctx.stream_tx
        .send(StreamMsg::ShellStart {
            command: cmd.clone(),
            kill_tx,
        })
        .map_err(|_| anyhow!("UI channel closed before shell start"))?;

    // Each reader task pushes lines into the shared collector AND emits
    // a stream event. The collector is what we hand back to the agent
    // at the end; the stream events drive the live monitor. Splitting
    // these means the agent's tool result is independent of whether the
    // UI ever saw the live stream (which matters if the App is busy or
    // the monitor wasn't opened).
    let stdout_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let stdout_collect = stdout_lines.clone();
    let stdout_tx = ctx.stream_tx.clone();
    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            stdout_collect.lock().unwrap().push(line.clone());
            let _ = stdout_tx.send(StreamMsg::ShellOutput {
                line,
                is_stderr: false,
            });
        }
    });

    let stderr_collect = stderr_lines.clone();
    let stderr_tx = ctx.stream_tx.clone();
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            stderr_collect.lock().unwrap().push(line.clone());
            let _ = stderr_tx.send(StreamMsg::ShellOutput {
                line,
                is_stderr: true,
            });
        }
    });

    // Three-way race: child exit, user kill, hard timeout. Whichever
    // wins, the other branches are dropped — but the reader tasks keep
    // running until the pipes close, which happens automatically once
    // the process dies (either naturally or via SIGKILL).
    let mut killed_by_user = false;
    let mut timed_out = false;
    let exit_status = tokio::select! {
        result = child.wait() => match result {
            Ok(s) => s,
            Err(e) => bail!("failed to wait for shell: {e}"),
        },
        _ = &mut kill_rx => {
            killed_by_user = true;
            let _ = child.kill().await;
            child.wait().await.map_err(|e| anyhow!("wait after kill failed: {e}"))?
        }
        _ = tokio::time::sleep(CMD_TIMEOUT) => {
            timed_out = true;
            let _ = child.kill().await;
            child.wait().await.map_err(|e| anyhow!("wait after timeout failed: {e}"))?
        }
    };

    // Pipes close when the child dies; the reader tasks then exit on
    // their next `next_line().await` returning None. Joining ensures we
    // don't miss the very last line if the child wrote+exited fast.
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    let stdout = stdout_lines.lock().unwrap();
    let stderr = stderr_lines.lock().unwrap();

    let mut text = String::new();
    if !stdout.is_empty() {
        text.push_str(&stdout.join("\n"));
    }
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str("[stderr]\n");
        text.push_str(&stderr.join("\n"));
    }

    let footer = if killed_by_user {
        "\n[killed by user]".to_string()
    } else if timed_out {
        format!("\n[timed out after {} seconds]", CMD_TIMEOUT.as_secs())
    } else {
        format!(
            "\n[exit {}]",
            exit_status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "?".into())
        )
    };
    text.push_str(&footer);

    let _ = ctx.stream_tx.send(StreamMsg::ShellDone {
        exit_code: exit_status.code(),
    });

    Ok(truncate_utf8(text, MAX_CMD_BYTES))
}
