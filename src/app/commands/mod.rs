//! Slash command parsing.
//!
//! The actual command *implementations* (switch_model, switch_workspace, …)
//! live as methods on `App` in `event.rs` and will move into per-domain
//! files under this directory in subsequent refactor steps. For now this
//! module owns:
//!   - the `Command` enum (the parser's output type)
//!   - `parse_command` (text → `Option<Command>`)
//!
//! The dispatcher (`App::handle_command`) stays in `event.rs` until the
//! command impls are colocated here.

pub(super) mod agent_templates;
pub(super) mod agents;
mod attach;
mod disconnect;
mod help;
mod host;
pub(super) mod mcp;
pub(super) mod model;
mod session;
mod settings;
pub(super) mod telegram;

/// One typed slash command. Output of [`parse_command`]; consumed by
/// `App::handle_command` in `event.rs`.
pub(super) enum Command {
    Model(Option<String>),
    Clear,
    Quit,
    Help,
    Host(String),
    New,
    ListSessions,
    Load(String),
    More,
    Workspace(String),
    Compact,
    Disconnect(String),
    Update,
    Settings,
    Trust,
    Untrust,
    Telegram(TelegramSub),
    /// `/agents [sub] [arg]` — manage specialist roster + session
    /// activation. See [`AgentsSub`].
    Agents(AgentsSub),
    /// `/ask <name> <query>` — manually invoke a specialist agent.
    /// Bypasses the main model entirely; runs a fresh agent loop with
    /// the specialist's model + system prompt + isolated history.
    Ask {
        name: String,
        query: String,
    },
    /// `/attach <path>` — read a file from disk and queue it as an
    /// attachment on the next user message. Images get inlined as
    /// `image_url` parts; other media types ride along the same way
    /// but only models that understand them will actually use the
    /// bytes.
    Attach(String),
    /// `/detach [name|all]` — drop a queued attachment. Empty arg drops
    /// the most recent; `all` clears the queue; a partial filename
    /// drops the first match.
    Detach(String),
    /// `/paste` — pull the current clipboard into chat. Image → queued
    /// as attachment; text → inserted into the input. Mirrors Ctrl+V;
    /// exists as an explicit fallback for terminals that swallow
    /// Ctrl+V before the app sees it.
    Paste,
    /// `/mcp` — open the MCP web search provider setup modal.
    /// Lets the user pick a search provider (Brave, Exa, Tavily, Parallel)
    /// and enter their API key. Once configured, `web_search` and
    /// `web_fetch` tools are added to the model's tool surface.
    Mcp,
    Unknown(String),
}

/// `/agents <sub> [arg]` — see the dispatch in
/// [`super::agents::handle_agents`] for what each sub does.
pub(super) enum AgentsSub {
    /// `/agents` / `/agents help` — show roster + session state.
    Show,
    /// `/agents on` / `/agents off` — flip session activation.
    SetEnabled(bool),
    /// `/agents list` — pretty-print roster (same as Show but more
    /// verbose; aliased here for muscle memory).
    List,
    /// `/agents add` — opens the four-step wizard.
    Add,
    /// `/agents remove <name>` — drop a specialist.
    Remove(String),
    /// `/agents edit <name>` — re-open the wizard pre-filled.
    Edit(String),
    /// `/agents enable <name>` / `/agents disable <name>`.
    SetSpecialistEnabled { name: String, enabled: bool },
    /// Catch-all for `/agents <gibberish>`. The handler shows a
    /// "did-you-mean" suggestion built from edit distance over the
    /// known subcommand names.
    Unknown(String),
}

