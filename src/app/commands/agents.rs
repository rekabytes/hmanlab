//! `/agents <sub>` + `/ask <name> <query>` — specialist agents (phase 1).
//!
//! Phase 1 is **manual invoke only**. There's no `consult_specialist` tool
//! yet (that's phase 2): the user routes by typing `/ask coder "…"`
//! directly. The plumbing here is what phase 2 will reuse when it adds
//! the tool — the per-specialist backend builder, the isolated history
//! shape, the per-agent token attribution.
//!
//! Specialists run with a **read-only tool surface** ([`ask_tool_defs`])
//! and a fresh history seeded with their own system prompt. They do not
//! see the main agent's chat history; each `/ask` is an isolated turn.

use tokio::sync::mpsc;

use super::super::{fresh_textarea, AgentsDraft, AgentsSetupStep, App, AppAction, Mode, StreamMsg};
use super::agent_templates::TEMPLATES;
use super::AgentsSub;
use crate::config::{SpecialistAgent, MAX_SPECIALISTS};
use crate::ollama::ChatMessage;

impl App {
    pub(in crate::app) fn handle_agents(
        &mut self,
        sub: AgentsSub,
        _tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        match sub {
            AgentsSub::Show => self.agents_show(),
            AgentsSub::SetEnabled(true) => self.agents_set_enabled(true),
            AgentsSub::SetEnabled(false) => self.agents_set_enabled(false),
            AgentsSub::List => self.agents_list(),
            AgentsSub::Add => self.open_agents_setup(None),
            AgentsSub::Remove(name) => self.agents_remove(&name),
            AgentsSub::Edit(name) => {
                if name.trim().is_empty() {
                    self.push_info("Usage: /agents edit <name>".into());
                    return;
                }
                if self.agents.by_name_mut(&name).is_none() {
                    self.push_info(format!("No specialist named '{name}'."));
                    return;
                }
                self.open_agents_setup(Some(name));
            }
            AgentsSub::SetSpecialistEnabled { name, enabled } => {
                self.agents_set_specialist_enabled(&name, enabled)
            }
            AgentsSub::Unknown(other) => {
                let suggestion = suggest_agents_sub(&other);
                let hint = suggestion
                    .map(|s| format!(" — did you mean `/agents {s}`?"))
                    .unwrap_or_default();
                self.push_info(format!(
                    "Unknown subcommand `/agents {other}`{hint}\n\
                     Try /agents (no arg) for status, or one of: \
                     on, off, list, add, remove, edit, enable-agent, disable-agent."
                ));
            }
        }
    }

    fn agents_show(&mut self) {
        let session = if self.agents_session_enabled {
            "ON"
        } else {
            "OFF"
        };
        let n = self.agents.specialists.len();
        let max = MAX_SPECIALISTS;
        let mut body = format!(
            "Specialist agents — {n}/{max} configured · session: {session}\n\
             /agents add                add a specialist\n\
             /agents on  | off          flip session activation\n\
             /agents list               pretty-print roster\n\
             /agents edit <name>        re-open the wizard\n\
             /agents remove <name>      drop a specialist\n\
             /agents enable-agent <n>   enable a single specialist\n\
             /agents disable-agent <n>  disable a single specialist\n\
             /ask <name> <query>        manually invoke a specialist\n"
        );
        if n > 0 {
            body.push('\n');
            body.push_str(&format_roster(&self.agents.specialists));
        }
        self.push_info(body);
    }

    fn agents_list(&mut self) {
        if self.agents.specialists.is_empty() {
            self.push_info("No specialists configured yet. Use /agents add to set one up.".into());
            return;
        }
        self.push_info(format_roster(&self.agents.specialists));
    }

    fn agents_set_enabled(&mut self, want: bool) {
        if want && self.agents.specialists.is_empty() {
            self.push_info(
                "No specialists configured. Run /agents add first, then /agents on.".into(),
            );
            return;
        }
        if self.agents_session_enabled == want {
            self.push_info(format!(
                "Specialist session is already {}.",
                if want { "ON" } else { "OFF" }
            ));
            return;
        }
        self.agents_session_enabled = want;
        if want {
            self.push_info(format!(
                "✓ Specialist session enabled for this run. Use /ask <name> <query>.\n\
                 (Resets on TUI restart — this is per-session by design.)\n\n{}",
                format_roster(&self.agents.specialists)
            ));
        } else {
            self.push_info("✓ Specialist session disabled.".into());
        }
    }

