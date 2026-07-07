//! Shell tool schema: `run_command`. Requires user approval.

use serde_json::json;

use crate::ollama::Tool;

pub(super) fn shell_tools() -> Vec<Tool> {
    vec![Tool::function(
        "run_command",
        "Run a shell command. User must approve before it executes. Use only when \
         a tool above can't get the info (e.g. `cargo check`, `wc -l`, version queries). \
         Never use for file reads (use read_file) or directory listings (use list_dir). \
         30 s timeout, output capped at ~4 KB.",
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command (run via sh -c)"}
            },
            "required": ["command"]
        }),
    )]
}