/// `/telegram <sub> [arg]` — pair the local TUI to a Telegram bot.
/// The bot lives in this process, so `setup` and `off` start/stop the
/// long-poll task; `pair` redeems a code minted by the bot when an
/// unknown sender first DMs it (Pattern C).
pub(super) enum TelegramSub {
    /// `/telegram setup <token>` — set the @BotFather token and start
    /// the bot. Validates via `getMe` first.
    Setup(String),
    /// `/telegram pair <code>` — redeem the 6-char code the bot DM'd
    /// the user, adding their Telegram id to the allowlist.
    Pair(String),
    /// `/telegram status` — show whether the bot is configured / online
    /// and how many users are paired.
    Status,
    /// `/telegram unpair` — clear the allowlist but keep the token /
    /// bot running. Useful if a paired account is compromised.
    Unpair,
    /// `/telegram off` — stop the bot, clear token + allowlist.
    Off,
    /// `/telegram notify [on|off]` — toggle the "DM me when a local
    /// turn finishes after the terminal goes idle" notification. With
    /// no arg, prints the current state.
    Notify(Option<bool>),
    /// `/telegram` with no subcommand, or an unknown sub — handler
    /// prints usage.
    Help,
}

/// Parse a textarea line into a [`Command`], or `None` if the line
/// isn't a slash command at all. Aliases are folded back to the
/// canonical name via `inline::slash_canonical` (single source of
/// truth in `SLASH_COMMANDS`) so adding an alias only requires
/// touching that table — this match dispatches on the canonical name
/// only.
pub(super) fn parse_command(text: &str) -> Option<Command> {
    let t = text.trim();
    if !t.starts_with('/') {
        return None;
    }
    let body = &t[1..];
    let (head, rest) = match body.split_once(char::is_whitespace) {
        Some((h, r)) => (h.to_string(), r.trim().to_string()),
        None => (body.to_string(), String::new()),
    };
    let canonical = super::inline::slash_canonical(&head);
    Some(match canonical {
        Some("model") => Command::Model(if rest.is_empty() { None } else { Some(rest) }),
        Some("clear") => Command::Clear,
        Some("quit") => Command::Quit,
        Some("help") => Command::Help,
        Some("host") => Command::Host(rest),
        Some("new") => Command::New,
        Some("sessions") => Command::ListSessions,
        Some("load") => Command::Load(rest),
        Some("more") => Command::More,
        Some("workspace") => Command::Workspace(rest),
        Some("compact") => Command::Compact,
        Some("disconnect") => Command::Disconnect(rest),
        Some("update") => Command::Update,
        Some("settings") => Command::Settings,
        Some("trust") => Command::Trust,
        Some("untrust") => Command::Untrust,
        Some("telegram") => Command::Telegram(parse_telegram_sub(&rest)),
        Some("agents") => Command::Agents(parse_agents_sub(&rest)),
        Some("ask") => parse_ask(&rest),
        Some("attach") => Command::Attach(rest),
        Some("detach") => Command::Detach(rest),
        Some("paste") => Command::Paste,
        Some("mcp") => Command::Mcp,
        Some(_) | None => Command::Unknown(head.to_ascii_lowercase()),
    })
}

/// `/agents <sub> [arg]` parser. Empty sub → Show (also covers
/// `/agents` with no body and `/agents help`).
fn parse_agents_sub(rest: &str) -> AgentsSub {
    let rest = rest.trim();
    if rest.is_empty() {
        return AgentsSub::Show;
    }
    let (sub, arg) = match rest.split_once(char::is_whitespace) {
        Some((s, a)) => (s.to_ascii_lowercase(), a.trim().to_string()),
        None => (rest.to_ascii_lowercase(), String::new()),
    };
    match sub.as_str() {
        "help" | "show" | "status" | "?" => AgentsSub::Show,
        "on" | "enable" => AgentsSub::SetEnabled(true),
        "off" | "disable" => AgentsSub::SetEnabled(false),
        "list" | "ls" => AgentsSub::List,
        "add" | "new" => AgentsSub::Add,
        "remove" | "rm" | "delete" | "del" => AgentsSub::Remove(arg),
        "edit" | "update" => AgentsSub::Edit(arg),
        "enable-agent" => AgentsSub::SetSpecialistEnabled {
            name: arg,
            enabled: true,
        },
        "disable-agent" => AgentsSub::SetSpecialistEnabled {
            name: arg,
            enabled: false,
        },
        // Catch-all: handler shows a did-you-mean suggestion built
        // from edit distance against the known subcommand names.
        other => AgentsSub::Unknown(other.to_string()),
    }
}

