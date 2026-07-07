//! `consult_specialist` — conditionally appended.
//!
//! Lives in its own file because the schema depends on the live
//! specialist roster (each one's name + task description goes into
//! the tool's description + `name` enum), and the registration is
//! gated on the roster being non-empty. `tool_definitions_with` in
//! the parent module calls `consult_specialist_tool` only when at
//! least one specialist is configured.

use serde_json::json;

use crate::agent::SpecialistRunner;
use crate::ollama::Tool;

pub(super) fn consult_specialist_tool(specialists: &[SpecialistRunner]) -> Tool {
    let roster = specialists
        .iter()
        .map(|s| format!("- {}: {}", s.name, s.task))
        .collect::<Vec<_>>()
        .join("\n");
    let names: Vec<String> = specialists.iter().map(|s| s.name.clone()).collect();
    let desc = format!(
        "Consult another model with a different specialty. The specialist runs in an \
         ISOLATED context (it does NOT see this conversation), but it has its OWN \
         read-only tools: `read_file`, `list_dir`, `find_files`, `git_status`, \
         `git_log`, `git_diff`, `git_show`, `read_memory`. The specialist returns \
         one consolidated reply you can use.\n\n\
         Available specialists:\n{roster}\n\n\
         DELEGATION RULES (read carefully — these matter):\n\
         1. Pass `query` as a short task instruction, ONE paragraph max. Reference \
            file PATHS, never paste file contents — the specialist reads them itself.\n\
         2. DO NOT `read_file` (or any read tool) yourself just to paste the result \
            into `query`. That duplicates work and wastes tokens. The whole point of \
            delegating is to hand off the reading too.\n\
         3. Include ONLY decisions, constraints, and prior conclusions the specialist \
            needs — never raw source. The specialist will fetch what it needs.\n\
         4. When chaining specialists (A → B → C), pass each one paths + the prior \
            specialist's *conclusion*, not the file contents they read.\n\n\
         Good query: \"Read src/agent.rs and list every function with a one-line role.\"\n\
         Bad query: \"Here is the file: ```rust\\n...3000 chars...\\n```\""
    );

    Tool::function(
        "consult_specialist",
        &desc,
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "enum": names,
                    "description": "Which specialist to consult. Must be one of the names listed above."
                },
                "query": {
                    "type": "string",
                    "description": "Short task instruction (one paragraph). Reference file paths — the specialist has its own read_file. NEVER paste file contents here."
                }
            },
            "required": ["name", "query"]
        }),
    )
}