    fn agents_remove(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.push_info("Usage: /agents remove <name>".into());
            return;
        }
        let before = self.agents.specialists.len();
        self.agents
            .specialists
            .retain(|s| !s.name.eq_ignore_ascii_case(name));
        if self.agents.specialists.len() == before {
            self.push_info(format!("No specialist named '{name}'."));
            return;
        }
        self.persist_config();
        self.push_info(format!("✓ Removed specialist '{name}'."));
    }

    fn agents_set_specialist_enabled(&mut self, name: &str, want: bool) {
        let name = name.trim();
        if name.is_empty() {
            self.push_info(format!(
                "Usage: /agents {}-agent <name>",
                if want { "enable" } else { "disable" }
            ));
            return;
        }
        let Some(s) = self.agents.by_name_mut(name) else {
            self.push_info(format!("No specialist named '{name}'."));
            return;
        };
        if s.enabled == want {
            let state = if want { "enabled" } else { "disabled" };
            self.push_info(format!("'{name}' is already {state}."));
            return;
        }
        s.enabled = want;
        let label = s.name.clone();
        self.persist_config();
        self.push_info(format!(
            "✓ Specialist '{label}' is now {}.",
            if want { "enabled" } else { "disabled" }
        ));
    }

    // --- /ask <name> <query> — manual invoke ---

    pub(in crate::app) fn handle_ask(
        &mut self,
        name: String,
        query: String,
        tx: &mpsc::UnboundedSender<StreamMsg>,
    ) {
        let name = name.trim().to_string();
        let query = query.trim().to_string();
        if name.is_empty() {
            self.push_info(
                "Usage: /ask <name> <query>   (configure specialists with /agents add)".into(),
            );
            return;
        }
        if query.is_empty() {
            self.push_info(format!("Usage: /ask {name} <query>"));
            return;
        }
        if !self.agents_session_enabled {
            self.push_info(
                "Specialist session is OFF. Run /agents on first (per-session opt-in).".into(),
            );
            return;
        }
        let Some(spec) = self.agents.enabled_by_name(&name) else {
            self.push_info(format!(
                "No enabled specialist named '{name}'. /agents list to see the roster."
            ));
            return;
        };
        // Build the specialist's backend BEFORE touching App state so a
        // missing API key fails clean (no half-flipped flags).
        let Some(backend) = self.make_backend_for(spec.provider.as_deref()) else {
            self.push_info(format!(
                "Specialist '{name}' uses provider '{}' but no API key is configured.",
                spec.provider.as_deref().unwrap_or("ollama")
            ));
            return;
        };
        if self.turn.is_busy() {
            self.push_info(
                "Wait for the current turn to finish before invoking a specialist.".into(),
            );
            return;
        }

        // Persist the visible turn before kicking off the agent loop so
        // the local user sees "[/ask coder] <query>" as a normal user
        // message and the assistant placeholder lights up below it.
        let user_label = format!("[/ask {name}] {query}");
        self.messages.push(ChatMessage {
            role: "user".into(),
            content: user_label,
            ..Default::default()
        });
        self.messages.push(ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            ..Default::default()
        });
        self.follow = true;
        self.status = format!("[/ask {name}] running on {}…", spec.model);
        self.active_specialist = Some(spec.name.clone());

        // Build the isolated history. First message = the specialist's
        // system prompt; second = the user query. No main-agent chat
        // crosses the boundary.
        let history = vec![
            ChatMessage {
                role: "system".into(),
                content: spec.system_prompt.clone(),
                ..Default::default()
            },
            ChatMessage {
                role: "user".into(),
                content: query,
                ..Default::default()
            },
        ];

        let model = spec.model.clone();
        let workspace = self.workspace.clone();
        let tool_defs = ask_tool_defs();
        let tx = tx.clone();
        let handle = tokio::spawn(async move {
            // /ask is itself a specialist invocation — pass an empty
            // runners vec so the invoked specialist can't recursively
            // consult further. Mirrors the consult_specialist nested-call
            // rule.
            crate::agent::agent_loop_with(
                backend,
                model,
                history,
                workspace,
                tx,
                tool_defs,
                Vec::new(),
            )
            .await;
        });
        self.turn = crate::app::TurnState::Generating { task: handle };
    }

    // --- Wizard ---

    pub(in crate::app) fn open_agents_setup(&mut self, editing: Option<String>) {
        if editing.is_none() && self.agents.specialists.len() >= MAX_SPECIALISTS {
            self.push_info(format!(
                "Roster is full ({MAX_SPECIALISTS} specialists max). Remove one with \
                 /agents remove <name> before adding another."
            ));
            return;
        }
        // Seed the draft from the existing spec when editing.
        self.agents_setup_draft = editing
            .as_deref()
            .and_then(|n| self.agents.by_name_mut(n))
            .map(|s| AgentsDraft {
                name: s.name.clone(),
                model: s.model.clone(),
                model_provider: s.provider.clone(),
                task: s.task.clone(),
                system_prompt: s.system_prompt.clone(),
            })
            .unwrap_or_default();
        let is_add = editing.is_none();
        self.agents_setup_editing = editing;
        self.agents_setup_picker_index = 0;
        self.agents_setup_error = None;
        self.agents_setup_input = fresh_textarea();
        self.mode = Mode::AgentsSetup;
        if is_add {
            // Adds start at the template picker. Picking a template
            // pre-fills the draft and advances to Name; "blank" leaves
            // the draft empty (today's behavior).
            self.agents_setup_step = AgentsSetupStep::Template;
            self.status =
                "Specialist setup — step 1 of 5: pick a template  ·  ↑↓ Enter  ·  Esc".into();
        } else {
            // Edit flow skips Template — re-templating an existing
            // specialist would wipe customisations on accident.
            self.agents_setup_step = AgentsSetupStep::Name;
            self.agents_setup_input
                .insert_str(&self.agents_setup_draft.name);
            self.agents_setup_input
                .set_placeholder_text("Short slug — letters, digits, _, - (3-30 chars)");
            self.status = "Specialist setup — step 1 of 4: name  ·  Esc to cancel".into();
        }
    }

    /// Wizard input handler. Esc cancels; Enter advances or commits.
    pub(in crate::app) fn handle_agents_setup_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        _tx: &mpsc::UnboundedSender<StreamMsg>,
    ) -> AppAction {
        use crossterm::event::{KeyCode, KeyModifiers};
        if key.code == KeyCode::Esc {
            self.mode = Mode::Chat;
            self.agents_setup_error = None;
            self.status = "Specialist setup cancelled".into();
            return AppAction::Continue;
        }

        // Picker steps have their own keymap (up/down/Enter), so dispatch
        // before falling through to the textarea. Both Template (step 1
        // for adds) and Model (step 3) are pickers.
        if matches!(
            self.agents_setup_step,
            AgentsSetupStep::Template | AgentsSetupStep::Model
        ) {
            return self.agents_setup_picker_key(key);
        }

        if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
            return self.agents_setup_advance();
        }

        let input: tui_textarea::Input = key.into();
        self.agents_setup_input.input(input);
        self.agents_setup_error = None;
        AppAction::Continue
    }

    fn agents_setup_picker_key(&mut self, key: crossterm::event::KeyEvent) -> AppAction {
        use crossterm::event::KeyCode;
        // Length depends on which picker is active — Template picker
        // walks TEMPLATES, Model picker walks live model candidates.
        let len = match self.agents_setup_step {
            AgentsSetupStep::Template => TEMPLATES.len(),
            AgentsSetupStep::Model => self.specialist_model_candidates().len(),
            // Other steps don't use the picker; safe upper bound = 0.
            _ => 0,
        };
        match key.code {
            KeyCode::Up | KeyCode::Char('k') if self.agents_setup_picker_index > 0 => {
                self.agents_setup_picker_index -= 1;
            }
            KeyCode::Down | KeyCode::Char('j') if self.agents_setup_picker_index + 1 < len => {
                self.agents_setup_picker_index += 1;
            }
            KeyCode::Enter => match self.agents_setup_step {
                AgentsSetupStep::Template => self.apply_template_and_advance(),
                AgentsSetupStep::Model => self.apply_model_and_advance(),
                _ => {}
            },
            _ => {}
        }
        AppAction::Continue
    }

    /// Step 1 (add only) — copy template fields onto the draft, then
    /// advance to Name with the suggested slug pre-typed. "blank" copies
    /// nothing so the draft stays empty.
    fn apply_template_and_advance(&mut self) {
        let Some(tpl) = TEMPLATES.get(self.agents_setup_picker_index) else {
            self.agents_setup_error = Some("No template selected.".into());
            return;
        };
        // Empty strings on the "blank" template mean "leave the draft as
        // already-defaulted" — perfect for the from-scratch path.
        if !tpl.name_suggest.is_empty() {
            self.agents_setup_draft.name = tpl.name_suggest.to_string();
        }
        if !tpl.task.is_empty() {
            self.agents_setup_draft.task = tpl.task.to_string();
        }
        if !tpl.system_prompt.is_empty() {
            self.agents_setup_draft.system_prompt = tpl.system_prompt.to_string();
        }
        self.agents_setup_step = AgentsSetupStep::Name;
        self.agents_setup_input = fresh_textarea();
        self.agents_setup_input
            .insert_str(&self.agents_setup_draft.name);
        self.agents_setup_input
            .set_placeholder_text("Short slug — letters, digits, _, - (3-30 chars)");
        self.status =
            "Specialist setup — step 2 of 5: name (pre-filled, editable)  ·  Esc to cancel".into();
        self.agents_setup_error = None;
    }

    /// Step 3 — record the picked model on the draft and advance to Task.
    /// Extracted from the old picker handler so the dispatch in
    /// `agents_setup_picker_key` stays a clean match.
    fn apply_model_and_advance(&mut self) {
        let candidates = self.specialist_model_candidates();
        let Some(picked) = candidates.get(self.agents_setup_picker_index).cloned() else {
            self.agents_setup_error =
                Some("No models available. Add one with /model or /host first.".into());
            return;
        };
        self.agents_setup_draft.model = picked.model;
        self.agents_setup_draft.model_provider = picked.provider;
        self.agents_setup_step = AgentsSetupStep::Task;
        self.agents_setup_input = fresh_textarea();
        self.agents_setup_input
            .insert_str(&self.agents_setup_draft.task);
        self.agents_setup_input.set_placeholder_text(
            "One line — \"use this when …\" (shown to the user + main agent)",
        );
        let (n, total) = self.agents_setup_step_counter();
        self.status =
            format!("Specialist setup — step {n} of {total}: task description  ·  Esc to cancel");
        self.agents_setup_error = None;
    }

    /// Step counter `(current_index, total_steps)`. Adds run a 5-step
    /// flow (Template → Name → Model → Task → Prompt); edits run a 4-step
    /// flow (Template skipped). The popup title and status line both
    /// pull from this so they stay in sync.
    pub fn agents_setup_step_counter(&self) -> (u8, u8) {
        let is_edit = self.agents_setup_editing.is_some();
        let total = if is_edit { 4 } else { 5 };
        // Step ordering is fixed by the enum:
        //   add: Template=1, Name=2, Model=3, Task=4, Prompt=5
        //   edit:            Name=1, Model=2, Task=3, Prompt=4
        let n = match (self.agents_setup_step, is_edit) {
            (AgentsSetupStep::Template, _) => 1,
            (AgentsSetupStep::Name, false) => 2,
            (AgentsSetupStep::Name, true) => 1,
            (AgentsSetupStep::Model, false) => 3,
            (AgentsSetupStep::Model, true) => 2,
            (AgentsSetupStep::Task, false) => 4,
            (AgentsSetupStep::Task, true) => 3,
            (AgentsSetupStep::Prompt, false) => 5,
            (AgentsSetupStep::Prompt, true) => 4,
        };
        (n, total)
    }

    fn agents_setup_advance(&mut self) -> AppAction {
        let value = self
            .agents_setup_input
            .lines()
            .join("\n")
            .trim()
            .to_string();
        match self.agents_setup_step {
            AgentsSetupStep::Template => {
                // Picker handles Enter via apply_template_and_advance —
                // this arm is defensive against future keymap changes.
            }
            AgentsSetupStep::Name => {
                if let Err(msg) =
                    validate_name(&value, &self.agents, self.agents_setup_editing.as_deref())
                {
                    self.agents_setup_error = Some(msg);
                    return AppAction::Continue;
                }
                self.agents_setup_draft.name = value;
                self.agents_setup_step = AgentsSetupStep::Model;
                self.agents_setup_picker_index = self
                    .specialist_model_candidates()
                    .iter()
                    .position(|m| {
                        m.model == self.agents_setup_draft.model
                            && m.provider == self.agents_setup_draft.model_provider
                    })
                    .unwrap_or(0);
                self.agents_setup_error = None;
                let (n, total) = self.agents_setup_step_counter();
                self.status = format!(
                    "Specialist setup — step {n} of {total}: pick a model  ·  ↑↓ Enter  ·  Esc"
                );
            }
            AgentsSetupStep::Model => {
                // Handled by agents_setup_picker_key; should be unreachable.
            }
            AgentsSetupStep::Task => {
                if value.is_empty() {
                    self.agents_setup_error = Some("Task description can't be empty.".into());
                    return AppAction::Continue;
                }
                if value.len() > 200 {
                    self.agents_setup_error =
                        Some("Task description should fit on one line (≤ 200 chars).".into());
                    return AppAction::Continue;
                }
                self.agents_setup_draft.task = value;
                self.agents_setup_step = AgentsSetupStep::Prompt;
                self.agents_setup_input = fresh_textarea();
                self.agents_setup_input
                    .insert_str(&self.agents_setup_draft.system_prompt);
                self.agents_setup_input.set_placeholder_text(
                    "Full system prompt — persona, rules, constraints. Multi-line (Alt+Enter).",
                );
                let (n, total) = self.agents_setup_step_counter();
                self.status = format!(
                    "Specialist setup — step {n} of {total}: system prompt  ·  Esc to cancel"
                );
                self.agents_setup_error = None;
            }
            AgentsSetupStep::Prompt => {
                if value.is_empty() {
                    self.agents_setup_error = Some("System prompt can't be empty.".into());
                    return AppAction::Continue;
                }
                self.agents_setup_draft.system_prompt = value;
                self.commit_specialist();
            }
        }
        AppAction::Continue
    }

    fn commit_specialist(&mut self) {
        let draft = self.agents_setup_draft.clone();
        let new_spec = SpecialistAgent {
            name: draft.name.clone(),
            model: draft.model,
            provider: draft.model_provider,
            task: draft.task,
            system_prompt: draft.system_prompt,
            enabled: true,
        };
        let editing = self.agents_setup_editing.take();
        let action_label;
        if let Some(orig) = editing {
            if let Some(slot) = self
                .agents
                .specialists
                .iter_mut()
                .find(|s| s.name.eq_ignore_ascii_case(&orig))
            {
                *slot = new_spec.clone();
                action_label = format!("Updated specialist '{}'", new_spec.name);
            } else {
                // Original was removed under our feet — fall back to add.
                self.agents.specialists.push(new_spec.clone());
                action_label = format!("Added specialist '{}'", new_spec.name);
            }
        } else {
            self.agents.specialists.push(new_spec.clone());
            action_label = format!("Added specialist '{}'", new_spec.name);
        }
        self.persist_config();
        self.mode = Mode::Chat;
        self.status = action_label.clone();
        self.push_info(format!(
            "✓ {action_label}.\nModel: {}{}\nTask: {}\n\
             (Use /agents on to enable this session, then /ask {} <query>.)",
            new_spec.model,
            match &new_spec.provider {
                Some(p) => format!(" [{p}]"),
                None => "".into(),
            },
            new_spec.task,
            new_spec.name
        ));
    }

    /// Picker rows for the Template step — just the user-visible
    /// labels. The renderer doesn't need the full struct (system_prompt
    /// lands on the textarea later via `apply_template_and_advance`).
    /// 8 ptr-sized copies per render; trivial.
    pub fn specialist_template_labels(&self) -> Vec<&'static str> {
        TEMPLATES.iter().map(|t| t.label).collect()
    }

    /// Live list of pickable models for the wizard (Ollama + every BYOK
    /// extra). Rebuilt every render — picker length can change while
    /// the wizard is open (rare, but cheap to be honest about state).
    pub fn specialist_model_candidates(&self) -> Vec<SpecialistModelCandidate> {
        let mut out: Vec<SpecialistModelCandidate> = self
            .models
            .iter()
            .map(|m| SpecialistModelCandidate {
                model: m.clone(),
                provider: None,
            })
            .collect();
        for em in &self.extra_models {
            out.push(SpecialistModelCandidate {
                model: em.name.clone(),
                provider: Some(em.provider.clone()),
            });
        }
        out
    }
}

