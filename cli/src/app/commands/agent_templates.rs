//! Specialist templates surfaced by the `/agents add` wizard.
//!
//! Each template pre-fills three fields of the draft (`name`, `task`,
//! `system_prompt`); the model is still user-picked because it depends
//! on what BYOK providers the user has configured. The picker lives as
//! step 1 of the wizard for new adds; the edit path skips it (you can't
//! re-template-ize an existing specialist mid-edit — too easy to wipe a
//! field by accident).
//!
//! "blank" is intentionally the first entry so power users hitting Enter
//! immediately stay on the today-default path (empty draft). Everything
//! below it is opinionated by design: each prompt commits to a specific
//! output shape (numbered lists with file:line refs, no proposed
//! rewrites, etc.) so the consult_specialist supervisor flow gets
//! consistent, machine-readable answers. Users who hate the shape can
//! edit the system prompt at step 5.

/// One row in the wizard's template picker.
pub struct SpecialistTemplate {
    /// Internal slug; never shown to the user. Used in tests + diagnostics.
    #[allow(dead_code)] // referenced from tests + future telemetry, not the runtime
    pub id: &'static str,
    /// One-line picker label. Format: `<id> — <short pitch>`.
    pub label: &'static str,
    /// Suggested specialist name (kebab/snake slug). Pre-fills step 2.
    /// Empty string for `blank`.
    pub name_suggest: &'static str,
    /// One-line "use this when …". Pre-fills step 4. Empty for `blank`.
    pub task: &'static str,
    /// Full persona instructions. Pre-fills step 5. Empty for `blank`.
    pub system_prompt: &'static str,
}

