//! Local Telegram bot — long-polls `getUpdates` and forwards events to
//! the TUI via `StreamMsg`. Pattern C pairing: an unknown sender's first
//! DM produces a 6-char code that the user has to redeem in the terminal
//! with `/telegram pair <code>`. Allowlisted senders get their messages
//! surfaced as info lines.
//!
//! Deliberately minimal: no `teloxide` dependency, two endpoints
//! (`getUpdates`, `sendMessage`), one long-poll loop. The pending-codes
//! map is shared between the bot task and the `/telegram pair` handler
//! via `Arc<Mutex<...>>` so redemption is synchronous (no extra channels
//! needed).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::app::StreamMsg;

/// Pairing codes expire 10 minutes after the bot DMs them. Short enough
/// that shoulder-surfing isn't a window the attacker can come back to,
/// long enough that a user can hunt for the terminal without re-DMing.
pub const PAIR_CODE_TTL: Duration = Duration::from_secs(600);

/// Telegram caps `getUpdates` long-polls at 50 s on their end; we ask for
/// 30 so a slow shutdown doesn't leave the user waiting nearly a minute.
const LONG_POLL_SECS: u64 = 30;

/// Telegram's Bot API root. The token is appended per request.
const API_ROOT: &str = "https://api.telegram.org/bot";

/// In-memory pending-code map. Keyed by code, value is `(telegram_user_id,
/// minted_at)`. Lookup checks TTL — expired entries are treated as absent.
/// Held by the bot task (writer) and the `/telegram pair` handler (reader);
/// no need for a separate cleanup task.
pub type PendingCodes = Arc<Mutex<HashMap<String, (i64, Instant)>>>;

pub fn new_pending_map() -> PendingCodes {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Outbound work item handed to the bot task by the App side. The bot
/// owns the HTTP client + token so the App handlers don't have to —
/// they just describe what they want sent.
pub enum TelegramOut {
    /// Plain DM with no buttons. `parse_mode` is None for system
    /// messages we author ourselves (ack lines, status, command
    /// results — keep them as literal characters) and `Some("HTML")`
    /// for AI replies that we've pre-converted with
    /// `format_for_telegram`. Telegram rejects unbalanced or unescaped
    /// HTML with 400, so the only legal `Some` values today are
    /// `"HTML"` produced by our converter.
    Send {
        chat_id: i64,
        text: String,
        parse_mode: Option<&'static str>,
    },
    /// Confirm prompt with the OpenClaw-style 3-button inline keyboard.
    /// `callback_token` is the short opaque key the buttons carry in
    /// their `callback_data` — App stores the same token in its
    /// `pending_telegram_confirm` so when a callback fires we can find
    /// the in-flight oneshot. The bot task POSTs `sendMessage` and
    /// emits `StreamMsg::TelegramConfirmSent` once Telegram returns a
    /// `message_id` so the App can edit the message later.
    SendConfirm {
        chat_id: i64,
        text: String,
        callback_token: String,
    },
    /// Overwrite a previously-sent message in place. Used to turn the
    /// confirm prompt into "✓ Allowed by …" once the user (either side)
    /// answers. The keyboard is always cleared — the buttons shouldn't
    /// sit there looking actionable after we've resolved the prompt.
    EditMessage {
        chat_id: i64,
        message_id: i64,
        text: String,
    },
    // NOTE: `answerCallbackQuery` is fired directly from the bot task
    // (via `answer_callback` below) — it has a ~15 s server-side
    // deadline and we don't want to lose it to App-side processing
    // latency. Not surfaced as a TelegramOut variant on purpose.
}

/// Plain-text DM constructor. Use this for system messages we author
/// ourselves — literal characters, no markdown rendering. Special chars
/// like `<` show up exactly as typed.
pub fn dm(chat_id: i64, text: String) -> TelegramOut {
    TelegramOut::Send {
        chat_id,
        text,
        parse_mode: None,
    }
}

/// HTML DM constructor. Use this AFTER passing the AI's text through
/// `format_for_telegram` (or `markdown_to_telegram_html` for a single
/// chunk). `text` must be valid Telegram HTML — unbalanced tags get
/// rejected with 400 by the API.
pub fn dm_html(chat_id: i64, text: String) -> TelegramOut {
    TelegramOut::Send {
        chat_id,
        text,
        parse_mode: Some("HTML"),
    }
}

/// Control signal for the bot task. Today only `Shutdown`; future
/// variants might be "reload allowlist" or "rotate token".
pub enum TelegramCtl {
    Shutdown,
}

/// Snapshot of allowlist + token shared with the bot task. The bot reads
/// the allowlist on every incoming DM (under the mutex) so `/telegram
/// pair` can flip a sender from unknown → known without restarting the
/// task. Token is immutable for the task's lifetime — if it changes
/// (`/telegram setup` with a different token), the old task is told to
/// shut down and a new one is spawned.
pub struct BotState {
    pub token: String,
    pub allowlist: Arc<Mutex<Vec<i64>>>,
    pub pending: PendingCodes,
}

/// Telegram bot info, returned by `getMe`. We only consume `username` so
/// `/telegram setup` and `/telegram status` can show the bot's @handle.
#[derive(Debug, Deserialize)]
pub struct BotInfo {
    pub username: Option<String>,
}

/// Validate a token by calling `getMe`. Returns the bot's username on
/// success; the handler uses that to confirm setup ("Connected as @foo").
pub async fn get_me(token: &str) -> Result<BotInfo> {
    let url = format!("{API_ROOT}{token}/getMe");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("getMe failed ({})", resp.status()));
    }
    let body: GetMeEnvelope = resp.json().await?;
    if !body.ok {
        return Err(anyhow!("getMe returned ok=false"));
    }
    body.result
        .ok_or_else(|| anyhow!("getMe response missing result"))
}

