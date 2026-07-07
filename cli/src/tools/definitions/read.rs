//! Read-only tool schemas: `read_file`, `list_dir`, `find_files`.
//!
//! These never need user approval — implementations live in `tools/read.rs`.
//! Descriptions tell the model when to reach for each, plus the line-number
//! contract for `read_file` output that `edit_file` / `multi_edit` rely on.

use serde_json::json;

use crate::ollama::Tool;

pub(super) fn read_tools() -> Vec<Tool> {
    vec![
        Tool::function(
            "read_file",
            "Read a text file from the workspace. Use when you need to see actual file \
             contents. Don't ask the user 'should I read X?' — if the question requires \
             knowing what's in a file, just call this. Output capped at ~50 KB. Each line is \
             returned prefixed with `<line_no>\\t<content>` — the line numbers are a \
             coordinate system you can refer to in chat, but you MUST strip the `<n>\\t` \
             prefix before quoting content into edit_file / multi_edit `old_string` (the \
             tool matches against the raw file, which has no prefixes).",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path (relative to workspace, e.g. 'src/main.rs')"
                    }
                },
                "required": ["path"]
            }),
        ),
        Tool::function(
            "list_dir",
            "List entries of a directory. Use at the start of any 'explore', 'summarize', \
             or 'what's in this repo' task to get the lay of the land before reading files. \
             Skip when the user already named the specific file they care about.",
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (default: workspace root)"
                    }
                }
            }),
        ),
        Tool::function(
            "find_files",
            "Find files matching a glob. Use when the user asks about a category of files \
             ('all rust files', 'every test file'). Prefer narrow globs like 'src/**/*.rs' \
             over '**/*' — broad globs return huge lists. Build/cache dirs (target, \
             node_modules, .git) are auto-filtered.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "Glob like '*.rs' or 'src/**/*.toml'. Avoid '**/*'."
                    }
                },
                "required": ["pattern"]
            }),
        ),
    ]
}
