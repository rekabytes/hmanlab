/// Local session persistence — JSONL files, one per session, no server needed.
///
/// Layout:
///   ~/.config/hmanlab/sessions/<cwd-hash>/<session-uuid>.jsonl
///
/// Each line is a JSON object representing one event in the conversation.
/// The file is append-only; loading replays all lines in order.
///
/// Record types:
///   - "meta"      — first line written when a session starts; carries title + model
///   - "user"      — a user message (role: user)
///   - "assistant" — an assistant reply (role: assistant)
///   - "tool"      — a tool result (role: tool, name: function name)
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// On-disk record
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Record {
    /// "meta" | "user" | "assistant" | "tool"
    #[serde(rename = "type")]
    pub kind: String,
    /// Shared session identifier (UUID v4).
    pub session_id: String,
    /// Per-record UUID — unique identifier for this line.
    pub uuid: String,
    /// ISO-8601 timestamp.
    pub timestamp: String,
    /// Message content (empty for meta records).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    /// Tool name (only set when kind == "tool").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Model in use at the time of this record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Human-readable session title (only meaningful on meta records,
    /// derived from the first user message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

// ---------------------------------------------------------------------------
// Session summary (for the /sessions picker)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub session_id: String,
    pub title: String,
    pub model: Option<String>,
    /// Modification time of the file (seconds since epoch). Used for sorting.
    pub updated_at: u64,
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

/// Base directory for all session files.
pub fn sessions_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(PathBuf::from(home).join(".config/hmanlab/sessions"))
}

/// Directory for sessions in a specific working directory.
/// The cwd is hashed to a short hex string so paths with special chars work.
pub fn session_dir_for_cwd(cwd: &std::path::Path) -> Result<PathBuf> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    cwd.hash(&mut h);
    let hash = format!("{:016x}", h.finish());
    Ok(sessions_dir()?.join(hash))
}

/// Path to a specific session file.
pub fn session_path(cwd: &std::path::Path, session_id: &str) -> Result<PathBuf> {
    Ok(session_dir_for_cwd(cwd)?.join(format!("{session_id}.jsonl")))
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Generate a new unique session ID.
pub fn new_session_id() -> String {
    new_id()
}

fn new_id() -> String {
    // Generate a pseudo-UUID from time + pid + counter.
    // Format: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx (UUID v4 shape)
    use std::sync::atomic::{AtomicU64, Ordering};
    static CTR: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let c = CTR.fetch_add(1, Ordering::Relaxed);
    let p = std::process::id() as u64;
    let a = t ^ (p << 32) ^ (c.wrapping_mul(6364136223846793005));
    let b = t.wrapping_shr(32) ^ c.wrapping_add(p);
    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (a >> 32) & 0xffff_ffff,
        (a >> 16) & 0xffff,
        a & 0x0fff,
        ((b >> 48) & 0x3fff) | 0x8000,
        b & 0x0000_ffff_ffff_ffff,
    )
}

fn now_iso() -> String {
    // Use std time, format manually to avoid a chrono dep.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Rough ISO-8601 UTC from epoch seconds (good enough for display).
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Approximate Gregorian date from day count (enough for display, not authoritative).
    let (year, month, day) = days_to_ymd(days);
    format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Epoch = 1970-01-01
    let mut year = 1970u64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let months = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &dim in &months {
        if days < dim {
            break;
        }
        days -= dim;
        month += 1;
    }
    (year, month, days + 1)
}

fn append_record(path: &PathBuf, rec: &Record) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let line = serde_json::to_string(rec)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Write the session-open meta record. Called once per session on first user message.
pub fn write_meta(path: &PathBuf, session_id: &str, title: &str, model: &str) -> Result<()> {
    let rec = Record {
        kind: "meta".into(),
        session_id: session_id.into(),
        uuid: new_id(),
        timestamp: now_iso(),
        content: String::new(),
        name: None,
        model: Some(model.into()),
        title: Some(title.into()),
    };
    append_record(path, &rec)
}

/// Append a user message.
pub fn write_user(path: &PathBuf, session_id: &str, content: &str, model: &str) -> Result<()> {
    let rec = Record {
        kind: "user".into(),
        session_id: session_id.into(),
        uuid: new_id(),
        timestamp: now_iso(),
        content: content.into(),
        name: None,
        model: Some(model.into()),
        title: None,
    };
    append_record(path, &rec)
}

