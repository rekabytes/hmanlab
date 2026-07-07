//! `render_agents_setup` — `/agents add | edit` wizard.
//!
//! Step layout:
//!   - Adds: template → name → model → task → prompt (5 steps)
//!   - Edits: name → model → task → prompt (4 steps; Template is skipped)
//!
//! Picker steps (Template, Model) render a scrollable list; the other
//! three render the textarea. Step counters come from
//! `App::agents_setup_step_counter` so add/edit modes stay in sync.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Padding, Paragraph},
    Frame,
};

use crate::app::{AgentsSetupStep, App};

use super::super::markdown::{parse_inline_md, wrap_styled_segments};
use super::super::theme;

pub(in crate::ui) fn render_agents_setup(f: &mut Frame, area: Rect, app: &mut App) {
    f.render_widget(Clear, area);

    let title_verb = if app.agents_setup_editing.is_some() {
        "edit"
    } else {
        "add"
    };
    let (step_n, step_total) = app.agents_setup_step_counter();

    let (step_label, body, hint, input_title) = match app.agents_setup_step {
        AgentsSetupStep::Template => (
            "template",
            "Start from a **canned template** or pick `blank` to fill every \
             field by hand. Each template pre-fills the name, task, and system \
             prompt — you can still edit any of them on the next steps.\n\n\
             Templates are opinionated by design: they commit to specific output \
             shapes (numbered lists with `file:line` refs, no proposed rewrites, \
             etc.) so the supervisor flow gets consistent answers."
                .to_string(),
            "↑↓ navigate  ·  Enter to pick  ·  Esc to cancel",
            " ❯ templates ",
        ),
        AgentsSetupStep::Name => (
            "name",
            "Pick a **short slug** for this specialist — letters, digits, `_`, `-` only \
             (3-30 chars). You'll invoke it later as `/ask <name> <query>`, and the \
             main agent will see it in the `consult_specialist` tool description.\n\n\
             Common picks: `coder`, `reviewer`, `planner`, `tester`."
                .to_string(),
            "Enter to advance  ·  Esc to cancel",
            " ❯ name ",
        ),
        AgentsSetupStep::Model => (
            "model",
            "Pick the model this specialist runs on. The list is your live Ollama \
             models + every BYOK extra you've configured.\n\n\
             Tip: pair specialists that complement each other — e.g. a fast cheap \
             planner with a careful reviewer."
                .to_string(),
            "↑↓ navigate  ·  Enter to pick  ·  Esc to cancel",
            " ❯ models ",
        ),
        AgentsSetupStep::Task => (
            "task",
            "One-line **\"use this when …\"** description (≤ 200 chars). Shown in \
             `/agents list` and fed into the consult tool description so the main \
             agent knows when to delegate.\n\n\
             Example: `use when reviewing security or correctness of just-written code`."
                .to_string(),
            "Enter to advance  ·  Esc to cancel",
            " ❯ task ",
        ),
        AgentsSetupStep::Prompt => (
            "system prompt",
            "Full **system prompt** — persona, rules, output format, constraints. \
             Multi-line allowed (Alt+Enter or Ctrl+J for newline). Specialists run \
             with read-only tools (read_file, list_dir, find_files, git_*, \
             read_memory) — no edits, no shell, no writes.\n\n\
             Tip: be explicit about what the specialist should NOT do."
                .to_string(),
            "Enter to save  ·  Alt+Enter for newline  ·  Esc to cancel",
            " ❯ prompt ",
        ),
    };

    let title = format!(" specialist {title_verb} · step {step_n} of {step_total}: {step_label} ");

    let block = theme::popup_block(title.trim(), false).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let content_width = (inner.width as usize).saturating_sub(2).max(10);

    // Picker steps reserve more vertical real estate; Prompt step uses
    // a tall textarea since system prompts run multi-paragraph.
    let input_height = match app.agents_setup_step {
        AgentsSetupStep::Template | AgentsSetupStep::Model => Constraint::Min(5),
        AgentsSetupStep::Prompt => Constraint::Length(8),
        _ => Constraint::Length(3),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(2),    // body
            input_height,          // input or picker
            Constraint::Length(1), // error (or blank)
            Constraint::Length(1), // hint
        ])
        .split(inner);

    f.render_widget(block, area);

    // Body paragraph — supports inline markdown.
    let mut lines: Vec<Line> = Vec::new();
    for paragraph in body.split('\n') {
        if paragraph.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        let segments = parse_inline_md(paragraph, Style::default());
        for spans in wrap_styled_segments(segments, content_width) {
            lines.push(Line::from(spans));
        }
    }
    f.render_widget(
        Paragraph::new(lines).style(Style::default().fg(theme::color::FG)),
        chunks[0],
    );

    // Input area — picker on Template + Model steps, textarea otherwise.
    let picker_block = || {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::color::ACCENT))
            .title(Span::styled(
                input_title,
                Style::default()
                    .fg(theme::color::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ))
            .padding(Padding::horizontal(1))
    };

    match app.agents_setup_step {
        AgentsSetupStep::Template => {
            let labels = app.specialist_template_labels();
            let items: Vec<ListItem> = labels
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let style = if i == app.agents_setup_picker_index {
                        Style::default()
                            .fg(ratatui::style::Color::Black)
                            .bg(theme::color::ACCENT_ALT)
                            .add_modifier(Modifier::BOLD)
                    } else if i == 0 {
                        // "blank" — render dim so the opinionated templates
                        // visually stand out as the recommended path.
                        Style::default().fg(theme::color::FG_DIM)
                    } else {
                        Style::default().fg(theme::color::FG)
                    };
                    ListItem::new(format!(" {label} ")).style(style)
                })
                .collect();
            f.render_widget(List::new(items).block(picker_block()), chunks[1]);
        }
        AgentsSetupStep::Model => {
            let candidates = app.specialist_model_candidates();
            if candidates.is_empty() {
                let empty = Paragraph::new(Line::from(Span::styled(
                    "(No models available — add an Ollama host with /host or a BYOK \
                     provider via /model first.)",
                    Style::default().fg(theme::color::FG_DIM),
                )))
                .block(picker_block());
                f.render_widget(empty, chunks[1]);
            } else {
                let items: Vec<ListItem> = candidates
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let style = if i == app.agents_setup_picker_index {
                            Style::default()
                                .fg(ratatui::style::Color::Black)
                                .bg(theme::color::ACCENT_ALT)
                                .add_modifier(Modifier::BOLD)
                        } else if c.provider.is_some() {
                            Style::default().fg(theme::color::ASSISTANT)
                        } else {
                            Style::default().fg(theme::color::FG)
                        };
                        ListItem::new(format!(" {} ", c.display())).style(style)
                    })
                    .collect();
                f.render_widget(List::new(items).block(picker_block()), chunks[1]);
            }
        }
        _ => {
            app.agents_setup_input.set_block(picker_block());
            f.render_widget(&app.agents_setup_input, chunks[1]);
        }
    }

    // Error row.
    if let Some(err) = &app.agents_setup_error {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("✗ {err}"),
                Style::default().fg(theme::color::ERROR),
            ))),
            chunks[2],
        );
    }

    // Hint footer.
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme::color::FG_DIM)),
        chunks[3],
    );
}