#[derive(Debug, Deserialize)]
struct GetMeEnvelope {
    ok: bool,
    result: Option<BotInfo>,
}

/// One row in the curated `/`-menu we publish to Telegram via
/// `setMyCommands`. Telegram's UI renders this as a tap-to-insert
/// dropdown the moment the user types `/` in a chat with the bot. Keep
/// `description` ≤ 64 chars per the API docs (we cap manually below).
pub struct BotCommand {
    pub command: &'static str,
    pub description: &'static str,
}

/// Commands that map 1:1 to safe Telegram-side handlers in
/// `App::handle_telegram_command`. **Don't add anything here that
/// `handle_telegram_command` doesn't know about** — Telegram will show
/// the entry but the bot will reply "not available via Telegram", which
/// is a worse UX than not advertising it at all.
pub const PUBLISHED_COMMANDS: &[BotCommand] = &[
    BotCommand {
        command: "help",
        description: "list the commands Hibiscus accepts via Telegram",
    },
    BotCommand {
        command: "sessions",
        description: "show recent saved Hibiscus sessions",
    },
    BotCommand {
        command: "new",
        description: "start a fresh Hibiscus session",
    },
    BotCommand {
        command: "models",
        description: "list available models",
    },
    BotCommand {
        command: "model",
        description: "switch the active Ollama model (arg: name)",
    },
    BotCommand {
        command: "settings",
        description: "show account + version snapshot",
    },
];

/// Publish the curated `/`-menu via `setMyCommands`. Telegram caches
/// this server-side per-bot; calling it once after the bot starts is
/// enough — subsequent restarts don't re-push unless the list changed
/// (Telegram dedupes harmlessly anyway). Failure is non-fatal: the bot
/// works without a menu, the user just doesn't see the dropdown.
pub async fn set_my_commands(http: &reqwest::Client, token: &str) -> Result<()> {
    let url = format!("{API_ROOT}{token}/setMyCommands");
    let commands: Vec<serde_json::Value> = PUBLISHED_COMMANDS
        .iter()
        .map(|c| {
            json!({
                "command": c.command,
                // Defensive trim — Telegram rejects descriptions >256
                // chars; ours are short but trimming is cheap insurance.
                "description": c.description.chars().take(250).collect::<String>(),
            })
        })
        .collect();
    let resp = http
        .post(&url)
        .json(&json!({ "commands": commands }))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("setMyCommands failed ({})", resp.status()));
    }
    Ok(())
}

/// Send a DM via `sendMessage`. Returns the `message_id` Telegram
/// assigned so callers that want to `editMessageText` later (e.g. the
/// confirm bridge) can hold onto it. `parse_mode = None` sends
/// literal characters; `Some("HTML")` lets Telegram render `<b>`,
/// `<i>`, `<code>` etc. produced by `format_for_telegram`.
pub async fn send_message(
    http: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<i64> {
    send_message_inner(http, token, chat_id, text, None, parse_mode).await
}

/// `sendMessage` variant that attaches an inline_keyboard. Used by the
/// confirm bridge to render OpenClaw-style ✅ Allow / 🔏 Always / ❌ Deny
/// buttons. `callback_token` is encoded into each button's
/// `callback_data` as `cfm:<action>:<token>` so the App can match a
/// later `callback_query` back to the in-flight tool request.
pub async fn send_confirm_message(
    http: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    callback_token: &str,
) -> Result<i64> {
    let kb = confirm_keyboard(callback_token);
    // Confirm prompts are bot-authored plain text (no markdown), so no
    // parse_mode — the diff lines need to render literally.
    send_message_inner(http, token, chat_id, text, Some(kb), None).await
}

async fn send_message_inner(
    http: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    reply_markup: Option<serde_json::Value>,
    parse_mode: Option<&str>,
) -> Result<i64> {
    let url = format!("{API_ROOT}{token}/sendMessage");
    let mut body = serde_json::Map::new();
    body.insert("chat_id".into(), json!(chat_id));
    body.insert("text".into(), json!(text));
    if let Some(rm) = reply_markup {
        body.insert("reply_markup".into(), rm);
    }
    if let Some(mode) = parse_mode {
        body.insert("parse_mode".into(), json!(mode));
    }
    let resp = http
        .post(&url)
        .json(&serde_json::Value::Object(body))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("sendMessage failed ({})", resp.status()));
    }
    let env: SendMessageEnvelope = resp.json().await?;
    Ok(env.result.message_id)
}