/// Append an assistant reply.
pub fn write_assistant(path: &PathBuf, session_id: &str, content: &str, model: &str) -> Result<()> {
    let rec = Record {
        kind: "assistant".into(),
        session_id: session_id.into(),
        uuid: new_id(),
        timestamp: now_iso(),
        content: content.into(),
        name: None,
        model: Some(model.into()),
        title: None,
    };
    append_record(path, &rec)
}

/// Append a tool result.
pub fn write_tool(path: &PathBuf, session_id: &str, name: &str, output: &str) -> Result<()> {
    let rec = Record {
        kind: "tool".into(),
        session_id: session_id.into(),
        uuid: new_id(),
        timestamp: now_iso(),
        content: output.into(),
        name: Some(name.into()),
        model: None,
        title: None,
    };
    append_record(path, &rec)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Load all records from a session file, in order.
pub fn load_records(path: &PathBuf) -> Result<Vec<Record>> {
    let content = std::fs::read_to_string(path)?;
    let mut records = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // skip malformed lines
        if let Ok(r) = serde_json::from_str::<Record>(line) {
            records.push(r);
        }
    }
    Ok(records)
}

/// Convert loaded records into `ChatMessage` objects for the UI.
pub fn records_to_chat(records: &[Record]) -> Vec<crate::ollama::ChatMessage> {
    records
        .iter()
        .filter(|r| matches!(r.kind.as_str(), "user" | "assistant" | "tool"))
        .map(|r| crate::ollama::ChatMessage {
            role: match r.kind.as_str() {
                "user" => "user".into(),
                "assistant" => "assistant".into(),
                "tool" => "tool".into(),
                other => other.into(),
            },
            content: r.content.clone(),
            name: r.name.clone(),
            ..Default::default()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

/// List all sessions for a given working directory, newest first.
pub fn list_sessions(cwd: &std::path::Path) -> Result<Vec<SessionSummary>> {
    let dir = session_dir_for_cwd(cwd)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut summaries = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if session_id.is_empty() {
            continue;
        }
        let updated_at = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Read the first few lines to get title + model.
        let (title, model) = read_summary(&path);
        summaries.push(SessionSummary {
            session_id,
            title,
            model,
            updated_at,
            path,
        });
    }
    summaries.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    Ok(summaries)
}

/// Extract title and model from a session file's meta + first user record.
fn read_summary(path: &PathBuf) -> (String, Option<String>) {
    let Ok(f) = std::fs::File::open(path) else {
        return ("(unreadable)".into(), None);
    };
    use std::io::BufRead;
    let reader = std::io::BufReader::new(f);
    let mut title = String::new();
    let mut model: Option<String> = None;

    for line in reader.lines().take(20) {
        let Ok(line) = line else { break };
        let Ok(rec) = serde_json::from_str::<Record>(&line) else {
            continue;
        };
        if rec.kind == "meta" {
            if let Some(t) = rec.title {
                title = t;
            }
            if model.is_none() {
                model = rec.model;
            }
        }
        if rec.kind == "user" && title.is_empty() {
            // Fall back: derive title from first user message.
            title = truncate_title(&rec.content);
        }
        if !title.is_empty() && model.is_some() {
            break;
        }
    }
    if title.is_empty() {
        title = "(untitled)".into();
    }
    (title, model)
}

/// Derive a display title from the first user message (≤60 chars, first line only).
pub fn truncate_title(s: &str) -> String {
    let first_line = s.lines().next().unwrap_or(s).trim();
    if first_line.len() <= 60 {
        first_line.to_string()
    } else {
        format!("{}…", &first_line[..59])
    }
}

/// Find a session by a short UUID prefix (≥4 hex chars, dashes ignored).
pub fn find_by_prefix<'a>(
    summaries: &'a [SessionSummary],
    prefix: &str,
) -> Result<&'a SessionSummary> {
    let clean: String = prefix
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>()
        .to_lowercase();
    if clean.len() < 4 {
        anyhow::bail!("prefix too short (need ≥4 hex chars)");
    }
    let matches: Vec<&SessionSummary> = summaries
        .iter()
        .filter(|s| {
            s.session_id
                .replace('-', "")
                .to_lowercase()
                .starts_with(&clean)
        })
        .collect();
    match matches.len() {
        0 => anyhow::bail!("no session matches '{prefix}'"),
        1 => Ok(matches[0]),
        n => anyhow::bail!("{n} sessions match '{prefix}' — use a longer prefix"),
    }
}