/// One row in the specialist-model picker (wizard step 2).
#[derive(Clone, Debug)]
pub struct SpecialistModelCandidate {
    pub model: String,
    /// `None` = local Ollama; `Some(provider_id)` = BYOK extra.
    pub provider: Option<String>,
}

impl SpecialistModelCandidate {
    pub fn display(&self) -> String {
        match &self.provider {
            Some(p) => format!("[{p}] {}", self.model),
            None => self.model.clone(),
        }
    }
}

// Read-only tool surface for specialists lives in `crate::tools`
// (see `ask_specialist_tool_defs`) so the consult flow and the `/ask`
// flow share one canonical READONLY list. Re-exported here under the
// historical name so `/ask` callers don't have to learn a new path.
pub use crate::tools::ask_specialist_tool_defs as ask_tool_defs;

/// Suggest the nearest valid `/agents <sub>` for an unknown input.
/// Returns `None` if nothing is within edit distance 2 — the catch-all
/// info line then just lists the available subcommands without
/// claiming a guess.
fn suggest_agents_sub(input: &str) -> Option<&'static str> {
    const KNOWN: &[&str] = &[
        "on",
        "off",
        "list",
        "add",
        "remove",
        "edit",
        "enable-agent",
        "disable-agent",
        "show",
        "help",
    ];
    let lo = input.to_ascii_lowercase();
    KNOWN
        .iter()
        .map(|k| (k, levenshtein(&lo, k)))
        .filter(|(_, d)| *d <= 2)
        .min_by_key(|(_, d)| *d)
        .map(|(k, _)| *k)
}