/// `/ask <name> <query>` — name is the first word, everything after
/// the first space is the query. Empty query falls through to Unknown
/// so the handler can print usage.
fn parse_ask(rest: &str) -> Command {
    let rest = rest.trim();
    if rest.is_empty() {
        return Command::Ask {
            name: String::new(),
            query: String::new(),
        };
    }
    match rest.split_once(char::is_whitespace) {
        Some((name, query)) => Command::Ask {
            name: name.to_string(),
            query: query.trim().to_string(),
        },
        None => Command::Ask {
            name: rest.to_string(),
            query: String::new(),
        },
    }
}

/// Split the tail of `/telegram <sub> [arg]` into the matching
/// [`TelegramSub`]. Unknown subs (and an empty tail) fall through to
/// `Help`, so the handler can print usage instead of silently dropping
/// the command.
fn parse_telegram_sub(rest: &str) -> TelegramSub {
    let rest = rest.trim();
    if rest.is_empty() {
        return TelegramSub::Help;
    }
    let (sub, arg) = match rest.split_once(char::is_whitespace) {
        Some((s, a)) => (s.to_ascii_lowercase(), a.trim().to_string()),
        None => (rest.to_ascii_lowercase(), String::new()),
    };
    match sub.as_str() {
        "setup" | "token" => TelegramSub::Setup(arg),
        "pair" | "connect" => TelegramSub::Pair(arg),
        "status" | "info" => TelegramSub::Status,
        "unpair" | "forget" => TelegramSub::Unpair,
        "off" | "stop" | "disable" => TelegramSub::Off,
        "notify" | "notifications" => TelegramSub::Notify(parse_on_off(&arg)),
        _ => TelegramSub::Help,
    }
}

