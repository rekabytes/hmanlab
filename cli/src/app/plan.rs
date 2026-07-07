//! Parsing the agent's plan / todo list out of its streamed markdown.
//!
//! The model emits a checklist using `- [ ]` / `- [x]` / `- [-]` lines.
//! We scan the *most recent* assistant message for the first contiguous
//! run of checklist lines and lift them into `Vec<PlanItem>` for the
//! inspector pane.
//!
//! Parsing rules:
//!   `- [ ] foo`  → Pending
//!   `- [-] foo`  → Active   (opencode / claude-code convention)
//!   `- [x] foo`  → Done
//!   `- [X] foo`  → Done     (case-insensitive on the x)
//! Lines that don't match the pattern end the run — the plan ends at
//! the first non-checklist line, so prose before/after doesn't get
//! pulled in by mistake.

use crate::app::state::{PlanItem, PlanState};

/// Extract a plan from a blob of assistant text. Returns an empty Vec
/// if no checklist run is found. A run is 2+ consecutive checklist
/// lines (one line alone is probably a typo, not a plan).
pub fn parse_plan(text: &str) -> Vec<PlanItem> {
    let mut plan = Vec::new();
    let mut in_run = false;
    for raw in text.lines() {
        let line = raw.trim_start();
        if let Some((state, body)) = parse_checklist_line(line) {
            plan.push(PlanItem {
                text: body.to_string(),
                state,
            });
            in_run = true;
        } else if in_run && line.is_empty() {
            // Blank lines inside a run are tolerated — the model often
            // wraps plans in blank-line-separated blocks. Don't break
            // the run on a blank; just skip.
            continue;
        } else if in_run {
            // Non-checklist, non-blank line ends the run.
            break;
        }
        // else: not in a run yet, keep scanning.
    }
    plan
}

/// Parse a single line. Returns `Some((state, text))` if the line
/// starts with `- [ ]`, `- [x]`, or `- [-]`.
fn parse_checklist_line(line: &str) -> Option<(PlanState, &str)> {
    let after_dash = line.strip_prefix('-')?.strip_prefix(' ')?;
    let (marker, body) = after_dash.split_once(']')?;
    if !marker.starts_with('[') || marker.len() != 2 {
        return None;
    }
    let state = match marker.as_bytes()[1] {
        b' ' | b'\t' => PlanState::Pending,
        b'x' | b'X' => PlanState::Done,
        b'-' => PlanState::Active,
        _ => return None,
    };
    let body = body.strip_prefix(' ').unwrap_or(body);
    Some((state, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_states() {
        let text = "- [ ] one\n- [-] two\n- [x] three\n";
        assert_eq!(
            parse_plan(text),
            vec![
                PlanItem {
                    text: "one".into(),
                    state: PlanState::Pending
                },
                PlanItem {
                    text: "two".into(),
                    state: PlanState::Active
                },
                PlanItem {
                    text: "three".into(),
                    state: PlanState::Done
                },
            ]
        );
    }

    #[test]
    fn empty_text_yields_empty_plan() {
        assert!(parse_plan("").is_empty());
        assert!(parse_plan("just some prose\nno checklists here").is_empty());
    }

    #[test]
    fn single_lonely_checklist_is_a_plan() {
        // Even a one-item list parses as a plan — the inspector pane
        // handles the empty case distinctly, so we don't gate here.
        let text = "Here is what I'll do:\n\n- [ ] just this\n\nThat's it.";
        let p = parse_plan(text);
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].text, "just this");
    }

    #[test]
    fn plan_stops_at_non_checklist_line() {
        let text = "- [ ] a\n- [ ] b\nThen I stopped.\n- [ ] c\n";
        let p = parse_plan(text);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].text, "a");
        assert_eq!(p[1].text, "b");
    }
}
