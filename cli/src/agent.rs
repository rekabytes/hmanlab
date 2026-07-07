//! The async task that drives one chat round: stream tokens from the LLM,
//! dispatch any tool calls the model emits, persist results, and loop until
//! the model produces a final text answer (or exhausts MAX_TURNS).
//!
//! Not part of `App` state — `App` only holds the things the UI needs to
//! render and the channels to talk to this task. The agent runs spawned
//! from `app.rs` and communicates back via `StreamMsg`.

use futures::StreamExt;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot};

use crate::app::{LlmBackend, StreamMsg};
use crate::ollama::{ChatMessage, StreamItem, ToolCall};
use crate::tools;

/// Hard cap on agent-loop iterations. If a model keeps calling tools without
/// ever emitting a final reply, we kill the run rather than burning tokens
/// forever — this is a panic-button for stuck-in-loop models, NOT a normal
/// exit condition and NOT related to chat history paging.
///
/// 50 is comfortable headroom for real coding tasks (15–30 tool calls is
/// routine for multi-file edits). Hitting it almost always means the model
/// is actually stuck — interrupt with Ctrl+C and rephrase, or extend if a
/// specific task legitimately needs more iterations.
const MAX_TURNS: usize = 50;

/// Phase-2 dependency bundle: one specialist's ready-to-run handle.
/// The backend is pre-built so the tool handler doesn't need to touch
/// `App` (which lives on the UI thread and can't be moved into the
/// agent task). Lives here rather than in `tools/` because it's owned
/// by the agent loop — `ToolContext` only borrows a clone for the
/// duration of one call.
#[derive(Clone)]
pub struct SpecialistRunner {
    pub name: String,
    pub model: String,
    /// One-line description ("use this when …"). Surfaced verbatim in
    /// the `consult_specialist` tool description so the main model can
    /// route by intent rather than guessing from the name alone.
    pub task: String,
    pub system_prompt: String,
    pub backend: LlmBackend,
}