/// Iterative Levenshtein, two-row tabulation. Tiny inputs (subcommand
/// names <16 chars) so the perf doesn't matter — clarity wins. Pulled
/// in here instead of taking a crate dep for a 25-line function.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            curr[j] = (curr[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

fn validate_name(
    name: &str,
    existing: &crate::config::AgentsConfig,
    editing: Option<&str>,
) -> Result<(), String> {
    let n = name.trim();
    if n.len() < 3 || n.len() > 30 {
        return Err("Name must be 3-30 characters.".into());
    }
    if !n
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("Name may only contain letters, digits, _, or -.".into());
    }
    let clash = existing.specialists.iter().any(|s| {
        s.name.eq_ignore_ascii_case(n)
            && editing.map(|e| !e.eq_ignore_ascii_case(n)).unwrap_or(true)
    });
    if clash {
        return Err(format!("Specialist '{n}' already exists."));
    }
    Ok(())
}

fn format_roster(specialists: &[SpecialistAgent]) -> String {
    let mut lines = vec!["Roster:".to_string()];
    for s in specialists {
        let provider = s.provider.clone().unwrap_or_else(|| "ollama".into());
        let mark = if s.enabled { "●" } else { "○" };
        lines.push(format!(
            "  {mark} {name:<14} [{provider}] {model}  ·  {task}",
            name = s.name,
            provider = provider,
            model = s.model,
            task = s.task,
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod suggest_tests {
    use super::*;

    #[test]
    fn suggests_close_match() {
        assert_eq!(suggest_agents_sub("lst"), Some("list"));
        assert_eq!(suggest_agents_sub("addd"), Some("add"));
        assert_eq!(suggest_agents_sub("remov"), Some("remove"));
    }

    #[test]
    fn returns_none_for_garbage() {
        assert_eq!(suggest_agents_sub("zzzzzzzz"), None);
    }

    #[test]
    fn exact_match_returns_self() {
        assert_eq!(suggest_agents_sub("list"), Some("list"));
    }
}
