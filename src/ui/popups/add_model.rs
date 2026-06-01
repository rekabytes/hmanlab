//! `render_add_model` — paste-your-API-key dialog. Opened by picking
//! one of the "+ Add … key" rows in the model picker. Provider-specific
//! copy explains where to generate the key and what becomes available
//! after saving.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Padding, Paragraph},
    Frame,
};

use crate::app::App;

use super::super::markdown::{parse_inline_md, wrap_styled_segments};
use super::super::theme;

pub(in crate::ui) fn render_add_model(f: &mut Frame, area: Rect, app: &mut App) {
    f.render_widget(Clear, area);

    // Single-step modal — pasting the API key is the whole flow.
    let (title, body, hint) = {
        let (title, body) = match app.add_model_provider.as_str() {
            p if p == crate::config::OLLAMA_CLOUD_PROVIDER => (
                " add Ollama Cloud key ".to_string(),
                "Paste your Ollama Cloud API key (generate one at \
                     https://ollama.com/settings/keys). After saving, the \
                     free-tier models (glm-4.7, gpt-oss:120b-cloud, \
                     qwen3-coder-next) become selectable in /model and you'll \
                     be switched to glm-4.7 by default.\n\n\
                     Note: glm-5.1, glm-5, deepseek, kimi, and minimax are \
                     subscription-only on Ollama Cloud — selecting them \
                     returns a 403 until you upgrade at ollama.com/upgrade.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to ollama.com — never to hmanlab-api."
                    .to_string(),
            ),
            p if p == crate::config::OPENROUTER_PROVIDER => (
                " add OpenRouter key ".to_string(),
                "Paste your OpenRouter API key (generate one at \
                     https://openrouter.ai/settings/keys). OpenRouter is a \
                     meta-provider that routes to OpenAI, Anthropic, Google, \
                     Meta, DeepSeek, Qwen and many others behind one \
                     OpenAI-compatible endpoint and one billing relationship.\n\n\
                     After saving, a curated cross-section of popular models \
                     becomes selectable in /model: gpt-4o, gpt-4o-mini, \
                     claude-3.5-sonnet/haiku, gemini-2.0-flash, \
                     llama-3.3-70b, qwen-2.5-72b, deepseek-chat. Default is \
                     gpt-4o-mini. Add others by editing \
                     ~/.config/hmanlab/config.json — see \
                     https://openrouter.ai/models for the full catalog.\n\n\
                     Billing is pay-per-token against your OpenRouter \
                     credit balance. Some rows are free (suffix `:free`).\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to openrouter.ai — never to hmanlab-api."
                    .to_string(),
            ),
            p if p == crate::config::OPENCODE_PROVIDER => (
                " add OpenCode Go key ".to_string(),
                "Paste your OpenCode API key (generate one at \
                     https://opencode.ai/zen). This provider points at the \
                     Go subscription endpoint — requests bill against your \
                     Go plan, not pay-per-credit.\n\n\
                     After saving, the Go-tier coding models become \
                     selectable in /model: glm-5.1, glm-5, \
                     qwen3.6/3.5-plus, kimi-k2.5/k2.6, minimax-m2.5/m2.7. \
                     Default is glm-5.1.\n\n\
                     Heads-up: Free-tier models (big-pickle, *-free) live \
                     on Zen's endpoint, not Go's — they're not in this \
                     provider's catalog and would 401 ModelError if added \
                     manually. Closed-weight models (claude-*, gpt-*, \
                     gemini-*) use non-OpenAI wire shapes and aren't routed \
                     through this provider yet.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to opencode.ai — never to hmanlab-api."
                    .to_string(),
            ),
            p if p == crate::config::ZAI_USAGE_PROVIDER => (
                " add z.ai (usage-based) key ".to_string(),
                "Paste your z.ai usage-based API key. After saving, all three \
                     z.ai models (glm-4.7, glm-4.6, glm-5.1) become selectable in \
                     /model and you'll be switched to glm-4.7 by default.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to z.ai — never to hmanlab-api."
                    .to_string(),
            ),
            p if p == crate::config::HMANLAB_PROVIDER => (
                " add hmanlab key ".to_string(),
                "Paste your hmanlab API key (generate one at \
                     https://ai.hmanlab.pro). After saving, the hmanlab-served \
                     models become selectable in /model.\n\n\
                     Models available: claude-sonnet-4-6, claude-opus-4-8, \
                     claude-opus-4-7, claude-haiku-4-5, gpt-5.5, gpt-5.4.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to ai.hmanlab.pro."
                    .to_string(),
            ),
            _ => (
                " add z.ai key ".to_string(),
                "Paste your z.ai coding-plan API key. After saving, all three \
                     z.ai models (glm-4.7, glm-4.6, glm-5.1) become selectable in \
                     /model and you'll be switched to glm-4.7 by default.\n\n\
                     The key is stored in ~/.config/hmanlab/config.json (mode \
                     0600) and only sent to z.ai — never to hmanlab-api."
                    .to_string(),
            ),
        };
        (title, body, "Enter to save  ·  Esc to cancel")
    };

    let block = theme::popup_block(title.trim(), false).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let content_width = (inner.width as usize).saturating_sub(2).max(10);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(2),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    f.render_widget(block, area);

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

    app.add_model_input.set_block(
        ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(theme::color::ACCENT))
            .title(Span::styled(
                " ❯ input ",
                Style::default()
                    .fg(theme::color::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ))
            .padding(Padding::horizontal(1)),
    );
    f.render_widget(&app.add_model_input, chunks[1]);

    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(theme::color::FG_DIM)),
        chunks[2],
    );
}
