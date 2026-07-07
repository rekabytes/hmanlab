//! The model-facing surface: tool schemas + system prompt.
//!
//! Schemas are split by category into sibling files under
//! `tools/definitions/`. This file is the orchestrator: it concatenates
//! the per-category lists into the final `tool_definitions()` Vec,
//! re-exports `system_prompt`, and hosts the conditional
//! `consult_specialist` registration in `tool_definitions_with`.
//!
//! IMPORTANT: when adding/removing tools or changing the system prompt,
//! mirror the change to `hmanlab-api/src/finetune.ts::TRAINING_SYSTEM_PROMPT`
//! so fine-tuned models stay in sync with the live prompt.

mod edit;
mod git;
mod memory;
mod prompt;
mod read;
mod shell;
mod specialist;
mod websearch;

use crate::agent::SpecialistRunner;
use crate::ollama::Tool;

pub use prompt::system_prompt;

/// Build the static tool surface — the set every agent turn sees,
/// regardless of specialist roster. Specialists are appended by
/// [`tool_definitions_with`] when at least one is configured.
pub fn tool_definitions() -> Vec<Tool> {
    let mut defs = Vec::with_capacity(20);
    defs.extend(read::read_tools());
    defs.extend(git::git_tools());
    defs.extend(edit::edit_tools());
    defs.extend(shell::shell_tools());
    defs.extend(memory::memory_tools());
    defs
}

/// Build the live tool surface for the main agent. Wraps
/// [`tool_definitions`] with a conditional `consult_specialist` entry —
/// registered only when there's at least one specialist available, so
/// the model never sees a tool it can't call. The description embeds
/// the per-specialist `task` lines so the main model knows when each
/// route makes sense.
///
/// `active_mcp` — when `Some(provider_id)`, the `web_search` and
/// `web_fetch` tools are appended so the model can search the web.
/// `None` omits them entirely (provider not configured).
pub fn tool_definitions_with(specialists: &[SpecialistRunner], active_mcp: Option<&str>) -> Vec<Tool> {
    let mut defs = tool_definitions();
    if active_mcp.is_some() {
        defs.extend(websearch::websearch_tools());
    }
    if !specialists.is_empty() {
        defs.push(specialist::consult_specialist_tool(specialists));
    }
    defs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::LlmBackend;
    use crate::ollama::Client;

    fn dummy_runner(name: &str, task: &str) -> SpecialistRunner {
        SpecialistRunner {
            name: name.into(),
            model: "test-model".into(),
            task: task.into(),
            system_prompt: "test-prompt".into(),
            backend: LlmBackend::Ollama(Client::new("http://127.0.0.1:0".into())),
        }
    }

    fn has_tool(defs: &[crate::ollama::Tool], name: &str) -> bool {
        defs.iter().any(|t| t.function.name == name)
    }

    #[test]
    fn empty_specialists_omits_consult_tool() {
        // No runners → no `consult_specialist` registration. The main
        // model never sees a tool it can't actually use.
        let defs = tool_definitions_with(&[], None);
        assert!(!has_tool(&defs, "consult_specialist"));
        // Sanity: the standard tools are still there.
        assert!(has_tool(&defs, "read_file"));
        assert!(has_tool(&defs, "edit_file"));
    }

    #[test]
    fn nonempty_specialists_appends_consult_tool() {
        let runners = vec![
            dummy_runner("coder", "use when writing code"),
            dummy_runner("reviewer", "use when reviewing changes"),
        ];
        let defs = tool_definitions_with(&runners, None);
        assert!(has_tool(&defs, "consult_specialist"));
        // The standard tools are still there too.
        assert!(has_tool(&defs, "read_file"));
    }

    #[test]
    fn consult_tool_embeds_specialist_roster() {
        // Both the description and the `enum` of the `name` parameter
        // must mention each specialist by name so the model can pick.
        let runners = vec![
            dummy_runner("coder", "use when writing code"),
            dummy_runner("reviewer", "use when reviewing"),
        ];
        let defs = tool_definitions_with(&runners, None);
        let consult = defs
            .iter()
            .find(|t| t.function.name == "consult_specialist")
            .expect("consult tool registered");

        assert!(consult.function.description.contains("coder"));
        assert!(consult.function.description.contains("reviewer"));
        assert!(consult
            .function
            .description
            .contains("use when writing code"));

        let name_enum = consult
            .function
            .parameters
            .get("properties")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.get("enum"))
            .and_then(|e| e.as_array())
            .expect("enum array on name param");
        let names: Vec<&str> = name_enum.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(names, vec!["coder", "reviewer"]);
    }

    #[test]
    fn full_tool_surface_includes_new_primitives() {
        // Regression guard for the split: every tool listed in the
        // README should still show up after the refactor.
        let defs = tool_definitions();
        for name in [
            "read_file",
            "list_dir",
            "find_files",
            "git_status",
            "git_log",
            "git_diff",
            "git_show",
            "edit_file",
            "multi_edit",
            "apply_patch",
            "move_lines",
            "delete_lines",
            "insert_at",
            "write_file",
            "run_command",
            "save_memory",
            "read_memory",
            "forget_memory",
        ] {
            assert!(has_tool(&defs, name), "missing tool: {name}");
        }
    }
}