/// Overwrite a previously-sent message. `keep_buttons=false` clears the
/// keyboard so the confirm prompt doesn't sit there looking tappable
/// after it's been resolved.
pub async fn edit_message(
    http: &reqwest::Client,
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
) -> Result<()> {
    let url = format!("{API_ROOT}{token}/editMessageText");
    let resp = http
        .post(&url)
        .json(&json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            // Explicitly send an empty inline_keyboard to clear the
            // buttons. Without this, Telegram leaves the old keyboard
            // attached to the edited message.
            "reply_markup": { "inline_keyboard": [] }
        }))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("editMessageText failed ({})", resp.status()));
    }
    Ok(())
}

/// Dismiss the loading spinner on the user's device. Must fire within
/// ~15s of the tap; the bot loop does this eagerly before forwarding
/// the callback to the App so we never miss the window.
pub async fn answer_callback(
    http: &reqwest::Client,
    token: &str,
    callback_id: &str,
    text: Option<&str>,
) -> Result<()> {
    let url = format!("{API_ROOT}{token}/answerCallbackQuery");
    let mut body = serde_json::Map::new();
    body.insert("callback_query_id".into(), json!(callback_id));
    if let Some(t) = text {
        body.insert("text".into(), json!(t));
    }
    let resp = http
        .post(&url)
        .json(&serde_json::Value::Object(body))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("answerCallbackQuery failed ({})", resp.status()));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct SendMessageEnvelope {
    result: SentMessageInfo,
}

#[derive(Debug, Deserialize)]
struct SentMessageInfo {
    message_id: i64,
}

/// Build the OpenClaw-style 3-button keyboard for a confirm prompt.
/// Each button's `callback_data` is `cfm:<action>:<token>` (well under
/// the 64-byte limit). The order — Allow / Always / Deny — matches
/// OpenClaw + JairFC; reading left-to-right is the natural "easy →
/// powerful → reject" gradient.
fn confirm_keyboard(token: &str) -> serde_json::Value {
    json!({
        "inline_keyboard": [[
            { "text": "✅ Allow",  "callback_data": format!("cfm:allow:{token}") },
            { "text": "🔏 Always", "callback_data": format!("cfm:always:{token}") },
            { "text": "❌ Deny",   "callback_data": format!("cfm:deny:{token}") },
        ]]
    })
}

/// 10-char base32 token packed into the `callback_data` payload. Same
/// alphabet as the pair-code generator (drops 0/O/1/I/L for human
/// legibility, even though humans won't type these). 30^10 ≈ 5e14 — no
/// realistic collision risk for the handful of confirms in flight at a
/// time.
pub fn generate_callback_token() -> String {
    use std::time::SystemTime;
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    let mut s = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xDECAFC0FFEE);
    let mut out = String::with_capacity(10);
    for _ in 0..10 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        out.push(ALPHABET[(s as usize) % ALPHABET.len()] as char);
    }
    out
}

/// Parsed `callback_data` from a confirm button tap. Returns `None`
/// if the payload doesn't match our `cfm:<action>:<token>` shape (e.g.
/// a stale button from a prior bot version, or a different feature
/// reusing the channel).
pub fn parse_callback_data(data: &str) -> Option<(CallbackAction, String)> {
    let mut parts = data.splitn(3, ':');
    let head = parts.next()?;
    if head != "cfm" {
        return None;
    }
    let action = match parts.next()? {
        "allow" => CallbackAction::Allow,
        "always" => CallbackAction::Always,
        "deny" => CallbackAction::Deny,
        _ => return None,
    };
    let token = parts.next()?.to_string();
    if token.is_empty() {
        return None;
    }
    Some((action, token))
}

/// Distinct outcomes the user can pick from the confirm keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackAction {
    Allow,
    Always,
    Deny,
}

