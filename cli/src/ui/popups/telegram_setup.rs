//! `render_telegram_setup` — two-step `/telegram` wizard. Step 1
//! collects the @BotFather token; step 2 collects the pair code the
//! bot DM'd. Layout mirrors `add_model`: a body paragraph above a
//! framed input, with an inline error line and a key-hint footer.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Padding, Paragraph},
    Frame,
};

use crate::app::{App, TelegramSetupStep};

use super::super::markdown::{parse_inline_md, wrap_styled_segments};
use super::super::theme;

pub(in crate::ui) fn render_telegram_setup(f: &mut Frame, area: Rect, app: &mut App) {
    f.render_widget(Clear, area);

    let bot_username = app.telegram.as_ref().and_then(|rt| rt.bot_username.clone());

    let (title, body, hint) = match app.telegram_setup_step {
        TelegramSetupStep::Token => (
            " telegram setup · step 1 of 2 ".to_string(),
            "Open Telegram and message **@BotFather**. Run `/newbot`, follow the prompts, \
             and copy the token it gives you — it looks like `1234567890:ABC-xyz`.\n\n\
             Paste the token below. We'll validate it with Telegram's API and \
             move you to step 2.\n\n\
             The token is stored in `~/.config/hmanlab/config.json` (mode 0600) \
             and only sent to api.telegram.org — never to hmanlab-api."
                .to_string(),
            "Enter to validate  ·  Esc to cancel",
        ),
        TelegramSetupStep::Pair => {
            let bot_line = match &bot_username {
                Some(u) => format!("✓ Bot @{u} is online and waiting.\n\n"),
                None => "✓ Bot is online and waiting.\n\n".to_string(),
            };
            let url_hint = match &bot_username {
                Some(u) => format!("`t.me/{u}`"),
                None => "your bot's t.me/<username> link".to_string(),
            };
            (
                " telegram setup · step 2 of 2 ".to_string(),
                format!(
                    "{bot_line}\
                     1. Open {url_hint} (or search your bot in Telegram).\n\
                     2. Send any message to the bot (a plain `/start` is fine).\n\
                     3. The bot will DM you a 6-char pairing code.\n\
                     4. Paste the code below to finish pairing.\n\n\
                     The code expires after 10 minutes. If it does, just send \
                     another message and the bot will mint a new one."
                ),
                "Enter to pair  ·  Esc to cancel (bot stays running, you can pair later)",
            )
        }
    };

    let block = theme::popup_block(title.trim(), false).padding(Padding::horizontal(1));
    let inner = block.inner(area);
    let content_width = (inner.width as usize).saturating_sub(2).max(10);

    // Body / input / error / hint. The error row is optional; we
    // reserve 1 line for it either way so the layout doesn't jump
    // around when an error appears.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(2),    // body
            Constraint::Length(3), // input
            Constraint::Length(1), // error (or blank)
            Constraint::Length(1), // hint
        ])
        .split(inner);

    f.render_widget(block, area);

    // Body paragraph — supports the same inline markdown subset as
    // add_model (backticks, bold).
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

    // Input — swap the textarea for a "validating…" placeholder while
    // the async getMe roundtrip is in flight.
    if app.telegram_setup_validating {
        let validating = Paragraph::new(Line::from(Span::styled(
            " ⟳ validating token with Telegram…",
            Style::default()
                .fg(theme::color::ACCENT)
                .add_modifier(Modifier::ITALIC),
        )))
        .block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(theme::color::ACCENT_DIM))
                .padding(Padding::horizontal(1)),
        );
        f.render_widget(validating, chunks[1]);
    } else {
        let input_title = match app.telegram_setup_step {
            TelegramSetupStep::Token => " ❯ token ",
            TelegramSetupStep::Pair => " ❯ code ",
        };
        app.telegram_setup_input.set_block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(theme::color::ACCENT))
                .title(Span::styled(
                    input_title,
                    Style::default()
                        .fg(theme::color::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ))
                .padding(Padding::horizontal(1)),
        );
        f.render_widget(&app.telegram_setup_input, chunks[1]);
    }

    // Error row.
    if let Some(err) = &app.telegram_setup_error {
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