/// Sole entry point for chat turns. The `tool_defs` slice decides which
/// tools the model sees this run — main agent gets the full set (via
/// `tools::tool_definitions_with`, which conditionally adds the phase-2
/// `consult_specialist` tool when `specialists` is non-empty); the
/// `/ask` and nested-specialist paths get the read-only subset (see
/// `app::commands::agents::ask_tool_defs`). If `history`'s first message
/// is `role == "system"` we treat it as a caller-supplied override and
/// skip prepending the default workspace system prompt; otherwise we
/// prepend the standard one as before.
pub async fn agent_loop_with(
    backend: LlmBackend,
    model: String,
    history: Vec<ChatMessage>,
    workspace: PathBuf,
    tx: mpsc::UnboundedSender<StreamMsg>,
    tool_defs: Vec<crate::ollama::Tool>,
    specialists: Vec<SpecialistRunner>,
    mcp_active_provider: Option<String>,
    mcp_keys: std::collections::HashMap<String, String>,
) {
    // ToolContext gets a clone of the same stream channel the rest of
    // the agent loop sends through. Confirm requests and specialist
    // dispatches both ride on that single channel as plain `StreamMsg`
    // variants — no per-purpose bridge task to spawn or tear down.
    let ctx = tools::ToolContext {
        workspace: workspace.clone(),
        stream_tx: tx.clone(),
        specialists,
        mcp_active_provider,
        mcp_keys,
    };

    let mut full_history: Vec<ChatMessage> =
        if history.first().map(|m| m.role == "system").unwrap_or(false) {
            // Caller already supplied a system prompt (e.g. a specialist
            // persona via `/ask`). Use it verbatim instead of stacking the
            // default workspace prompt on top.
            history
        } else {
            let mut full = vec![ChatMessage {
                role: "system".into(),
                content: tools::system_prompt(&workspace),
                ..Default::default()
            }];
            full.extend(history);
            full
        };

    for _ in 0..MAX_TURNS {
        let stream_res = match &backend {
            LlmBackend::Ollama(c) => {
                c.stream_chat(&model, full_history.clone(), Some(tool_defs.clone()))
                    .await
            }
            LlmBackend::OpenAi(c) => {
                c.stream_chat(&model, full_history.clone(), Some(tool_defs.clone()))
                    .await
            }
        };
        let mut stream = match stream_res {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(StreamMsg::Error(e.to_string()));
                return;
            }
        };

        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut turn_prompt_tokens: u32 = 0;
        let mut turn_completion_tokens: u32 = 0;
        while let Some(item) = stream.next().await {
            match item {
                Ok(StreamItem::Content(s)) => {
                    let _ = tx.send(StreamMsg::Chunk(s.clone()));
                    content.push_str(&s);
                }
                Ok(StreamItem::ToolCalls(calls)) => {
                    tool_calls.extend(calls);
                }
                Ok(StreamItem::Done {
                    prompt_tokens,
                    completion_tokens,
                }) => {
                    turn_prompt_tokens = prompt_tokens;
                    turn_completion_tokens = completion_tokens;
                }
                Err(e) => {
                    let _ = tx.send(StreamMsg::Error(e.to_string()));
                    return;
                }
            }
        }

        if tool_calls.is_empty() {
            let _ = tx.send(StreamMsg::Done {
                prompt_tokens: turn_prompt_tokens,
                completion_tokens: turn_completion_tokens,
            });
            return;
        }

        let _ = tx.send(StreamMsg::AssistantTurnEnded {
            tool_calls: tool_calls.clone(),
        });
        full_history.push(ChatMessage::assistant_with_tool_calls(
            content.clone(),
            tool_calls.clone(),
        ));

        for (i, tc) in tool_calls.iter().enumerate() {
            let _ = tx.send(StreamMsg::ToolStart {
                name: tc.function.name.clone(),
                args: tc.function.arguments.clone(),
            });
            let output =
                match tools::execute_tool(&tc.function.name, &tc.function.arguments, &ctx).await {
                    Ok(s) => s,
                    Err(e) => format!("error: {e}"),
                };
            let _ = tx.send(StreamMsg::ToolResult {
                output: output.clone(),
            });
            // Positional call id (`call_<index>`) matches the synthesized
            // id the OpenAI-compat serializer attaches to the assistant
            // turn's tool_calls. Strict providers (MiniMax) 400 without
            // this; Ollama is lenient.
            full_history.push(ChatMessage::tool_result(
                tc.function.name.clone(),
                output,
                format!("call_{i}"),
            ));
        }

        let _ = tx.send(StreamMsg::NewAssistantTurn);
    }

    let _ = tx.send(StreamMsg::Error(format!(
        "agent stopped after {MAX_TURNS} tool-call rounds without a final text reply — \
         the model is likely stuck in a loop. Ask it to summarise what it has so far, \
         or rephrase the request."
    )));
}

