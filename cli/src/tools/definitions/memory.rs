//! Memory tool schemas: `save_memory`, `read_memory`, `forget_memory`.
//!
//! Persistent across sessions; `save_memory` and `forget_memory` require
//! user approval. The implementation in `tools/memory_tools.rs` writes to
//! `~/.hmanlab/memory/` (user scope) or `<workspace>/.hmanlab/memory/`
//! (project scope), and the `MEMORY.md` index is auto-rebuilt + loaded
//! into the system prompt every turn.

use serde_json::json;

use crate::ollama::Tool;

pub(super) fn memory_tools() -> Vec<Tool> {
    vec![
        Tool::function(
            "save_memory",
            "Save a durable memory under ~/.hmanlab/memory/ (user scope) or \
             <workspace>/.hmanlab/memory/ (project scope). Use for facts that should \
             survive the session: user preferences, project decisions, behaviour \
             corrections, references to external systems. User approves before write. \
             The MEMORY.md index is auto-rebuilt and re-loaded into the system prompt \
             next turn. Keep `description` ≤200 chars — it sits in the always-loaded index.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["user", "project"],
                        "description": "'user' for facts that apply to the human across every project; 'project' for facts specific to this codebase"
                    },
                    "name": {
                        "type": "string",
                        "description": "Kebab-case slug used as the filename (e.g. 'user-role-data-scientist'). ASCII alphanumerics, '-' and '_' only."
                    },
                    "type": {
                        "type": "string",
                        "enum": ["user", "project", "feedback", "reference"],
                        "description": "user = profile facts; project = state of work; feedback = behaviour rules; reference = pointers to external systems"
                    },
                    "description": {
                        "type": "string",
                        "description": "One-line summary (≤200 chars) that lands in the always-loaded MEMORY.md index"
                    },
                    "body": {
                        "type": "string",
                        "description": "Memory content in markdown. Cross-link with [[other-name]]. Keep ≤16 KB."
                    }
                },
                "required": ["scope", "name", "type", "description", "body"]
            }),
        ),
        Tool::function(
            "read_memory",
            "Fetch the full body of a memory by slug. The system prompt's MEMORY.md \
             index lists what's available — call this when you need the actual content \
             of one (the body is not in the prompt by default).",
            json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["user", "project"]},
                    "name": {"type": "string", "description": "Slug from the MEMORY.md index"}
                },
                "required": ["scope", "name"]
            }),
        ),
        Tool::function(
            "forget_memory",
            "Delete a memory. Use only when the user explicitly asks to forget \
             something, or when you've saved a corrected memory that replaces an older \
             wrong one. User approves before deletion.",
            json!({
                "type": "object",
                "properties": {
                    "scope": {"type": "string", "enum": ["user", "project"]},
                    "name": {"type": "string"}
                },
                "required": ["scope", "name"]
            }),
        ),
    ]
}
