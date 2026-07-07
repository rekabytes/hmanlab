/// Minimal types kept for the session picker. The full hmanlab-stack client
/// has been removed — sessions are now persisted locally as JSONL files.
use serde::{Deserialize, Serialize};

/// A session row shown in the `/sessions` picker.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub model: Option<String>,
}
