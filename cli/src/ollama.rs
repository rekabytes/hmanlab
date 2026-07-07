use anyhow::{anyhow, Result};
use base64::Engine;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

/// An attachment (image or other media) accompanying a chat message.
/// Stored as raw bytes; encoded as base64 data URL when serializing to
/// the API format.
#[derive(Clone, Debug)]
pub struct Attachment {
    /// MIME type, e.g. "image/png", "image/jpeg"
    pub media_type: String,
    /// Raw bytes of the file (not base64-encoded)
    pub data: Vec<u8>,
    /// Original filename for UI display
    pub filename: String,
}

impl Attachment {
    /// Read a file and create an Attachment, auto-detecting media type.
    pub fn from_path(path: &std::path::Path) -> Result<Self> {
        let data =
            std::fs::read(path).map_err(|e| anyhow!("Failed to read {}: {}", path.display(), e))?;

        let media_type = mime_guess::from_path(path)
            .first()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(Self {
            media_type,
            data,
            filename,
        })
    }

    /// Encode as data URL for API payload: "data:image/png;base64,..."
    pub fn to_data_url(&self) -> String {
        let b64 = base64::engine::general_purpose::STANDARD.encode(&self.data);
        format!("data:{};base64,{}", self.media_type, b64)
    }

    /// Raw base64 (no `data:` prefix). Ollama's native `/api/chat`
    /// wants `images: ["<base64>"]` on the message, not OpenAI-style
    /// `image_url` content parts — without this, vision models like
    /// llava / qwen2-vl never see the image.
    pub fn to_base64(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(&self.data)
    }

    /// Human-readable size (e.g. "1.2 MB")
    pub fn size_display(&self) -> String {
        let bytes = self.data.len() as f64;
        if bytes < 1024.0 {
            format!("{} B", bytes as u64)
        } else if bytes < 1024.0 * 1024.0 {
            format!("{:.1} KB", bytes / 1024.0)
        } else {
            format!("{:.1} MB", bytes / (1024.0 * 1024.0))
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Function name for tool-result messages (role="tool").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// OpenAI-compat tool-result correlation: matches the `id` field of
    /// the originating `tool_call` from the prior assistant turn. Set
    /// by `agent_loop_with` when pushing tool result messages. Optional
    /// because the Ollama native protocol doesn't require it; some
    /// OpenAI-compat backends do (MiniMax 400s without it, glm-5.1 is
    /// lenient). Skipped from serialization when None so Ollama doesn't
    /// see a stray field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Locally-injected user messages (Y/N quick-reply) that go to the model
    /// but never render in the chat UI. Never sent over the wire.
    #[serde(skip)]
    pub hidden: bool,
    /// Snapshot of the coloured diff the user authorised for this tool call
    /// (write_file / edit_file / save_memory). Populated when `pending_tool_diff`
    /// is taken in `ToolResult`; consumed by the chat renderer to redraw the
    /// diff inline when the user clicks the tool row to expand it. Never sent
    /// over the wire — only relevant locally to the UI.
    #[serde(skip)]
    pub diff: Option<Vec<crate::tools::DiffLine>>,
    /// Original JSON arguments the model passed to the tool. Stamped in
    /// `on_tool_start` so the collapsed tile in the chat renderer can show
    /// a clean summary (e.g. `src/main.rs · 312 lines`) without re-parsing
    /// the placeholder content string. Never sent back to the model on a
    /// re-submit — the agent loop already keeps its own copy in the
    /// turn-bound `tool_calls` array.
    #[serde(skip)]
    pub tool_args: Option<Value>,
    /// Image/media attachments. Not serialized to disk (session persistence)
    /// — images should be re-read from disk, not stored in chat history JSON.
    #[serde(default, skip_serializing, skip_deserializing)]
    pub attachments: Vec<Attachment>,
}

impl ChatMessage {
    /// Build a tool-result message correctly correlated to its
    /// originating tool_call. `call_id` must match the synthesized id
    /// the OpenAI-compat serializer attaches to the assistant turn's
    /// `tool_calls` (positionally: `call_<index>`) — strict providers
    /// like MiniMax 400 without it. Use this constructor in the agent
    /// loop so the correlation never gets dropped by a future contributor
    /// reaching for `..Default::default()`.
    pub fn tool_result(name: String, content: String, call_id: String) -> Self {
        Self {
            role: "tool".into(),
            content,
            name: Some(name),
            tool_call_id: Some(call_id),
            ..Default::default()
        }
    }

    /// Build the assistant message that records this turn's emitted
    /// `tool_calls`. Counterpart to [`Self::tool_result`] — pairing the
    /// two builders keeps the request/response shape spec-correct
    /// without relying on hand-built struct literals.
    pub fn assistant_with_tool_calls(content: String, calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls: Some(calls),
            ..Default::default()
        }
    }

    /// Serialize to the API format (content-parts array if attachments present,
    /// or plain string otherwise). Returns a Value that can be used directly
    /// in JSON payloads.
    pub fn to_api_content(&self) -> Value {
        if self.attachments.is_empty() {
            Value::String(self.content.clone())
        } else {
            let mut parts = Vec::with_capacity(1 + self.attachments.len());

            // Text part first
            if !self.content.is_empty() {
                parts.push(serde_json::json!({
                    "type": "text",
                    "text": self.content,
                }));
            }

            // Image parts
            for att in &self.attachments {
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {
                        "url": att.to_data_url(),
                    }
                }));
            }

