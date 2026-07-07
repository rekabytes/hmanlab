//! Pre-TUI workspace-trust prompt.
//!
//! Fires once at launch when the current workspace isn't on the persisted
//! trusted list. Uses crossterm raw mode for a minimal arrow-key + Enter
//! menu — runs BEFORE `EnterAlternateScreen`, so the prompt artifacts end
//! up in normal scrollback rather than the TUI buffer.
//!
//! Non-TTY launches (CI, piped input) skip the prompt and stay untrusted —
//! safer default than auto-trusting whatever folder the script happened
//! to land in.

use anyhow::Result;
use crossterm::{
    cursor, event,
    event::{Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::{self, IsTerminal, Write};
use std::path::Path;

/// Show the trust prompt for `workspace` and return the user's choice.
/// `true` = trust, `false` = skip / deny / no-TTY.
pub fn prompt_workspace_trust(workspace: &Path) -> Result<bool> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        return Ok(false);
    }

    // Header in cooked mode — uses raw ANSI codes so we don't pull in
    // ratatui just for a 4-line banner.
    println!();
    println!("\x1b[1m▎ Workspace trust\x1b[0m");
    println!("  \x1b[2m{}\x1b[0m", workspace.display());
    println!();
    println!("  The agent can edit files, run shell commands, and save");
    println!("  memories inside a trusted workspace. Read-only browsing");
    println!("  (read_file, list_dir, find_files, git_*) works either way.");
    println!();
    println!("  \x1b[2m↑↓ select  ·  Enter confirm  ·  Esc to skip\x1b[0m");
    println!();

    enable_raw_mode()?;
    let outcome = run_loop();
    disable_raw_mode()?;
    println!();
    outcome
}

fn run_loop() -> Result<bool> {
    let mut selected = false; // default to "No" — safer.
    render(selected, true)?;
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Tab => {
                selected = !selected;
                render(selected, false)?;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => return Ok(true),
            KeyCode::Char('n') | KeyCode::Char('N') => return Ok(false),
            KeyCode::Enter => return Ok(selected),
            KeyCode::Esc => return Ok(false),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                // Ctrl+C: bail out as "no", same as Esc.
                return Ok(false);
            }
            _ => {}
        }
    }
}

fn render(selected: bool, first: bool) -> Result<()> {
    let mut stdout = io::stdout();
    if !first {
        // Walk back over the two option rows from the previous render and
        // overwrite them. `\r` returns to column 0 before each write so we
        // don't pile lines on top of each other.
        execute!(stdout, cursor::MoveUp(2))?;
    }
    let opts = [
        (true, "Yes — trust this workspace"),
        (false, "No — keep destructive tools blocked"),
    ];
    for (val, text) in opts {
        let active = val == selected;
        if active {
            // Bright peach matches the TUI's primary accent.
            write!(
                stdout,
                "\r\x1b[2K  \x1b[1;38;2;250;179;135m▶ {text}\x1b[0m\r\n"
            )?;
        } else {
            write!(stdout, "\r\x1b[2K  \x1b[2m  {text}\x1b[0m\r\n")?;
        }
    }
    stdout.flush()?;
    Ok(())
}