/// Run the long-poll loop. Returns when `ctl_rx` receives Shutdown or the
/// channel closes. Logs lifecycle + errors to the TUI via `app_tx`.
pub async fn run(
    state: BotState,
    app_tx: mpsc::UnboundedSender<StreamMsg>,
    mut out_rx: mpsc::UnboundedReceiver<TelegramOut>,
    mut ctl_rx: mpsc::UnboundedReceiver<TelegramCtl>,
) {
    let http = match reqwest::Client::builder()
        // 5 s slack on top of the long-poll timeout so the request can
        // unwind cleanly even if Telegram holds the connection right up
        // to the wire.
        .timeout(Duration::from_secs(LONG_POLL_SECS + 5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = app_tx.send(StreamMsg::TelegramBotStatus(format!(
                "telegram: HTTP client build failed: {e}"
            )));
            return;
        }
    };

    let _ = app_tx.send(StreamMsg::TelegramBotStatus("telegram: bot online".into()));

    // Publish the slash menu so Telegram's UI shows the dropdown when
    // the user taps `/`. Best-effort — a failure here doesn't take the
    // bot down; the user can still type `/help` manually.
    if let Err(e) = set_my_commands(&http, &state.token).await {
        let _ = app_tx.send(StreamMsg::TelegramBotStatus(format!(
            "telegram: couldn't publish command menu ({e}) — typing slash commands still works"
        )));
    }

    let mut offset: i64 = 0;

    loop {
        tokio::select! {
            biased;
            ctl = ctl_rx.recv() => {
                match ctl {
                    Some(TelegramCtl::Shutdown) | None => {
                        let _ = app_tx.send(StreamMsg::TelegramBotStatus(
                            "telegram: bot stopped".into(),
                        ));
                        return;
                    }
                }
            }
            out = out_rx.recv() => {
                let Some(out) = out else { continue };
                handle_out(&http, &state.token, &app_tx, out).await;
            }
            updates = poll_updates(&http, &state.token, offset) => {
                match updates {
                    Ok(items) => {
                        for u in items {
                            offset = offset.max(u.update_id + 1);
                            if let Some(msg) = u.message {
                                handle_incoming(&http, &state, &app_tx, msg).await;
                            }
                            if let Some(cb) = u.callback_query {
                                handle_callback(&http, &state, &app_tx, cb).await;
                            }
                        }
                    }
                    Err(e) => {
                        let _ = app_tx.send(StreamMsg::TelegramBotStatus(format!(
                            "telegram: getUpdates failed: {e}"
                        )));
                        // Back off so a server-side hiccup doesn't melt
                        // the bot loop into a tight retry.
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}

async fn handle_out(
    http: &reqwest::Client,
    token: &str,
    app_tx: &mpsc::UnboundedSender<StreamMsg>,
    out: TelegramOut,
) {
    match out {
        TelegramOut::Send {
            chat_id,
            text,
            parse_mode,
        } => {
            if let Err(e) = send_message(http, token, chat_id, &text, parse_mode).await {
                let _ = app_tx.send(StreamMsg::TelegramBotStatus(format!(
                    "telegram: send to {chat_id} failed: {e}"
                )));
            }
        }
        TelegramOut::SendConfirm {
            chat_id,
            text,
            callback_token,
        } => match send_confirm_message(http, token, chat_id, &text, &callback_token).await {
            Ok(message_id) => {
                let _ = app_tx.send(StreamMsg::TelegramConfirmSent {
                    callback_token,
                    message_id,
                });
            }
            Err(e) => {
                let _ = app_tx.send(StreamMsg::TelegramBotStatus(format!(
                    "telegram: confirm send to {chat_id} failed: {e}"
                )));
            }
        },
        TelegramOut::EditMessage {
            chat_id,
            message_id,
            text,
        } => {
            if let Err(e) = edit_message(http, token, chat_id, message_id, &text).await {
                let _ = app_tx.send(StreamMsg::TelegramBotStatus(format!(
                    "telegram: edit msg {message_id} in {chat_id} failed: {e}"
                )));
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct GetUpdatesEnvelope {
    ok: bool,
    result: Option<Vec<Update>>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Update {
    update_id: i64,
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    callback_query: Option<CallbackQuery>,
}

#[derive(Debug, Deserialize)]
struct Message {
    chat: Chat,
    #[serde(default)]
    message_id: Option<i64>,
    from: Option<User>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Chat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct User {
    id: i64,
    username: Option<String>,
    first_name: Option<String>,
}

/// `CallbackQuery` shape — Telegram delivers this when a user taps an
/// inline_keyboard button. We need `id` (to answer), `from` (to
/// allowlist-check), the embedded message (for `chat_id` +
/// `message_id` so we can `editMessageText` later), and the raw
/// `data` payload that carries our `cfm:<action>:<token>`.
#[derive(Debug, Deserialize)]
struct CallbackQuery {
    id: String,
    from: User,
    #[serde(default)]
    message: Option<Message>,
    #[serde(default)]
    data: Option<String>,
}

async fn poll_updates(http: &reqwest::Client, token: &str, offset: i64) -> Result<Vec<Update>> {
    let url = format!("{API_ROOT}{token}/getUpdates");
    let resp = http
        .get(&url)
        .query(&[
            ("offset", offset.to_string()),
            ("timeout", LONG_POLL_SECS.to_string()),
            // Phase 5: also subscribe to button taps. Channel posts,
            // inline queries etc. stay excluded.
            (
                "allowed_updates",
                "[\"message\",\"callback_query\"]".to_string(),
            ),
        ])
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!("HTTP {}", resp.status()));
    }
    let body: GetUpdatesEnvelope = resp.json().await?;
    if !body.ok {
        return Err(anyhow!(
            "telegram API ok=false ({})",
            body.description.unwrap_or_default()
        ));
    }
    Ok(body.result.unwrap_or_default())
}

async fn handle_incoming(
    http: &reqwest::Client,
    state: &BotState,
    app_tx: &mpsc::UnboundedSender<StreamMsg>,
    msg: Message,
) {
    let chat_id = msg.chat.id;
    let from = match msg.from {
        Some(u) => u,
        None => return, // Channel post or anonymous — ignore in phase 1.
    };

    // Allowlisted? Surface the DM to the TUI and ack. Read the allowlist
    // under the mutex so a concurrent /telegram pair takes effect on the
    // very next message.
    let is_allowed = {
        let list = state.allowlist.lock().expect("telegram allowlist mutex");
        list.contains(&from.id)
    };

    if is_allowed {
        // No bot-side ack here in phase 2 — the TUI sends the model's
        // actual reply back as the response. If the TUI is busy / has
        // no model / errors, the handler DMs the diagnostic itself.
        let text = msg.text.unwrap_or_default();
        let display = display_handle(&from);
        let _ = app_tx.send(StreamMsg::TelegramIncoming {
            chat_id,
            from: display,
            text,
        });
        return;
    }

    // Unknown sender. Generate a pair code, store it, DM it back. We
    // deliberately don't surface the unknown sender's message text to
    // the TUI — that would let any stranger inject content into the
    // user's chat history before pairing.
    let code = generate_code();
    {
        let mut pending = state.pending.lock().expect("telegram pending mutex");
        pending.insert(code.clone(), (from.id, Instant::now()));
        // Opportunistic GC: drop expired codes whenever we touch the
        // map. Cheap (map is tiny — at most a handful of codes in flight)
        // and avoids a dedicated timer task.
        pending.retain(|_, (_, minted)| minted.elapsed() <= PAIR_CODE_TTL);
    }

    let body = format!(
        "Pairing code: {code}\n\n\
         In your Hibiscus terminal, run:\n\
         /telegram pair {code}\n\n\
         The code expires in 10 minutes."
    );
    if let Err(e) = send_message(http, &state.token, chat_id, &body, None).await {
        let _ = app_tx.send(StreamMsg::TelegramBotStatus(format!(
            "telegram: pair-code send failed: {e}"
        )));
    }
}

async fn handle_callback(
    http: &reqwest::Client,
    state: &BotState,
    app_tx: &mpsc::UnboundedSender<StreamMsg>,
    cb: CallbackQuery,
) {
    // Ack EARLY — Telegram's 15s deadline starts when the user taps,
    // and we don't want to lose it to the App's processing latency.
    // The user's spinner stops here; the visible result lands via the
    // editMessageText fired by the App handler.
    let _ = answer_callback(http, &state.token, &cb.id, None).await;

    // Allowlist check — strangers shouldn't be able to drive confirms
    // even if they somehow knew the callback_data shape.
    let is_allowed = {
        let list = state.allowlist.lock().expect("telegram allowlist mutex");
        list.contains(&cb.from.id)
    };
    if !is_allowed {
        // Don't surface to the App at all — this is the same silence
        // we give to unknown senders' text messages.
        return;
    }

    let Some(raw_data) = cb.data else { return };
    let Some((action, token)) = parse_callback_data(&raw_data) else {
        // Unknown callback shape (stale button from a prior version,
        // or some other feature using the same channel). Ignore.
        return;
    };
    let Some(msg) = cb.message else { return };
    let Some(message_id) = msg.message_id else {
        return;
    };

    let _ = app_tx.send(StreamMsg::TelegramCallback {
        callback_token: token,
        action,
        chat_id: msg.chat.id,
        message_id,
        from: display_handle(&cb.from),
        from_id: cb.from.id,
    });
}

fn display_handle(u: &User) -> String {
    if let Some(uname) = &u.username {
        format!("@{uname}")
    } else if let Some(name) = &u.first_name {
        name.clone()
    } else {
        format!("id:{}", u.id)
    }
}

/// 6-char pairing code. Alphabet excludes 0/O/1/I/L to avoid the classic
/// "is that an oh or a zero" confusion. ~30^6 ≈ 729M codes — plenty for
/// a map that rarely holds more than a handful at a time.
pub fn generate_code() -> String {
    use std::time::SystemTime;
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
    // Cheap PRNG: xorshift seeded from the system clock. Codes are
    // capability tokens, not crypto keys, and they live for 10 minutes
    // — a deterministic RNG would still need an unguessable seed, which
    // the clock provides at micro-second resolution.
    let mut s = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0xC0FFEE);
    let mut out = String::with_capacity(6);
    for _ in 0..6 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        out.push(ALPHABET[(s as usize) % ALPHABET.len()] as char);
    }
    out
}

/// Try to redeem a code. Returns the Telegram user id on success.
/// Expired or missing codes return `None`. Removes the entry on success
/// so a code can't be redeemed twice.
pub fn redeem_code(pending: &PendingCodes, code: &str) -> Option<i64> {
    let code = code.trim().to_ascii_uppercase();
    let mut map = pending.lock().expect("telegram pending mutex");
    if let Some((id, minted)) = map.get(&code).copied() {
        if minted.elapsed() <= PAIR_CODE_TTL {
            map.remove(&code);
            return Some(id);
        }
        // Expired — drop it so the user gets a clean "expired" error
        // next time too, not a stale entry.
        map.remove(&code);
    }
    None
}

/// Telegram caps `sendMessage` payloads at 4096 chars. We aim a little
/// lower so emoji and trailing newlines don't push us over after Telegram
/// re-encodes — and the buffer gives us headroom for a "(1/3)" prefix
/// later if we want one.
pub const TELEGRAM_MSG_MAX_CHARS: usize = 4000;

/// Convert one Markdown-ish AI reply into HTML chunks ready for
/// `sendMessage` with `parse_mode: "HTML"`. Chunks the raw text first
/// (so we never split inside a triple-backtick fence by surprise) and
/// then converts each chunk independently, carrying the "are we still
/// inside a code fence?" state across boundaries so a single long
/// block that crosses chunks renders as `<pre><code>…</code></pre>`
/// per chunk instead of one giant unterminated tag.
///
/// Empty input → empty vec (callers skip the sendMessage call).
pub fn format_for_telegram(text: &str) -> Vec<String> {
    let chunks = chunk_for_telegram(text);
    if chunks.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(chunks.len());
    let mut in_fence = false;
    for chunk in chunks {
        let (html, ended_in_fence) = render_html_chunk(&chunk, in_fence);
        out.push(html);
        in_fence = ended_in_fence;
    }
    out
}

/// Convert one chunk of Markdown text to Telegram HTML. `starts_in_fence`
/// = true means a triple-backtick block from the previous chunk hasn't
/// closed yet, so we open `<pre><code>` at the top. Returns the HTML and
/// whether this chunk also leaves a fence unclosed (caller forwards to
/// the next chunk).
fn render_html_chunk(text: &str, starts_in_fence: bool) -> (String, bool) {
    let mut out = String::new();
    let mut in_fence = starts_in_fence;
    if in_fence {
        out.push_str("<pre><code>");
    }
    for line in text.split_inclusive('\n') {
        // Detect a fence open/close. The ``` line itself isn't part of
        // the rendered output — we just toggle state.
        let trimmed = line.trim_end_matches('\n').trim_start();
        if trimmed.starts_with("```") {
            if in_fence {
                out.push_str("</code></pre>\n");
                in_fence = false;
            } else {
                out.push_str("<pre><code>");
                in_fence = true;
            }
            continue;
        }
        if in_fence {
            out.push_str(&escape_html(line));
        } else {
            out.push_str(&render_outside_fence(line));
        }
    }
    if in_fence {
        // Force-close so this chunk's HTML is balanced; the next chunk
        // re-opens via `starts_in_fence`. Telegram's HTML parser
        // rejects unbalanced tags with 400.
        out.push_str("</code></pre>");
    }
    (out, in_fence)
}

/// Outside a code fence we recognise a tiny markdown subset on each
/// line: ATX headings (`#`/`##`/`###`) become `<b>`. Everything else
/// goes through `convert_inline_md` for backticks / bold / italic / links.
fn render_outside_fence(line: &str) -> String {
    // Preserve the trailing newline if present so paragraph spacing
    // survives the conversion.
    let (body, nl) = match line.strip_suffix('\n') {
        Some(b) => (b, "\n"),
        None => (line, ""),
    };
    for marker in ["### ", "## ", "# "] {
        if let Some(rest) = body.strip_prefix(marker) {
            return format!("<b>{}</b>{nl}", convert_inline_md(rest));
        }
    }
    format!("{}{nl}", convert_inline_md(body))
}

/// Parse inline markdown on one line: backticks → `<code>`, `**` → `<b>`,
/// `*` → `<i>`, `[text](url)` → `<a>`. Anything else gets HTML-escaped
/// so a stray `<` or `&` doesn't break Telegram's HTML parser.
///
/// Implementation is a single-pass byte walk because the delimiters are
/// all ASCII (`*` `` ` `` `[` `]` `(` `)` `<` `>` `&`). UTF-8 elsewhere
/// passes through unchanged.
fn convert_inline_md(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        let b = bytes[i];
        match b {
            b'`' => {
                if let Some(end) = find_byte(bytes, b'`', i + 1) {
                    let inner = &line[i + 1..end];
                    out.push_str("<code>");
                    out.push_str(&escape_html(inner));
                    out.push_str("</code>");
                    i = end + 1;
                    continue;
                }
                out.push('`');
                i += 1;
            }
            b'*' => {
                // `**bold**` first — peek for the doubled marker so we
                // don't mis-parse it as two empty italics.
                if i + 1 < n && bytes[i + 1] == b'*' {
                    if let Some(end) = find_bytes(bytes, b"**", i + 2) {
                        let inner = &line[i + 2..end];
                        out.push_str("<b>");
                        out.push_str(&convert_inline_md(inner));
                        out.push_str("</b>");
                        i = end + 2;
                        continue;
                    }
                    out.push_str("**");
                    i += 2;
                    continue;
                }
                if let Some(end) = find_byte(bytes, b'*', i + 1) {
                    let inner = &line[i + 1..end];
                    // Avoid converting `* item` (list bullet at start of
                    // line) — bare `* ` with whitespace after isn't
                    // italic. Cheapest check: if the closing `*` would
                    // wrap zero/whitespace content, skip.
                    if !inner.is_empty() && !inner.starts_with(char::is_whitespace) {
                        out.push_str("<i>");
                        out.push_str(&convert_inline_md(inner));
                        out.push_str("</i>");
                        i = end + 1;
                        continue;
                    }
                }
                out.push('*');
                i += 1;
            }
            b'[' => {
                // `[text](url)` — strict shape; bail to literal `[` if
                // either the `]` or `(` isn't where we expect it.
                if let Some(close_text) = find_byte(bytes, b']', i + 1) {
                    if close_text + 1 < n && bytes[close_text + 1] == b'(' {
                        if let Some(close_url) = find_byte(bytes, b')', close_text + 2) {
                            let inner = &line[i + 1..close_text];
                            let url = &line[close_text + 2..close_url];
                            out.push_str("<a href=\"");
                            out.push_str(&escape_html(url));
                            out.push_str("\">");
                            out.push_str(&escape_html(inner));
                            out.push_str("</a>");
                            i = close_url + 1;
                            continue;
                        }
                    }
                }
                out.push('[');
                i += 1;
            }
            b'<' => {
                out.push_str("&lt;");
                i += 1;
            }
            b'>' => {
                out.push_str("&gt;");
                i += 1;
            }
            b'&' => {
                out.push_str("&amp;");
                i += 1;
            }
            _ => {
                // Forward the next full char (could be multi-byte UTF-8).
                let c = line[i..]
                    .chars()
                    .next()
                    .expect("non-empty slice at known boundary");
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            c => out.push(c),
        }
    }
    out
}

fn find_byte(haystack: &[u8], needle: u8, start: usize) -> Option<usize> {
    haystack[start..]
        .iter()
        .position(|&b| b == needle)
        .map(|p| start + p)
}

fn find_bytes(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    for i in start..=haystack.len().saturating_sub(needle.len()) {
        if &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

/// Split an assistant reply into Telegram-sized chunks. Prefers newline
/// boundaries: pack whole lines until the next one would overflow, then
/// emit. A single line longer than `max` falls back to a char-by-char
/// hard split — rare for chat text, but we still don't want to panic on
/// a giant code block.
pub fn chunk_for_telegram(text: &str) -> Vec<String> {
    chunk_with_max(text, TELEGRAM_MSG_MAX_CHARS)
}

fn chunk_with_max(text: &str, max: usize) -> Vec<String> {
    let trimmed = text.trim_end_matches('\n');
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_chars = 0usize;

    for line in trimmed.split_inclusive('\n') {
        let line_chars = line.chars().count();
        // Whole-line case: line fits as-is into a (possibly empty) chunk.
        if current_chars + line_chars > max {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_chars = 0;
            }
            if line_chars <= max {
                current.push_str(line);
                current_chars = line_chars;
                continue;
            }
            // Single line bigger than the limit — hard split by chars.
            for c in line.chars() {
                if current_chars + 1 > max {
                    chunks.push(std::mem::take(&mut current));
                    current_chars = 0;
                }
                current.push(c);
                current_chars += 1;
            }
        } else {
            current.push_str(line);
            current_chars += line_chars;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_are_six_chars_from_alphabet() {
        let code = generate_code();
        assert_eq!(code.len(), 6);
        const ALPHABET: &str = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";
        for c in code.chars() {
            assert!(ALPHABET.contains(c), "unexpected char in code: {c}");
        }
    }

    #[test]
    fn redeem_consumes_a_valid_code() {
        let pending = new_pending_map();
        let code = "ABC123".to_string();
        pending
            .lock()
            .unwrap()
            .insert(code.clone(), (42, Instant::now()));
        assert_eq!(redeem_code(&pending, &code), Some(42));
        // Second redeem should fail — single-use.
        assert_eq!(redeem_code(&pending, &code), None);
    }

    #[test]
    fn redeem_rejects_an_expired_code() {
        let pending = new_pending_map();
        let code = "EXPIRE".to_string();
        let stale = Instant::now() - PAIR_CODE_TTL - Duration::from_secs(1);
        pending.lock().unwrap().insert(code.clone(), (99, stale));
        assert_eq!(redeem_code(&pending, &code), None);
        // Expired entry should be GC'd on the way out.
        assert!(pending.lock().unwrap().get(&code).is_none());
    }

    #[test]
    fn redeem_is_case_insensitive() {
        let pending = new_pending_map();
        pending
            .lock()
            .unwrap()
            .insert("ABCDEF".into(), (7, Instant::now()));
        assert_eq!(redeem_code(&pending, "abcdef"), Some(7));
    }

    #[test]
    fn chunker_returns_single_chunk_when_short() {
        let chunks = chunk_for_telegram("hello world");
        assert_eq!(chunks, vec!["hello world".to_string()]);
    }

    #[test]
    fn chunker_drops_empty_input() {
        assert!(chunk_for_telegram("").is_empty());
        assert!(chunk_for_telegram("\n\n\n").is_empty());
    }

    #[test]
    fn chunker_splits_on_line_boundaries() {
        // Three 50-char lines + newlines — must split into two chunks at
        // max=120 (room for two lines but not three).
        let line = "a".repeat(50);
        let text = format!("{line}\n{line}\n{line}");
        let chunks = chunk_with_max(&text, 120);
        assert_eq!(chunks.len(), 2);
        // Each chunk respects the cap.
        for c in &chunks {
            assert!(c.chars().count() <= 120);
        }
    }

    #[test]
    fn chunker_hard_splits_oversize_line() {
        // No newlines, longer than max — falls back to char-split.
        let text = "x".repeat(2500);
        let chunks = chunk_with_max(&text, 1000);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 1000);
        assert_eq!(chunks[1].len(), 1000);
        assert_eq!(chunks[2].len(), 500);
    }

    #[test]
    fn chunker_preserves_unicode() {
        // 6-char UTF-8 emojis, mix with ASCII. Ensure we count chars not
        // bytes (UTF-8 emoji is multi-byte) and never split mid-codepoint.
        let text = "👋 hi there\n".repeat(20);
        let chunks = chunk_for_telegram(&text);
        let total: String = chunks.join("");
        assert_eq!(total.trim_end_matches('\n'), text.trim_end_matches('\n'));
    }

    // --- markdown_to_telegram_html / convert_inline_md ---

    fn html(s: &str) -> String {
        // Single-chunk convenience for tests.
        format_for_telegram(s).join("")
    }

    #[test]
    fn html_escapes_specials() {
        assert_eq!(html("1 < 2 & 3 > 0"), "1 &lt; 2 &amp; 3 &gt; 0");
    }

    #[test]
    fn html_passes_plain_text() {
        assert_eq!(html("hello world"), "hello world");
    }

    #[test]
    fn html_converts_inline_code() {
        assert_eq!(html("call `foo()` here"), "call <code>foo()</code> here");
    }

    #[test]
    fn html_escapes_inside_inline_code() {
        // `<` inside backticks should still be escaped before going
        // into <code>…</code>.
        assert_eq!(html("`a < b`"), "<code>a &lt; b</code>");
    }

    #[test]
    fn html_converts_bold_and_italic() {
        assert_eq!(html("**big** and *small*"), "<b>big</b> and <i>small</i>");
    }

    #[test]
    fn html_ignores_list_bullets() {
        // `* foo` (with trailing space) is a list bullet, not italic —
        // converter should leave it alone. The chunker strips trailing
        // newlines, so the final \n is dropped on the way out.
        let got = html("* one\n* two\n");
        assert_eq!(got, "* one\n* two");
    }

    #[test]
    fn html_converts_link() {
        assert_eq!(
            html("see [docs](https://example.com)"),
            "see <a href=\"https://example.com\">docs</a>"
        );
    }

    #[test]
    fn html_converts_heading() {
        // Same as above: trailing newline from input is trimmed by the
        // chunker; the result is `<b>Title</b>` without an end-line.
        assert_eq!(html("# Title\n"), "<b>Title</b>");
        assert_eq!(html("## Sub\n"), "<b>Sub</b>");
    }

    #[test]
    fn html_wraps_triple_backtick_block() {
        let got = html("here:\n```\nfn main() {}\n```\n");
        assert!(got.contains("<pre><code>"));
        assert!(got.contains("fn main() {}"));
        assert!(got.contains("</code></pre>"));
    }

    #[test]
    fn html_escapes_inside_fenced_block() {
        let got = html("```\n<html>&\n```\n");
        assert!(got.contains("&lt;html&gt;&amp;"));
    }

    #[test]
    fn html_keeps_unicode() {
        assert_eq!(html("👋 hello"), "👋 hello");
    }

    #[test]
    fn html_balances_cross_chunk_fence() {
        // A long fenced block that crosses a chunk boundary should
        // close in chunk N and re-open in chunk N+1, so each chunk's
        // HTML is balanced when Telegram parses it.
        let body = "x".repeat(2000);
        let text = format!("```\n{body}\n{body}\n{body}\n```\n");
        let chunks = format_for_telegram(&text);
        assert!(chunks.len() >= 2);
        for chunk in &chunks {
            let opens = chunk.matches("<pre><code>").count();
            let closes = chunk.matches("</code></pre>").count();
            assert_eq!(
                opens, closes,
                "unbalanced pre/code in chunk: opens={opens} closes={closes}"
            );
        }
    }

    #[test]
    fn html_drops_empty() {
        assert!(format_for_telegram("").is_empty());
        assert!(format_for_telegram("\n\n").is_empty());
    }
}