            Value::Array(parts)
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub function: ToolCallFunction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ToolFunctionSpec,
}

#[derive(Clone, Debug, Serialize)]
pub struct ToolFunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl Tool {
    pub fn function(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ApiMessage>,
    stream: bool,
    /// Disables the built-in chain-of-thought on thinking models (qwen3,
    /// deepseek-r1, etc.). No effect on non-thinking models.
    think: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Tool]>,
}

/// Message format sent to Ollama's `/api/chat`. Ollama is NOT
/// OpenAI-compatible for vision: it wants a plain-text `content`
/// alongside a separate `images: ["<base64>"]` array on the message
/// — content-parts arrays with `image_url` are silently dropped, so
/// the model never sees the attachment. The OpenAI-compat path in
/// `openai_compat.rs` uses a different shape (content parts) since
/// that's what z.ai / OpenRouter / opencode actually want.
#[derive(Serialize)]
struct ApiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
    content: String,
    /// Raw base64 image bytes (no `data:` prefix). Omitted when the
    /// message has no attachments so non-vision Ollama models don't see
    /// a stray field.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
}

impl From<&ChatMessage> for ApiMessage {
    fn from(msg: &ChatMessage) -> Self {
        Self {
            role: msg.role.clone(),
            name: msg.name.clone(),
            tool_call_id: msg.tool_call_id.clone(),
            tool_calls: msg.tool_calls.clone(),
            content: msg.content.clone(),
            images: msg.attachments.iter().map(|a| a.to_base64()).collect(),
        }
    }
}

#[derive(Deserialize)]
struct ChatChunk {
    message: Option<ChunkMessage>,
    done: bool,
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    #[serde(default)]
    eval_count: Option<u32>,
}

#[derive(Default, Deserialize)]
struct ChunkMessage {
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    name: String,
}

/// What a single streamed chunk yields back to the agent loop.
#[derive(Debug, Clone)]
pub enum StreamItem {
    Content(String),
    ToolCalls(Vec<ToolCall>),
    Done {
        prompt_tokens: u32,
        completion_tokens: u32,
    },
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    pub base: String,
    /// Bearer token for Ollama Cloud's hosted API. `None` for local daemons,
    /// which don't authenticate. Set via `with_api_key` so the local
    /// constructor path stays unchanged.
    api_key: Option<String>,
}

impl Client {
    pub fn new(base: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("reqwest client");
        let base = base.trim_end_matches('/').to_string();
        Self {
            http,
            base,
            api_key: None,
        }
    }

    /// Construct a client that authenticates every request with a Bearer
    /// token — the shape Ollama Cloud's public API expects.
    pub fn with_api_key(base: String, api_key: String) -> Self {
        let mut c = Self::new(base);
        c.api_key = Some(api_key);
        c
    }