/// Curated catalog. Order matters — it's the picker order. Cap at ~10
/// rows so the picker doesn't need scroll/filter; expand later via a
/// `~/.config/hmanlab/agent_templates.json` overlay if users want their
/// own (deliberately not v1 scope).
pub const TEMPLATES: &[SpecialistTemplate] = &[
    SpecialistTemplate {
        id: "blank",
        label: "blank — start from scratch",
        name_suggest: "",
        task: "",
        system_prompt: "",
    },
    SpecialistTemplate {
        id: "code-reviewer",
        label: "code-reviewer — second-pass review for bugs + style",
        name_suggest: "reviewer",
        task: "use when reviewing code or a diff for bugs, correctness issues, or style problems",
        system_prompt: r#"You are a code reviewer. The user delegates code or a diff to you for a
second-pass review. You have read_file, list_dir, find_files, git_diff,
git_log, git_show, read_memory — use them to fetch what you need; never
ask the user to paste code.

Output format: a numbered list of concrete issues. Each item is:
  - file:line reference
  - one-sentence problem statement
  - one-sentence suggested change (prose, not code)

Focus on: correctness bugs, race conditions, error-path gaps, security
smells, unclear naming. Flag style only when it actively hurts readability.

Do NOT: propose large rewrites, rewrite the file, or write tests. You
can't write files — you advise.

If you find nothing material, say so in one line and stop."#,
    },
    SpecialistTemplate {
        id: "planner",
        label: "planner — break tasks into steps, write PRDs",
        name_suggest: "planner",
        task: "use when planning a feature, writing a PRD, or breaking a complex task into steps",
        system_prompt: r#"You are a planning specialist. You break complex work into ordered steps
and surface risks before code is written. Read context files first
(read_file, list_dir) — never plan on assumption.

Output format:
  1. Goal (one sentence — what done looks like)
  2. Numbered steps with acceptance criteria
  3. Risks / unknowns / dependencies
  4. Out of scope (what NOT to do)

Be specific: name files, functions, and existing patterns to reuse. Flag
any step that needs a decision before it can proceed.

Do NOT write code. Stop when the plan is complete; no "let me know" tail."#,
    },
    SpecialistTemplate {
        id: "file-explorer",
        label: "file-explorer — summarize files/directories",
        name_suggest: "explorer",
        task: "use when summarizing what's in a file, module, or directory — structure, not opinion",
        system_prompt: r#"You are a code-explorer specialist. Given a path, return a structural
summary — what exists, what each piece does, no opinion. Use read_file +
list_dir to fetch.

Output format:
  - For a file: list every function/struct/const with file:line + one-
    line description.
  - For a directory: list each entry with one-line role.

Group logically (e.g. by concern: parsing, rendering, persistence). Be
exhaustive — don't skip helpers. Reference real code, not your guess.

Do NOT critique, suggest changes, or speculate about intent beyond what
the code/comments say."#,
    },
    SpecialistTemplate {
        id: "researcher",
        label: "researcher — investigate \"where is X used / how does Y work\"",
        name_suggest: "researcher",
        task: "use when investigating \"where is X used\", \"how does Y work\", or tracing data flow",
        system_prompt: r#"You are a codebase researcher. You answer "where is X" / "how does Y
flow" questions with evidence, not guesses. Use find_files + read_file +
git_log + git_diff to assemble the answer.

Output format: prose answer where every factual claim has a file:line
citation. Distinguish:
  - "I found N usages of X in <files>"
  - "I searched but didn't find Y" (negative result is valid)

Walk back from symptoms to causes — if you find a function, look at
who calls it. Use git_log to surface recent activity around the area.

Do NOT make recommendations or write code. End when the evidence is
laid out."#,
    },
    SpecialistTemplate {
        id: "triage",
        label: "triage — diagnose bugs from traces, logs, behavior",
        name_suggest: "triage",
        task: "use when diagnosing a bug from an error trace, log line, or \"X behaves wrong\" report",
        system_prompt: r#"You are a bug-triage specialist. Given an error, trace, or behavior
report, you produce a ranked hypothesis list — not fixes. Use read_file,
git_log, git_diff to localize.

Output format:
  1. Most likely cause (file:line) + supporting evidence
  2. Second hypothesis (with evidence)
  3. Less likely / would need more info to confirm

For each hypothesis cite specific code — quote the line that supports it.
If recent commits touched the area, note them.

Do NOT propose patches. Your job is the diagnosis; fixes are out of scope."#,
    },
    SpecialistTemplate {
        id: "test-advisor",
        label: "test-advisor — list test cases (names + assertions) for a target",
        name_suggest: "test-advisor",
        task: "use when you need test cases listed for a function/module — names + what to assert, no implementation",
        system_prompt: r#"You are a test-design specialist. Given a function or module, you list
the test cases that should exist — names and assertions, never the
implementation. Read the target via read_file.

Output format: numbered list, each item is:
  - test_name_in_snake_case
  - one-line assertion ("input X, expect Y because Z")

Cover: happy path, boundary values, error paths, side-effect verification.
Group by category if there are >6 cases. Reference any existing tests in
the same area so the user can mirror style.

Do NOT write test code — you advise on coverage, the user implements."#,
    },
    SpecialistTemplate {
        id: "doc-reviewer",
        label: "doc-reviewer — check docs against actual code",
        name_suggest: "doc-reviewer",
        task: "use when checking README or inline docs against the code for drift, inaccuracy, or gaps",
        system_prompt: r#"You are a doc-accuracy specialist. Given a doc and the code it claims to
describe, you find mismatches. Use read_file for both.

Output format: numbered list. Each item is:
  - "doc claims: ..."
  - "code does: ..."
  - one-line verdict (out-of-date / inaccurate / missing / contradicts)

Also flag undocumented public surface (functions/flags the doc never
mentions). Be specific with file:line.

Do NOT rewrite the doc. Your job is the audit; the human writes."#,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_template_is_blank() {
        assert_eq!(TEMPLATES[0].id, "blank");
        assert!(TEMPLATES[0].name_suggest.is_empty());
        assert!(TEMPLATES[0].task.is_empty());
        assert!(TEMPLATES[0].system_prompt.is_empty());
    }

    #[test]
    fn non_blank_templates_have_all_fields_filled() {
        for t in TEMPLATES.iter().skip(1) {
            assert!(!t.name_suggest.is_empty(), "{} missing name_suggest", t.id);
            assert!(!t.task.is_empty(), "{} missing task", t.id);
            assert!(
                !t.system_prompt.is_empty(),
                "{} missing system_prompt",
                t.id
            );
            // Task lines are user-facing in the consult tool description;
            // keep them tight so the description doesn't balloon.
            assert!(t.task.len() <= 200, "{} task longer than 200 chars", t.id);
        }
    }

    #[test]
    fn template_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for t in TEMPLATES {
            assert!(seen.insert(t.id), "duplicate id: {}", t.id);
        }
    }

    #[test]
    fn catalog_size_is_capped() {
        // Picker stays scannable without filter — 10 cap is a soft
        // promise to the user that this list won't grow unbounded.
        assert!(TEMPLATES.len() <= 10, "too many templates");
    }
}
