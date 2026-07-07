//! Git tool schemas: `git_status`, `git_log`, `git_diff`, `git_show`.
//!
//! Read-only — all four shell out to `git` under the hood and never need
//! user approval. Implementations live in `tools/git.rs`.

use serde_json::json;

use crate::ollama::Tool;

pub(super) fn git_tools() -> Vec<Tool> {
    vec![
        Tool::function(
            "git_status",
            "Show working-tree status. Use when the user asks what's changed, what's \
             staged, or what's modified.",
            json!({"type": "object", "properties": {}}),
        ),
        Tool::function(
            "git_log",
            "Show recent commits. Use for 'what changed recently', 'who worked on X', or \
             history questions.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "description": "Commits to show (default 10)"}
                }
            }),
        ),
        Tool::function(
            "git_diff",
            "Show a git diff. Use to inspect actual line-level changes. Without args, \
             shows unstaged changes.",
            json!({
                "type": "object",
                "properties": {
                    "range": {"type": "string", "description": "Optional commit range like HEAD~3..HEAD"},
                    "path":  {"type": "string", "description": "Optional path filter"}
                }
            }),
        ),
        Tool::function(
            "git_show",
            "Show a commit's full message + line-level diff by hash or ref. Use for \
             'what did commit X change' or 'read the latest commit' (pass rev: \"HEAD\"). \
             For very large commits the output is tail-truncated — fall back to git_diff \
             with a `path:` filter if you need to drill into one file.",
            json!({
                "type": "object",
                "properties": {
                    "rev": {"type": "string", "description": "Commit hash or ref (e.g. HEAD, HEAD~1, abc1234)"}
                },
                "required": ["rev"]
            }),
        ),
    ]
}