    /// Attach the Bearer header when this client was built with an API key.
    /// No-op for keyless local daemons, which short-circuit auth entirely.
    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(k) => req.bearer_auth(k),
            None => req,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base);
        let resp: TagsResponse = self
            .auth(self.http.get(&url))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.models.into_iter().map(|m| m.name).collect())
    }

    pub async fn stream_chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<Tool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>> {
        let url = format!("{}/api/chat", self.base);
        let api_messages: Vec<ApiMessage> = messages.iter().map(ApiMessage::from).collect();
        let req = ChatRequest {
            model,
            messages: api_messages,
            stream: true,
            think: false,
            tools: tools.as_deref(),
        };
        let resp = self
            .auth(self.http.post(&url).json(&req))
            .send()
            .await?
            .error_for_status()?;
        let byte_stream = resp.bytes_stream();

        let stream = futures::stream::unfold(
            (byte_stream, Vec::<u8>::new(), false),
            |(mut bs, mut buf, mut done)| async move {
                if done {
                    return None;
                }
                loop {
                    if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = buf.drain(..=pos).collect();
                        let line = String::from_utf8_lossy(&line_bytes);
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<ChatChunk>(trimmed) {
                            Ok(chunk) => {
                                let msg = chunk.message.unwrap_or_default();
                                // Tool calls can arrive on done:false OR done:true — emit
                                // them whenever present, then let later iterations drain
                                // any remaining content/Done marker.
                                if !msg.tool_calls.is_empty() {
                                    if chunk.done {
                                        done = true;
                                    }
                                    return Some((
                                        Ok(StreamItem::ToolCalls(msg.tool_calls)),
                                        (bs, buf, done),
                                    ));
                                }
                                if !msg.content.is_empty() {
                                    if chunk.done {
                                        done = true;
                                    }
                                    return Some((
                                        Ok(StreamItem::Content(msg.content)),
                                        (bs, buf, done),
                                    ));
                                }
                                if chunk.done {
                                    done = true;
                                    return Some((
                                        Ok(StreamItem::Done {
                                            prompt_tokens: chunk.prompt_eval_count.unwrap_or(0),
                                            completion_tokens: chunk.eval_count.unwrap_or(0),
                                        }),
                                        (bs, buf, done),
                                    ));
                                }
                                // Empty intermediate chunk — keep looping
                                continue;
                            }
                            Err(e) => {
                                return Some((Err(anyhow!("parse error: {e}")), (bs, buf, true)));
                            }
                        }
                    }
                    match bs.next().await {
                        Some(Ok(bytes)) => buf.extend_from_slice(&bytes),
                        Some(Err(e)) => return Some((Err(anyhow!("http: {e}")), (bs, buf, true))),
                        None => {
                            let trimmed = String::from_utf8_lossy(&buf).trim().to_string();
                            if trimmed.is_empty() {
                                return None;
                            }
                            match serde_json::from_str::<ChatChunk>(&trimmed) {
                                Ok(chunk) => {
                                    let msg = chunk.message.unwrap_or_default();
                                    if !msg.tool_calls.is_empty() {
                                        return Some((
                                            Ok(StreamItem::ToolCalls(msg.tool_calls)),
                                            (bs, Vec::new(), true),
                                        ));
                                    }
                                    if !msg.content.is_empty() {
                                        return Some((
                                            Ok(StreamItem::Content(msg.content)),
                                            (bs, Vec::new(), true),
                                        ));
                                    }
                                    return Some((
                                        Ok(StreamItem::Done {
                                            prompt_tokens: chunk.prompt_eval_count.unwrap_or(0),
                                            completion_tokens: chunk.eval_count.unwrap_or(0),
                                        }),
                                        (bs, Vec::new(), true),
                                    ));
                                }
                                Err(e) => {
                                    return Some((
                                        Err(anyhow!("parse-eof: {e}")),
                                        (bs, Vec::new(), true),
                                    ));
                                }
                            }
                        }
                    }
                }
            },
        );
        Ok(Box::pin(stream))
    }
}