/// Tri-state parse for the `notify` arg: `on`/`enable`/`true` → Some(true),
/// `off`/`disable`/`false` → Some(false), anything else (including
/// empty) → None ("just show me the current state").
fn parse_on_off(arg: &str) -> Option<bool> {
    match arg.trim().to_ascii_lowercase().as_str() {
        "on" | "enable" | "enabled" | "true" | "yes" | "1" => Some(true),
        "off" | "disable" | "disabled" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod telegram_parse_tests {
    use super::*;

    fn parse(s: &str) -> Command {
        parse_command(s).expect("expected a slash command")
    }

    fn telegram_sub(s: &str) -> TelegramSub {
        match parse(s) {
            Command::Telegram(sub) => sub,
            _ => panic!("expected Command::Telegram"),
        }
    }

    #[test]
    fn telegram_bare_yields_help() {
        assert!(matches!(telegram_sub("/telegram"), TelegramSub::Help));
        assert!(matches!(telegram_sub("/tg"), TelegramSub::Help));
        assert!(matches!(telegram_sub("/telegram   "), TelegramSub::Help));
    }

    #[test]
    fn telegram_setup_captures_token() {
        match telegram_sub("/telegram setup 12345:ABCxyz") {
            TelegramSub::Setup(t) => assert_eq!(t, "12345:ABCxyz"),
            other => panic!("expected Setup, got {:?}", std::mem::discriminant(&other)),
        }
        // `token` alias works.
        match telegram_sub("/telegram token 99:zzz") {
            TelegramSub::Setup(t) => assert_eq!(t, "99:zzz"),
            _ => panic!("expected Setup"),
        }
    }

    #[test]
    fn telegram_pair_lowercases_subcommand_only() {
        // Sub is case-insensitive; the code itself must be preserved
        // verbatim — `redeem_code` upper-cases on lookup.
        match telegram_sub("/telegram PAIR K7m3Q9") {
            TelegramSub::Pair(c) => assert_eq!(c, "K7m3Q9"),
            _ => panic!("expected Pair"),
        }
    }

    #[test]
    fn telegram_status_unpair_off_aliases() {
        assert!(matches!(
            telegram_sub("/telegram status"),
            TelegramSub::Status
        ));
        assert!(matches!(
            telegram_sub("/telegram info"),
            TelegramSub::Status
        ));
        assert!(matches!(
            telegram_sub("/telegram unpair"),
            TelegramSub::Unpair
        ));
        assert!(matches!(
            telegram_sub("/telegram forget"),
            TelegramSub::Unpair
        ));
        assert!(matches!(telegram_sub("/telegram off"), TelegramSub::Off));
        assert!(matches!(telegram_sub("/telegram stop"), TelegramSub::Off));
        assert!(matches!(
            telegram_sub("/telegram disable"),
            TelegramSub::Off
        ));
    }

    #[test]
    fn telegram_notify_tri_state() {
        // Bare → show current state (None).
        assert!(matches!(
            telegram_sub("/telegram notify"),
            TelegramSub::Notify(None)
        ));
        // Truthy.
        for arg in &["on", "enable", "enabled", "true", "yes", "1"] {
            match telegram_sub(&format!("/telegram notify {arg}")) {
                TelegramSub::Notify(Some(true)) => {}
                _ => panic!("expected Notify(Some(true)) for {arg}"),
            }
        }
        // Falsy.
        for arg in &["off", "disable", "disabled", "false", "no", "0"] {
            match telegram_sub(&format!("/telegram notify {arg}")) {
                TelegramSub::Notify(Some(false)) => {}
                _ => panic!("expected Notify(Some(false)) for {arg}"),
            }
        }
        // Garbage → show, don't toggle blindly.
        assert!(matches!(
            telegram_sub("/telegram notify maybe"),
            TelegramSub::Notify(None)
        ));
    }

    #[test]
    fn unknown_subcommand_falls_through_to_help() {
        assert!(matches!(
            telegram_sub("/telegram launch-the-nukes"),
            TelegramSub::Help
        ));
    }
}

#[cfg(test)]
mod agents_parse_tests {
    use super::*;

    fn parse(s: &str) -> Command {
        parse_command(s).expect("expected a slash command")
    }

    fn agents_sub(s: &str) -> AgentsSub {
        match parse(s) {
            Command::Agents(sub) => sub,
            _ => panic!("expected Command::Agents"),
        }
    }

    #[test]
    fn agents_bare_shows_help() {
        assert!(matches!(agents_sub("/agents"), AgentsSub::Show));
        assert!(matches!(agents_sub("/agent"), AgentsSub::Show));
        assert!(matches!(agents_sub("/team"), AgentsSub::Show));
        assert!(matches!(agents_sub("/agents help"), AgentsSub::Show));
    }

    #[test]
    fn agents_on_off_aliases() {
        assert!(matches!(
            agents_sub("/agents on"),
            AgentsSub::SetEnabled(true)
        ));
        assert!(matches!(
            agents_sub("/agents enable"),
            AgentsSub::SetEnabled(true)
        ));
        assert!(matches!(
            agents_sub("/agents off"),
            AgentsSub::SetEnabled(false)
        ));
        assert!(matches!(
            agents_sub("/agents disable"),
            AgentsSub::SetEnabled(false)
        ));
    }

    #[test]
    fn agents_list_add_aliases() {
        assert!(matches!(agents_sub("/agents list"), AgentsSub::List));
        assert!(matches!(agents_sub("/agents ls"), AgentsSub::List));
        assert!(matches!(agents_sub("/agents add"), AgentsSub::Add));
        assert!(matches!(agents_sub("/agents new"), AgentsSub::Add));
    }

    #[test]
    fn agents_remove_captures_name() {
        match agents_sub("/agents remove coder") {
            AgentsSub::Remove(n) => assert_eq!(n, "coder"),
            _ => panic!("expected Remove"),
        }
        match agents_sub("/agents rm reviewer") {
            AgentsSub::Remove(n) => assert_eq!(n, "reviewer"),
            _ => panic!("expected Remove"),
        }
    }

    #[test]
    fn agents_edit_captures_name() {
        match agents_sub("/agents edit coder") {
            AgentsSub::Edit(n) => assert_eq!(n, "coder"),
            _ => panic!("expected Edit"),
        }
    }

    #[test]
    fn agents_enable_disable_per_specialist() {
        match agents_sub("/agents enable-agent coder") {
            AgentsSub::SetSpecialistEnabled { name, enabled } => {
                assert_eq!(name, "coder");
                assert!(enabled);
            }
            _ => panic!("expected SetSpecialistEnabled(true)"),
        }
        match agents_sub("/agents disable-agent coder") {
            AgentsSub::SetSpecialistEnabled { name, enabled } => {
                assert_eq!(name, "coder");
                assert!(!enabled);
            }
            _ => panic!("expected SetSpecialistEnabled(false)"),
        }
    }

    #[test]
    fn ask_splits_name_and_query() {
        match parse("/ask coder write a hello world") {
            Command::Ask { name, query } => {
                assert_eq!(name, "coder");
                assert_eq!(query, "write a hello world");
            }
            _ => panic!("expected Ask"),
        }
    }

    #[test]
    fn ask_with_no_query_keeps_name_only() {
        match parse("/ask coder") {
            Command::Ask { name, query } => {
                assert_eq!(name, "coder");
                assert!(query.is_empty());
            }
            _ => panic!("expected Ask"),
        }
    }

    #[test]
    fn ask_bare_yields_empty_pair() {
        match parse("/ask") {
            Command::Ask { name, query } => {
                assert!(name.is_empty());
                assert!(query.is_empty());
            }
            _ => panic!("expected Ask"),
        }
    }
}

#[cfg(test)]
mod agents_config_tests {
    use crate::config::{AgentsConfig, SpecialistAgent};

    fn make(name: &str, enabled: bool) -> SpecialistAgent {
        SpecialistAgent {
            name: name.to_string(),
            model: "test-model".to_string(),
            provider: None,
            task: "test".to_string(),
            system_prompt: "test".to_string(),
            enabled,
        }
    }

    #[test]
    fn roundtrips_via_serde() {
        let cfg = AgentsConfig {
            specialists: vec![make("coder", true), make("reviewer", false)],
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: AgentsConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.specialists.len(), 2);
        assert_eq!(back.specialists[0].name, "coder");
        assert!(back.specialists[0].enabled);
        assert_eq!(back.specialists[1].name, "reviewer");
        assert!(!back.specialists[1].enabled);
    }

    #[test]
    fn enabled_by_name_is_case_insensitive_and_skips_disabled() {
        let cfg = AgentsConfig {
            specialists: vec![make("coder", true), make("Reviewer", false)],
        };
        assert!(cfg.enabled_by_name("CODER").is_some());
        assert!(cfg.enabled_by_name("coder").is_some());
        // Disabled specialists are invisible to enabled_by_name even
        // when the name matches.
        assert!(cfg.enabled_by_name("reviewer").is_none());
        assert!(cfg.enabled_by_name("nobody").is_none());
    }

    #[test]
    fn enabled_default_is_true_when_missing_from_json() {
        // Older configs (or hand-written ones) may omit the `enabled`
        // field. The default_true() shim should fill it in.
        let json = r#"{"specialists":[{"name":"x","model":"m","task":"t","system_prompt":"p"}]}"#;
        let cfg: AgentsConfig = serde_json::from_str(json).expect("deserialize");
        assert!(cfg.specialists[0].enabled);
    }

    #[test]
    fn by_name_mut_finds_disabled_too() {
        // Editing a disabled specialist needs to work, otherwise you'd
        // be locked out of re-enabling.
        let mut cfg = AgentsConfig {
            specialists: vec![make("coder", false)],
        };
        let entry = cfg.by_name_mut("coder").expect("found");
        entry.enabled = true;
        assert!(cfg.specialists[0].enabled);
    }
}
