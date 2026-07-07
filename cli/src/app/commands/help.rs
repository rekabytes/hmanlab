//! `/help` — print the inline command + tool + keys cheat-sheet.
//!
//! The commands section is generated from `SLASH_COMMANDS` so adding /
//! renaming a command in one place automatically updates the help. The
//! tool surface and keymap sections stay hand-written because they
//! aren't surfaced through the same metadata.

use super::super::{App, SLASH_COMMANDS};

impl App {
    pub(in crate::app) fn show_help_inline(&mut self) {
        let mut help = String::from("Commands:\n");
        for line in commands_section_lines() {
            help.push_str(&line);
            help.push('\n');
        }
        help.push_str(
            "\n\
            Tools (agent uses these on its own — needs a tool-capable model):\n\
            \x20 read_file, list_dir, find_files, git_status, git_log, git_diff,\n\
            \x20 git_show, run_command (shell — you confirm each call).\n\
            \n\
            Keys:\n\
            \x20 Enter         send  ·  Alt+Enter / Ctrl+J  newline\n\
            \x20 Ctrl+N        new session  ·  Ctrl+T  fold/unfold all tool + thinking blocks\n\
            \x20 Wheel         scroll chat  ·  PgUp/PgDn  Home/End  also scroll\n\
            \x20 Ctrl+C        cancel/quit  ·  Esc  interrupt generation / clear draft\n\
            \n\
            Drag with your mouse to select text — copy with your terminal's normal\n\
            shortcut (Ctrl+Shift+C / Cmd+C). The wheel scrolls the chat in single-line\n\
            input mode; when composing multi-line input (Alt+Enter / Ctrl+J), use PgUp/PgDn.",
        );
        self.push_info(help);
    }
}

/// Render one line per command from `SLASH_COMMANDS`. Format:
/// `  /name, /alias, /alias <args>    desc`, with the names column
/// padded so the descriptions line up. Returns owned strings; the
/// caller joins them with newlines.
fn commands_section_lines() -> Vec<String> {
    // Pre-compute the names+args column to find the widest entry, so
    // descriptions land in a stable column without hardcoded padding.
    let entries: Vec<(String, &'static str)> = SLASH_COMMANDS
        .iter()
        .map(|cmd| {
            let mut names = format!("/{}", cmd.name);
            for alias in cmd.aliases {
                names.push_str(", /");
                names.push_str(alias);
            }
            if !cmd.args.is_empty() {
                names.push(' ');
                names.push_str(cmd.args);
            }
            (names, cmd.desc)
        })
        .collect();
    let col_w = entries
        .iter()
        .map(|(names, _)| names.chars().count())
        .max()
        .unwrap_or(0);
    entries
        .into_iter()
        .map(|(names, desc)| format!("  {names:<col_w$}  {desc}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{parse_command, Command};
    use super::*;

    /// Every entry's canonical name must parse to a non-Unknown
    /// variant. Catches drift where someone adds a SLASH_COMMANDS row
    /// without wiring it into `parse_command`.
    #[test]
    fn every_canonical_name_parses() {
        for cmd in SLASH_COMMANDS {
            let input = format!("/{}", cmd.name);
            let parsed = parse_command(&input)
                .unwrap_or_else(|| panic!("SLASH_COMMANDS entry '{}' didn't parse", cmd.name));
            if let Command::Unknown(other) = parsed {
                panic!(
                    "SLASH_COMMANDS entry '{}' parsed as Unknown('{}')",
                    cmd.name, other
                );
            }
        }
    }

    /// Every alias must also fold back to a non-Unknown variant — i.e.
    /// `slash_canonical` knows about it and the parser dispatches it.
    #[test]
    fn every_alias_parses() {
        for cmd in SLASH_COMMANDS {
            for alias in cmd.aliases {
                let input = format!("/{alias}");
                let parsed = parse_command(&input)
                    .unwrap_or_else(|| panic!("alias '{alias}' (of /{}) didn't parse", cmd.name));
                if let Command::Unknown(other) = parsed {
                    panic!(
                        "alias '{alias}' (of /{}) parsed as Unknown('{other}')",
                        cmd.name
                    );
                }
            }
        }
    }
}