/// Phase 3: top-level consult-specialist task.
///
/// The main agent's `consult_specialist` tool sends a
/// `StreamMsg::SpecialistRequest` carrying a `oneshot::Sender` and
/// awaits the receiver. App's stream handler spawns *this* task on a
/// fresh Tokio worker — no recursive future, no `Box::pin`, no
/// `tokio::join!` workaround.
///
/// The drain loop races `sub_rx.recv()` against `reply_tx.closed()`.
/// If the caller's oneshot receiver gets dropped (parent task aborted
/// by Ctrl+C, for example), we abort the inner agent_loop_with task
/// immediately rather than letting it run to MAX_TURNS. Chain
/// cancellation falls out for free — the user pressing one Ctrl+C
/// stops everything in flight.
///
/// Streaming chunks are intentionally NOT forwarded to the main UI:
/// `consult_specialist` presents to the user as a single tool block
/// whose output is the consolidated reply. Live token-by-token
/// streaming under `[name]` is reserved for the explicit `/ask` path
/// (which still uses `agent_loop_with` directly).
pub async fn run_specialist_consult(
    runner: SpecialistRunner,
    query: String,
    workspace: PathBuf,
    parent_tx: mpsc::UnboundedSender<StreamMsg>,
    mut reply_tx: oneshot::Sender<Result<String, String>>,
) {
    let history = vec![
        ChatMessage {
            role: "system".into(),
            content: runner.system_prompt.clone(),
            ..Default::default()
        },
        ChatMessage {
            role: "user".into(),
            content: query,
            ..Default::default()
        },
    ];

    let (sub_tx, mut sub_rx) = mpsc::unbounded_channel::<StreamMsg>();
    let agent_task = tokio::spawn(agent_loop_with(
        runner.backend.clone(),
        runner.model.clone(),
        history,
        workspace,
        sub_tx,
        crate::tools::ask_specialist_tool_defs(),
        Vec::new(), // no nested specialists — chain prevention
        None,       // specialists don't get MCP web-search tools
        std::collections::HashMap::new(),
    ));

    let mut reply = String::new();
    let mut prompt_tokens: u32 = 0;
    let mut completion_tokens: u32 = 0;
    let mut error: Option<String> = None;

    loop {
        tokio::select! {
            biased;
            // Caller dropped the receiver (e.g. parent agent cancelled).
            // Stop the sub-agent and return without sending anything —
            // there's nobody listening anyway.
            _ = reply_tx.closed() => {
                agent_task.abort();
                return;
            }
            msg = sub_rx.recv() => {
                match msg {
                    Some(StreamMsg::Chunk(t)) => reply.push_str(&t),
                    Some(StreamMsg::Done {
                        prompt_tokens: p,
                        completion_tokens: c,
                    }) => {
                        prompt_tokens = p;
                        completion_tokens = c;
                    }
                    Some(StreamMsg::Error(e)) => {
                        error = Some(e);
                    }
                    // Read-only tool surface means ConfirmRequest can't
                    // happen; AssistantTurnEnded / NewAssistantTurn /
                    // ToolStart / ToolResult are internal bookkeeping the
                    // main UI doesn't need to see for a consult.
                    Some(_) => {}
                    None => break, // sub-agent dropped its tx → done
                }
            }
        }
    }
    // Make sure the spawned agent task is fully reaped before we touch
    // reply_tx — keeps the resource ordering tidy even though abort
    // would never fire from this path.
    let _ = agent_task.await;

    // Per-specialist token attribution. Fire-and-forget; the parent
    // stream might be torn down concurrently and that's fine.
    if prompt_tokens > 0 || completion_tokens > 0 {
        let _ = parent_tx.send(StreamMsg::SpecialistTokens {
            name: runner.name.clone(),
            prompt_tokens,
            completion_tokens,
        });
    }

    let final_result = if let Some(e) = error {
        Err(format!("specialist '{}' errored: {e}", runner.name))
    } else {
        let stripped = strip_think(&reply).trim().to_string();
        if stripped.is_empty() {
            Ok(format!("[specialist:{}] (returned no text)", runner.name))
        } else {
            Ok(format!("[specialist:{}]\n{stripped}", runner.name))
        }
    };
    // If the parent already dropped the receiver, send() returns Err
    // — that's the cancellation path we'd already have caught above,
    // but the race is benign so ignore.
    let _ = reply_tx.send(final_result);
}

/// Drop the `<think>…</think>` reasoning block. Same shape as the
/// chat-side helper; kept here so `agent.rs` can post-process a
/// specialist reply without reaching back into `app::stream`.
fn strip_think(raw: &str) -> String {
    match raw.find("</think>") {
        Some(idx) => raw[idx + "</think>".len()..]
            .trim_start_matches(['\n', '\r'])
            .to_string(),
        None => raw.to_string(),
    }
}
