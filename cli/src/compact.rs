//! Conversation compaction — squeeze old turns into a single summary so the
//! context window keeps room for new work. Used both by the `/compact` slash
//! command and the auto-trigger in `send_to_llm` when the last prompt's
//! token count crosses [`AUTO_COMPACT_THRESHOLD`].
//!
//! The compaction prompt asks the current model for a terse bullet list
//! covering decisions, file paths touched, code changes, errors observed,
//! open questions, and user preferences — the load-bearing context a fresh
//! turn would need to keep being useful.

use anyhow::Result;
use futures::StreamExt;

use crate::app::LlmBackend;
use crate::ollama::{ChatMessage, StreamItem};

/// If the most recent turn's prompt size exceeds this many tokens, the next
/// user message triggers an auto-compaction before being sent. Picked
/// conservatively for a 32k-context budget (≈ 75% headroom for the summary
/// call + next turn); tune later if needed.
pub const AUTO_COMPACT_THRESHOLD: u32 = 24_000;

const SYSTEM_PROMPT: &str = "You are compressing a chat history into a context briefing for \
yourself in the next turn. Output a tight markdown bullet list (8–15 items) covering ONLY \
load-bearing context:\n\
- key decisions made\n\
- files touched (with paths) and what changed\n\
- errors observed and how they were resolved\n\
- open questions or pending work\n\
- user preferences or constraints expressed\n\n\
Skip greetings, repetitions, off-topic chatter, and play-by-play of tool calls. No prose \
preamble, no closing remark, no headers — just the bullets.";

/// Render the visible conversation into a single transcript string for the
/// model to consume. Tool calls are rendered inline as `[tool ran: name → \
/// N lines]` so the model knows actions happened without re-ingesting the
/// full tool output.
fn render_transcript(messages: &[ChatMessage]) -> String {
    let mut out = String::new();
    for m in messages {
        if m.hidden {
            continue;
        }
        match m.role.as_str() {
            "user" => {
                out.push_str("USER: ");
                out.push_str(m.content.trim());
                out.push_str("\n\n");
            }
            "assistant" => {
                // Strip <think>...</think> reasoning blocks from the
                // transcript — they're chain-of-thought, not durable context.
                let content = strip_thinking(&m.content);
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    out.push_str("ASSISTANT: ");
                    out.push_str(trimmed);
                    out.push_str("\n\n");
                }
                if let Some(tcs) = &m.tool_calls {
                    for tc in tcs {
                        out.push_str(&format!("  → called {}\n", tc.function.name));
                    }
                    if !tcs.is_empty() {
                        out.push('\n');
                    }
                }
            }
            "tool" => {
                let name = m.name.as_deref().unwrap_or("tool");
                let line_count = m.content.lines().count();
                out.push_str(&format!(
                    "  [tool {name} → {line_count} line{}]\n\n",
                    if line_count == 1 { "" } else { "s" }
                ));
            }
            "summary" => {
                // Prior summary from an earlier compaction — fold it in so
                // we don't lose history older than the most-recent compact.
                out.push_str("PRIOR-SUMMARY:\n");
                out.push_str(m.content.trim());
                out.push_str("\n\n");
            }
            _ => {} // info, system, anything else — skipped
        }
    }
    out
}

/// Strip every `<think>…</think>` block from an assistant message body.
/// Closed and unclosed (mid-stream) blocks are both removed.
fn strip_thinking(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        rest = &rest[start + "<think>".len()..];
        if let Some(end) = rest.find("</think>") {
            rest = &rest[end + "</think>".len()..];
        } else {
            // Unclosed — assume the whole rest is thinking; drop it.
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

/// Compact a chat history into a summary by calling the current backend
/// with the summarization prompt. Returns the summary text plus token
/// usage for accounting. The backend is called WITHOUT tools — this is a
/// pure text-generation call.
pub async fn compact_history(
    backend: &LlmBackend,
    model: &str,
    messages: Vec<ChatMessage>,
) -> Result<(String, u32, u32)> {
    let transcript = render_transcript(&messages);
    let request_messages = vec![
        ChatMessage {
            role: "system".into(),
            content: SYSTEM_PROMPT.to_string(),
            ..Default::default()
        },
        ChatMessage {
            role: "user".into(),
            content: format!("Compress this conversation:\n\n{transcript}"),
            ..Default::default()
        },
    ];

    let mut stream = match backend {
        LlmBackend::Ollama(c) => c.stream_chat(model, request_messages, None).await?,
        LlmBackend::OpenAi(c) => c.stream_chat(model, request_messages, None).await?,
    };

    let mut summary = String::new();
    let mut prompt_tokens = 0u32;
    let mut completion_tokens = 0u32;
    while let Some(item) = stream.next().await {
        match item? {
            StreamItem::Content(chunk) => summary.push_str(&chunk),
            StreamItem::Done {
                prompt_tokens: pt,
                completion_tokens: ct,
            } => {
                prompt_tokens = pt;
                completion_tokens = ct;
                break;
            }
            // We didn't pass any tool definitions; if the model emits a
            // tool_call anyway, ignore it.
            StreamItem::ToolCalls(_) => {}
        }
    }

    let summary = strip_thinking(&summary).trim().to_string();
    Ok((summary, prompt_tokens, completion_tokens))
}
