//! `render_chat` — assembles every visible row in the chat panel.
//!
//! Walks `app.messages` once, emitting both styled `Line`s (for ratatui)
//! and a parallel plain-text snapshot (for copy-on-drag). Branches on
//! message role + state:
//!   - Permission cards (inline y/n prompts).
//!   - Plain assistant / user / info / summary text.
//!
//! Tool-result messages render as one-line tiles using a single-icon
//! design language: every tool gets a ⚡ bolt, the verb (`read`,
//! `list`, `shell`) carries the differentiation. A run of N
//! consecutive tool calls from one assistant turn collapses into a
//! single `⚡⚡⚡ 3 reads · …` tile; click to expand reveals every
//! member's tile + body inline. Single-tile body expand state lives
//! on `App::expanded_tools`; multi-tool group expand state lives on
//! `App::expanded_tool_groups` (kept distinct so the two affordances
//! don't collide). The ⚡ bolt breathes (DIM pulse on the off-beat)
//! while a tool is in flight — see `helpers::pulse_modifier`.
//!
//! After paragraph render, two buffer-overlay passes paint the drag-select
//! rectangle and (for permission cards) the Approve/Deny button hover.

use ratatui::{
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::App;
use crate::ollama::ChatMessage;

use super::super::theme;
use super::super::wrap_cache::wrap_md_paragraph;
use super::helpers::{
    animated_ellipsis, breathing_count, compute_tool_groups, group_noun, group_progress,
    progress_bar, pulse_modifier, split_thinking, summarize_group, tool_arg_summary, tool_label,
    GroupProgress, ToolGroup,
};

pub(in crate::ui) fn render_chat(f: &mut Frame, area: Rect, app: &mut App) {
    // **Critical**: Clear the chat area before rendering. `Clear` calls
    // `cell.reset()` which wipes BOTH the character and style. Without
    // this, stale characters from the previous frame (e.g. scrolled-away
    // content, or content that changed while the renderer was mid-paint)
    // bleed through as ghost text. `Block::set_style` only patches
    // style, not `cell.symbol`.
    f.render_widget(ratatui::widgets::Clear, area);
    f.render_widget(
        Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(theme::color::BG_BASE)),
        area,
    );

    // Stash full-area geometry (no inner border shrink).
    app.render.chat_x = area.x;
    app.render.chat_y = area.y;
    app.render.chat_w = area.width;
    app.render.chat_h = area.height;

    let inner = area;
    let indent = "  ";
    // 2-col gutter under each speaker label: rendered as a colored `▎` bar
    // in the role's color. Recorded in `text_lines` as two spaces so
    // copy-on-drag doesn't grab the bar glyph and selection cell-widths
    // still line up.
    let gutter_glyph = "▎ ";
    let content_width = (area.width as usize).saturating_sub(indent.len()).max(10);

    let mut lines: Vec<Line> = Vec::new();
    // Parallel plain-text copy of `lines` so copy-on-drag can extract the
    // selected substring without re-parsing styled spans.
    let mut text_lines: Vec<String> = Vec::new();
    let mut ranges: Vec<(usize, u16, u16)> = Vec::with_capacity(app.messages.len());
    // Tool-tile hit-test entries — `(absolute_screen_y, message_idx)`.
    // Reset every frame so a collapsed/expanded toggle doesn't leave a
    // stale row pointing at an old message index. The mouse handler
    // reads this to convert click rows into expand/collapse toggles.
    app.render.card_row_targets.clear();
    let last_idx = app.messages.len().saturating_sub(1);

    // ── Tool-group precomputation ─────────────────────────────────
    // Consecutive tool messages from one assistant turn collapse into
    // a single `◰×3 read 3 files · …` tile so a multi-call turn
    // doesn't drown the column in near-identical rows. Built fresh
    // every frame from the current message list.
    //
    // `group_of[i]` = the start index of whatever group owns message
    // `i` (only set for `role == "tool"` messages). `multi_groups`
    // is the set of start indices whose group has 2+ members — those
    // are the ones that take the consolidated render path. Single-tool
    // groups fall through to the existing tile code unchanged.
    let groups = compute_tool_groups(&app.messages);
    let group_by_start: std::collections::HashMap<usize, ToolGroup> =
        groups.iter().map(|g| (g.start, *g)).collect();
    let mut group_of: std::collections::HashMap<usize, usize> =
        std::collections::HashMap::with_capacity(app.messages.len());
    let mut multi_groups: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // Surfaced to the click handler so it can dispatch group-affordance
    // clicks (toggle `expanded_tool_groups`) vs single-tile clicks
    // (toggle `expanded_tools`). Reset every frame alongside
    // `card_row_targets`.
    app.render.multi_group_starts.clear();
    for g in &groups {
        if g.count() > 1 {
            multi_groups.insert(g.start);
            app.render.multi_group_starts.insert(g.start);
        }
        for i in g.start..=g.end {
            group_of.insert(i, g.start);
        }
    }

    for (i, msg) in app.messages.iter().enumerate() {
        if msg.hidden {
            continue;
        }
        // Inline permission card — renders in-place where the tool
        // call happened so the user keeps the conversation context.
        // Approve/Deny buttons are mouse-clickable; y/n still works via
        // `handle_confirm`. The card is removed from chat history on
        // answer (see `App::resolve_confirm`).
        if msg.role == "permission" {
            let card = render_permission_card(msg, inner.width);
            let start = lines.len() as u16;
            for l in &card.lines {
                let plain: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                text_lines.push(plain);
            }
            lines.extend(card.lines);
            let end = lines.len() as u16;
            ranges.push((i, start, end));

            // Compute absolute hit rects for the Approve/Deny buttons.
            // `app.scroll` translates logical line index → visual row, so
            // we subtract it. We only register the rects while
            // `pending_confirm.is_some()` so a stale card from a prior
            // frame (the message lingers one frame between resolve and
            // removal) can't catch a stray click.
            if app.pending_confirm.is_some() {
                let abs_y = inner
                    .y
                    .saturating_add(start)
                    .saturating_add(card.button_row)
                    .saturating_sub(app.scroll);
                let abs_x_approve = inner.x.saturating_add(2).saturating_add(card.approve_col);
                let abs_x_deny = inner.x.saturating_add(2).saturating_add(card.deny_col);
                app.render.permission_btn_approve = Some((abs_x_approve, abs_y, card.btn_w, 1));
                app.render.permission_btn_deny = Some((abs_x_deny, abs_y, card.btn_w, 1));
            } else {
                app.render.permission_btn_approve = None;
                app.render.permission_btn_deny = None;
            }

            if i != last_idx {
                text_lines.push(String::new());
                lines.push(Line::from(""));
            }
            continue;
        }

        // Tool and assistant messages render as a header line (role
        // label + content) followed by the body text. Tool tiles take
        // a fast path when collapsed — a single header line with an
        // icon, label, arg summary, and a chevron — so a multi-tool
        // turn doesn't drown the column in raw output. The user clicks
        // the tile row to expand inline (see App::expanded_tools for
        // single tiles, App::expanded_tool_groups for batched groups).
        let line_start = lines.len() as u16;
        let is_tool = msg.role == "tool";
        let is_streaming_here = app.turn.is_generating() && i == last_idx;

        // Resolve this msg's group membership up front so both the
        // role-label decision and the fast-path decision below can
        // branch on it without re-walking the maps.
        //
        // `in_expanded_group` is true for every member (start or not)
        // of a multi-tool group the user has clicked to expand. Such
        // members always render tile + body inline — the per-tile body
        // expand state (`expanded_tools`) is bypassed because the group
        // affordance already speaks for them.
        let group_start_opt: Option<usize> = if is_tool {
            group_of.get(&i).copied()
        } else {
            None
        };
        let in_multi_group = group_start_opt
            .map(|start| multi_groups.contains(&start))
            .unwrap_or(false);
        let group_expanded = group_start_opt
            .map(|start| app.expanded_tool_groups.contains(&start))
            .unwrap_or(false);
        let in_expanded_group = in_multi_group && group_expanded;
        let is_group_start = group_start_opt.map(|start| start == i).unwrap_or(false);

        // Header line — role label for non-tool, tool name for tool.
        // Skip the label for non-start members of a multi-tool group:
        // a collapsed non-start member is skipped entirely (the
        // start's grouped tile speaks for it), and an expanded
        // non-start member belongs visually to the group (which
        // already got its `▎ tool` label from the start msg).
        // Repeating the label would either dangle (collapsed case,
        // where the body is `continue`d below) or read as N separate
        // speakers rather than one batch (expanded case).
        let skip_role_label = in_multi_group && !is_group_start;
        if !skip_role_label {
            let (label, color) = if is_tool {
                let (text, c) = theme::role_label("tool");
                (text.to_string(), c)
            } else {
                let (text, c) = theme::role_label(&msg.role);
                (text.to_string(), c)
            };
            text_lines.push(label.clone());
            let header_style = Style::default().fg(color).add_modifier(Modifier::BOLD);
            lines.push(Line::from(Span::styled(label, header_style)));
        }

        // ── Multi-tool group handling ─────────────────────────────
        // Consecutive tool messages from one assistant turn
        // consolidate into a single clickable tile when collapsed
        // (one row covers the whole batch) or a header-above-tiles
        // stack when expanded (group header on top, each individual
        // tool's tile + body renders underneath with no loss of
        // detail). Single-tool groups and non-tool messages skip
        // this block entirely and take the existing render path.
        if in_multi_group {
            // `multi_groups` only holds starts we registered during
            // precomputation, so the lookup is infallible.
            let start = group_start_opt.unwrap();
            let group = *group_by_start.get(&start).unwrap();
            match (is_group_start, group_expanded) {
                // Non-start member of a collapsed group: the start
                // msg's grouped tile already spoke for us — skip
                // silently. No role label was pushed above (the
                // `in_expanded_group && !is_group_start` guard would
                // have skipped it anyway, but collapsed non-start
                // members want to skip the body too).
                (false, false) => continue,
                // Non-start member of an expanded group: render this
                // member's tile + body inline below. No group header
                // (only the start msg emits one).
                (false, true) => {}
                // Start of a collapsed group: render the grouped tile
                // (one row for the whole batch) and skip the body.
                (true, false) => {
                    let tile = render_grouped_tile(&app.messages, group, app, inner.width);
                    let tile_y = lines.len() as u16;
                    lines.push(tile.line);
                    text_lines.push(tile.text);
                    let abs_y = inner.y.saturating_add(tile_y).saturating_sub(app.scroll);
                    app.render.card_row_targets.push((abs_y, start));
                    let line_end_excl = lines.len() as u16;
                    ranges.push((start, line_start, line_end_excl));
                    if i != last_idx {
                        text_lines.push(String::new());
                        lines.push(Line::from(""));
                    }
                    continue;
                }
                // Start of an expanded group: render the group header
                // above this msg's individual tile + body, then fall
                // through so the individual tile + body render below.
                (true, true) => {
                    let header = render_group_header(&app.messages, group, app, inner.width);
                    let header_y = lines.len() as u16;
                    lines.push(header.line);
                    text_lines.push(header.text);
                    let abs_y = inner.y.saturating_add(header_y).saturating_sub(app.scroll);
                    app.render.card_row_targets.push((abs_y, start));
                    // Fall through — `line_start` was captured before
                    // the role label, so the eventual range push below
                    // covers role-label + group-header + this msg's
                    // individual tile + body.
                }
            }
        }

        // ── Collapsed tool tile (single-tile or expanded-group member)
        // Skip the body-render below — just emit the one-line tile and
        // a click hit-test rect, then jump to the per-msg range push.
        // The tile's BG_CARD background is painted by a post-render
        // buffer pass over `card_row_targets`.
        //
        // Members of an expanded group ALSO render their tile here (one
        // row, same shape as a standalone collapsed tile), but then
        // fall through to the body render below so the output shows
        // inline. The click target for an expanded-group member's tile
        // is the GROUP START idx (clicking any tile in an expanded
        // group collapses the whole group back to the summary).
        let render_tile_only = is_tool && !app.expanded_tools.contains(&i) && !in_expanded_group;
        let render_tile_then_body = is_tool && in_expanded_group;
        if render_tile_only || render_tile_then_body {
            let tile = render_collapsed_tile(msg, i, app, inner.width);
            let tile_y = lines.len() as u16;
            lines.push(tile.line);
            text_lines.push(tile.text);
            // Register hit-test rect. The tile row is one screen row
            // tall; absolute y accounts for chat scroll so the click
            // handler doesn't need to know about scroll state.
            let abs_y = inner.y.saturating_add(tile_y).saturating_sub(app.scroll);
            // Click target: group start idx for members of an expanded
            // group (so any tile click collapses the whole group), or
            // this msg's idx for standalone single tiles.
            let click_idx = if in_expanded_group {
                group_start_opt.unwrap()
            } else {
                i
            };
            app.render.card_row_targets.push((abs_y, click_idx));
            if render_tile_only {
                let line_end_excl = lines.len() as u16;
                ranges.push((i, line_start, line_end_excl));
                if i != last_idx {
                    text_lines.push(String::new());
                    lines.push(Line::from(""));
                }
                continue;
            }
            // Else: `render_tile_then_body` — fall through so the body
            // renders below the tile.
        }

        // Visible body — `split_thinking` returns (`None`, "") while
        // still inside a <think>…</think> block, and (`None`, raw)
        // for non-reasoning models. For tool messages we just dump
        // `msg.content` directly (the tool output is what it is).
        let visible_content: &str = if is_tool {
            &msg.content
        } else {
            // `generating` is the `is_streaming_here` flag — passed so
            // mid-stream thinking doesn't leak the partial tag.
            let (_, v) = split_thinking(&msg.content, is_streaming_here);
            v
        };
        let trimmed = visible_content.trim_end_matches(['\n', '\r']);

        // Body render. Skip if the message is empty and not currently
        // streaming (nothing to show). For an actively-streaming
        // assistant with no content yet, show a breathing "..."
        // line so the user knows generation is in progress.
        if trimmed.is_empty() {
            if !is_tool && is_streaming_here {
                text_lines.push(format!("{indent}…"));
                let breath = Style::default().fg(theme::color::FG_DIM);
                lines.push(Line::from(vec![
                    Span::styled(gutter_glyph.to_string(), breath),
                    Span::styled("…", breath),
                ]));
            }
        } else {
            let base_style = match msg.role.as_str() {
                "info" => Style::default().fg(theme::color::SYSTEM),
                "summary" => Style::default().fg(theme::color::SYSTEM),
                "tool" => Style::default().fg(theme::color::FG_DIM),
                _ => Style::default().fg(theme::color::FG),
            };
            let gutter_style = Style::default().fg(theme::color::FG_DIMMER);
            for paragraph in trimmed.split('\n') {
                if paragraph.is_empty() {
                    text_lines.push(String::new());
                    lines.push(Line::from(""));
                    continue;
                }
                let wrapped = wrap_md_paragraph(paragraph, base_style, content_width);
                for spans in wrapped {
                    let mut plain = String::with_capacity(content_width);
                    plain.push_str(indent);
                    for span in &spans {
                        plain.push_str(span.content.as_ref());
                    }
                    text_lines.push(plain);
                    let mut line_spans: Vec<Span<'static>> = Vec::with_capacity(spans.len() + 1);
                    line_spans.push(Span::styled(gutter_glyph.to_string(), gutter_style));
                    line_spans.extend(spans);
                    lines.push(Line::from(line_spans));
                }
            }
            // Blinking caret at the tail of the in-flight assistant
            // reply. Toggles every 4 ticks (~480 ms at the 120 ms
            // ticker) — reads as a real terminal cursor. Appended
            // only to `lines` so copy-on-drag won't pick up the glyph
            // from `text_lines`. Off-state is a space so the visual
            // width doesn't shift between blinks.
            if is_streaming_here && msg.role == "assistant" {
                let caret = if (app.anim_tick / 4) % 2 == 0 {
                    "▌"
                } else {
                    " "
                };
                if let Some(line) = lines.last_mut() {
                    line.spans
                        .push(Span::styled(caret, Style::default().fg(theme::color::FG)));
                }
            }
        }

        let line_end_excl = lines.len() as u16;
        ranges.push((i, line_start, line_end_excl));

        // Spacer between messages, but not after the very last one.
        if i != last_idx {
            text_lines.push(String::new());
            lines.push(Line::from(""));
        }
    }
    app.render.rendered_text_lines = text_lines;
    app.render.message_line_ranges = ranges;

    // Scroll math has to use VISUAL row count, not `lines.len()`. With
    // `Wrap { trim: false }`, one logical Line can render as multiple
    // rows when it's wider than the viewport. The earlier version used
    // `lines.len() as u16` here, which under-counted long messages — a
    // 3-paragraph reply where each paragraph wraps to ~3 visual rows
    // would only let scroll reach the middle of the last paragraph,
    // cutting off the bottom even with follow=true. The copy buffer
    // (`rendered_text_lines`) intentionally stays one-per-logical-line
    // so click hit-testing keeps working; only the scroll bound changes.
    let mut para = Paragraph::new(lines)
        .style(Style::default().bg(theme::color::BG_BASE))
        .wrap(Wrap { trim: false });
    let total = para.line_count(area.width).min(u16::MAX as usize) as u16;
    let visible = inner.height;
    let max_scroll = total.saturating_sub(visible);

    if app.follow {
        app.scroll = max_scroll;
    } else {
        app.scroll = app.scroll.min(max_scroll);
    }

    para = para.scroll((app.scroll, 0));
    f.render_widget(para, area);

    // Paint the BG_CARD background under each collapsed tool tile.
    // Done as a post-render buffer pass (same pattern as the selection
    // overlay below) so the tile reads as one elevated block without
    // needing a border. Each row's x-range is the inner chat width;
    // y is the absolute screen row the renderer registered during the
    // text pass. Out-of-range rows (e.g. scrolled past the top) are
    // skipped — the rectangle math just no-ops on them.
    {
        let buf = f.buffer_mut();
        for &(abs_y, _msg_idx) in &app.render.card_row_targets {
            if abs_y < inner.y || abs_y >= inner.y.saturating_add(inner.height) {
                continue;
            }
            for x in inner.x..inner.x.saturating_add(inner.width) {
                if let Some(cell) = buf.cell_mut(Position::new(x, abs_y)) {
                    let s = cell.style().bg(theme::color::BG_CARD);
                    cell.set_style(s);
                }
            }
        }
    }

    // Paint the selection overlay on top of the chat. Cells inside the
    // (sel_start, sel_end) rectangle, clamped to the chat inner area, get the
    // REVERSED modifier so they look highlighted.
    if let (Some(start), Some(end)) = (app.sel_start, app.sel_end) {
        let ((sx, sy), (ex, ey)) = if (start.1, start.0) <= (end.1, end.0) {
            (start, end)
        } else {
            (end, start)
        };
        let cx_min = inner.x;
        let cx_max = inner.x.saturating_add(inner.width).saturating_sub(1);
        let cy_min = inner.y;
        let cy_max = inner.y.saturating_add(inner.height).saturating_sub(1);
        let row_lo = sy.max(cy_min);
        let row_hi = ey.min(cy_max);
        if row_lo <= row_hi {
            let buf = f.buffer_mut();
            for y in row_lo..=row_hi {
                let row_start = if y == sy { sx.max(cx_min) } else { cx_min };
                let row_end = if y == ey { ex.min(cx_max) } else { cx_max };
                if row_start > row_end {
                    continue;
                }
                for x in row_start..=row_end {
                    if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
                        let s = cell.style().add_modifier(Modifier::REVERSED);
                        cell.set_style(s);
                    }
                }
            }
        }
    }
}

/// One rendered row of the collapsed tool tile, plus its plain-text
/// projection for copy-on-drag. The tile is always exactly one row;
/// `tall` is reserved for a future multi-row variant (e.g. an error
/// preview line) and currently always false.
struct CollapsedTile {
    line: Line<'static>,
    text: String,
    #[allow(dead_code)]
    tall: bool,
}

/// Build the one-line collapsed tile for a tool message. Modern
/// single-icon design — every tool gets a ⚡ bolt; the verb carries
/// the differentiation. Layout:
///
///   idle:     `  ▎ ⚡  read   src/main.rs · 312 lines            ⌄`
///   running:  `  ▎ ⚡  read   src/main.rs · ▰▰▱▱▱ 3.2s`
///   error:    `  ▎ ⚡  read   src/main.rs · ✕ failed`
///
/// Visual hierarchy:
///   - ⚡ bolt: peach (running, pulses DIM on off-beat) / sky (idle) /
///     red (error). The pulse is the animation hook — a frozen glyph
///     reads as "stuck", a breathing one reads as "alive".
///   - verb: bold, matches the bolt color.
///   - args: FG_DIM, the quietest layer.
///   - chevron: FG_DIMMER, sits behind the affordance.
///
/// While in flight, the trailing chevron is replaced by the progress
/// bar — there's nothing to expand yet, and the bar carries liveness
/// on its own.
fn render_collapsed_tile(msg: &ChatMessage, idx: usize, app: &App, inner_w: u16) -> CollapsedTile {
    let is_active = app.active_tool_msg_idx == Some(idx);
    let is_error = msg.content.trim_start().starts_with("error:");
    let tool_name = msg.name.as_deref().unwrap_or("");
    let args = msg.tool_args.as_ref();

    // Arg summary — fall back to the placeholder content if we
    // somehow lost the structured args (e.g. session reload from
    // before this field existed).
    let arg_text = match args {
        Some(v) => tool_arg_summary(tool_name, v),
        None => msg.content.lines().next().unwrap_or("").to_string(),
    };

    let fg_main = if is_error {
        theme::color::TOOL_ERROR
    } else if is_active {
        theme::color::STATE_TOOLING
    } else {
        theme::color::TOOL
    };
    let fg_dim = if is_active {
        theme::color::FG
    } else {
        theme::color::FG_DIM
    };

    // Build the spans. Layout: indent + gutter + bolt + verb + args
    // + (progress | diff | chevron).
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(10);
    spans.push(Span::styled("  ".to_string(), Style::default()));
    let gutter_style = Style::default().fg(theme::color::FG_DIMMER);
    spans.push(Span::styled("▎ ".to_string(), gutter_style));

    // Lightning bolt — single glyph for the whole tool surface. While
    // the tool is active, the bolt breathes (DIM on the off-beat) so
    // a long-running shell read stays lively instead of looking stuck.
    let bolt_style = Style::default()
        .fg(fg_main)
        .add_modifier(Modifier::BOLD)
        .add_modifier(if is_active {
            pulse_modifier(app.anim_tick, 4)
        } else {
            Modifier::empty()
        });
    spans.push(Span::styled("⚡ ".to_string(), bolt_style));

    // Verb — bold, matches the bolt color so the verb+bolt read as
    // one unit ("⚡ read").
    spans.push(Span::styled(
        format!("{}  ", tool_label(tool_name)),
        Style::default().fg(fg_main).add_modifier(Modifier::BOLD),
    ));

    // Args — truncate to fit so the trailing progress / chevron never
    // gets pushed off-row. Budget leaves room for the chevron (2) and
    // a separator gap (3) on top of the prefix we've already spent.
    let spent: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let arg_budget = (inner_w as usize).saturating_sub(spent + 2 + 3).max(8);
    let arg_text = if arg_text.chars().count() > arg_budget {
        let cut: String = arg_text
            .chars()
            .take(arg_budget.saturating_sub(1))
            .collect();
        format!("{cut}…")
    } else {
        arg_text
    };
    if !arg_text.is_empty() {
        spans.push(Span::styled(arg_text, Style::default().fg(fg_dim)));
    }

    if is_active {
        // In-flight: trailing progress bar instead of chevron. The
        // bar carries liveness on its own (fills over time), and the
        // tile isn't expandable yet anyway.
        let elapsed = app
            .active_tool_started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        spans.push(Span::styled(
            format!("   {}", progress_bar(elapsed)),
            Style::default().fg(fg_main),
        ));
    } else if is_error {
        spans.push(Span::styled(
            "   ✕ failed".to_string(),
            Style::default().fg(theme::color::TOOL_ERROR),
        ));
    } else {
        // Diff hint: if a write/edit was authorised, show +N -M so the
        // user can see the size of the change at a glance. Counts
        // derived from msg.diff (already populated in resolve_confirm).
        let mut diff_hint: Option<(usize, usize)> = None;
        if let Some(diff) = msg.diff.as_ref() {
            if !diff.is_empty() {
                let mut adds = 0usize;
                let mut dels = 0usize;
                for line in diff {
                    match line.kind {
                        crate::tools::DiffLineKind::Added => adds += 1,
                        crate::tools::DiffLineKind::Removed => dels += 1,
                        crate::tools::DiffLineKind::Context
                        | crate::tools::DiffLineKind::Summary => {}
                    }
                }
                if adds > 0 || dels > 0 {
                    diff_hint = Some((adds, dels));
                }
            }
        }
        if let Some((adds, dels)) = diff_hint {
            spans.push(Span::styled(
                format!("   +{adds} -{dels}"),
                Style::default().fg(theme::color::FG_DIM),
            ));
        }
        // Chevron — "click to expand". Sits at the visual tail so the
        // affordance reads as "more behind this".
        spans.push(Span::styled(
            "   ⌄".to_string(),
            Style::default().fg(theme::color::FG_DIMMER),
        ));
    }

    let line = Line::from(spans);

    // Plain-text projection for copy-on-drag — strip styles, keep chars.
    let mut text = String::with_capacity(inner_w as usize);
    for s in &line.spans {
        text.push_str(s.content.as_ref());
    }

    CollapsedTile {
        line,
        text,
        tall: false,
    }
}

/// Collapsed tile for a multi-tool group. Modern single-icon design —
/// the bolt count in the prefix carries the batch size visually.
///
///   idle:      `  ▎ ⚡⚡⚡  3 reads   src/main.rs, src/lib.rs, …   ⌄`
///   in flight: `  ▎ ⚡⚡⚡  reading files   ▰▰▰▱▱ 2/3 …`
///   error:     `  ▎ ⚡⚡⚡  3 reads   ✕ 1 failed · …`
///
/// The bolts pulse together while the batch is in flight; the count
/// badge in the header (`2/3`) animates a subtle width-jitter via
/// `breathing_count` so the user sees liveness even when no individual
/// tool is currently running (the gap between `ToolResult` and the
/// next `ToolStart`).
fn render_grouped_tile(
    messages: &[ChatMessage],
    group: ToolGroup,
    app: &App,
    inner_w: u16,
) -> CollapsedTile {
    let summary = summarize_group(messages, group);
    let progress = group_progress(messages, group, app.active_tool_msg_idx);
    let count = group.count();
    // The batch is "pending" while either a single tool is mid-flight
    // OR the agent turn is still generating and the planned total
    // hasn't fully landed yet. The latter covers the brief gap between
    // one tool's `ToolResult` and the next tool's `ToolStart`, where
    // `active_tool_msg_idx` is briefly `None` but the batch isn't done.
    let batch_pending =
        progress.in_flight || (group.count() < progress.total && app.turn.is_generating());

    let all_errors = summary.errors == count;
    let some_errors = summary.errors > 0 && !all_errors;

    let fg_main = if batch_pending {
        theme::color::STATE_TOOLING
    } else if all_errors {
        theme::color::TOOL_ERROR
    } else if some_errors {
        theme::color::WARNING
    } else {
        theme::color::TOOL
    };
    let fg_dim = if batch_pending {
        theme::color::FG
    } else {
        theme::color::FG_DIM
    };

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(10);
    spans.push(Span::styled("  ".to_string(), Style::default()));
    let gutter_style = Style::default().fg(theme::color::FG_DIMMER);
    spans.push(Span::styled("▎ ".to_string(), gutter_style));

    // Bolt-stack prefix: render up to 3 bolts inline (reads as "a
    // small batch"), then switch to `⚡×N` so a 10-file batch doesn't
    // spend ten chars on glyphs alone. While the batch is in flight,
    // the whole stack breathes (DIM on the off-beat) — same animation
    // language as the single-tile bolt.
    let bolt_stack: String = if count <= 3 {
        "⚡".repeat(count)
    } else {
        format!("⚡×{count}")
    };
    let bolt_style = Style::default()
        .fg(fg_main)
        .add_modifier(Modifier::BOLD)
        .add_modifier(if batch_pending {
            pulse_modifier(app.anim_tick, 4)
        } else {
            Modifier::empty()
        });
    spans.push(Span::styled(format!("{bolt_stack}  "), bolt_style));

    // Header — uniform batches ("3 reads") unify under one verb,
    // mixed batches fall back to a per-verb tally ("3 calls · read 2 · list 1").
    // In-flight uniform batches switch to the gerund + animated
    // ellipsis ("reading files …") since the count is already in the
    // trailing progress bar.
    let header_text = if summary.uniform {
        let noun = group_noun(&summary.canonical);
        let verb = tool_label(&summary.canonical);
        if batch_pending {
            let gerund = gerund_verb(&summary.canonical);
            let dots = animated_ellipsis(app.anim_tick, 3);
            format!("{gerund} {noun} {dots}")
        } else {
            format!("{count} {verb} {noun}")
        }
    } else {
        let breakdown: String = summary
            .verb_counts
            .iter()
            .map(|(n, c)| format!("{} {c}", tool_label(n)))
            .collect::<Vec<_>>()
            .join(" · ");
        if batch_pending {
            format!("{count} calls · {breakdown}")
        } else {
            format!("{count} calls · {breakdown}")
        }
    };
    spans.push(Span::styled(
        header_text,
        Style::default().fg(fg_main).add_modifier(Modifier::BOLD),
    ));

    // Trailing region — one of: progress bar (in flight), error count,
    // diff totals + chevron (idle). Always computed last so we can
    // budget the per-tool arg list against what's left.
    let trailing = if batch_pending {
        let elapsed = app
            .active_tool_started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let bar = group_progress_bar(progress, elapsed);
        let badge = breathing_count(progress.completed, progress.total, app.anim_tick);
        format!("   {bar} {badge}")
    } else if all_errors {
        "   ✕ all failed".to_string()
    } else if some_errors {
        format!("   ✕ {} failed", summary.errors)
    } else {
        String::new()
    };
    let trailing_is_chevron = trailing.is_empty();

    // Per-tool arg list — only on the idle path (the in-flight path
    // already spends its trailing budget on the progress bar).
    let arg_text = if !summary.args.is_empty() && trailing_is_chevron {
        let joined = summary.args.join(", ");
        Some(joined)
    } else {
        None
    };

    // Budget the arg list against what's left of inner_w.
    if let Some(joined) = arg_text {
        let spent: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        // Reserve: 3 (separator) + 6 (diff hint or chevron) + 1 (ellipsis).
        let budget = (inner_w as usize).saturating_sub(spent + 3 + 6 + 1).max(8);
        let trimmed = if joined.chars().count() > budget {
            let cut: String = joined.chars().take(budget.saturating_sub(1)).collect();
            format!("{cut}…")
        } else {
            joined
        };
        spans.push(Span::styled(
            format!("   {trimmed}"),
            Style::default().fg(fg_dim),
        ));
    }

    // Diff totals for write/edit batches — same affordance as the
    // single-tile path: `+N -M` at a glance.
    if trailing_is_chevron && (summary.total_adds > 0 || summary.total_dels > 0) {
        spans.push(Span::styled(
            format!("   +{} -{}", summary.total_adds, summary.total_dels),
            Style::default().fg(theme::color::FG_DIM),
        ));
    }

    if !trailing.is_empty() {
        spans.push(Span::styled(
            trailing,
            Style::default().fg(if batch_pending { fg_main } else { fg_dim }),
        ));
    } else {
        // Chevron — "click to expand".
        spans.push(Span::styled(
            "   ⌄".to_string(),
            Style::default().fg(theme::color::FG_DIMMER),
        ));
    }

    let line = Line::from(spans);
    let mut text = String::with_capacity(inner_w as usize);
    for s in &line.spans {
        text.push_str(s.content.as_ref());
    }
    CollapsedTile {
        line,
        text,
        tall: false,
    }
}

/// Group header shown at the top of an expanded multi-tool group.
/// Mirrors the collapsed tile's visual language (bolts + bold header)
/// so the expand/collapse transition reads as a state change rather
/// than a redesign:
///
///   `  ▎ ⚡⚡⚡  ▼ 3 reads  · expanded                      ⌃`
///
/// The downward-chevron (`▼`) marks the expanded affordance — visual
/// mirror of the collapsed tile's trailing `⌄`. Click toggles
/// `expanded_tool_groups.contains(&start)` → collapses back.
fn render_group_header(
    messages: &[ChatMessage],
    group: ToolGroup,
    app: &App,
    inner_w: u16,
) -> CollapsedTile {
    let summary = summarize_group(messages, group);
    let progress = group_progress(messages, group, app.active_tool_msg_idx);
    let count = group.count();
    let batch_pending =
        progress.in_flight || (group.count() < progress.total && app.turn.is_generating());

    let fg_main = if batch_pending {
        theme::color::STATE_TOOLING
    } else if summary.errors == count {
        theme::color::TOOL_ERROR
    } else if summary.errors > 0 {
        theme::color::WARNING
    } else {
        theme::color::TOOL
    };

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(8);
    spans.push(Span::styled("  ".to_string(), Style::default()));
    let gutter_style = Style::default().fg(theme::color::FG_DIMMER);
    spans.push(Span::styled("▎ ".to_string(), gutter_style));

    // Same bolt-stack prefix as the collapsed tile — the expansion
    // shouldn't redesign the row, just open it up.
    let bolt_stack: String = if count <= 3 {
        "⚡".repeat(count)
    } else {
        format!("⚡×{count}")
    };
    let bolt_style = Style::default()
        .fg(fg_main)
        .add_modifier(Modifier::BOLD)
        .add_modifier(if batch_pending {
            pulse_modifier(app.anim_tick, 4)
        } else {
            Modifier::empty()
        });
    spans.push(Span::styled(format!("{bolt_stack}  "), bolt_style));

    // Down-chevron marks the expanded affordance — bold so it reads
    // as the clickable handle for the whole batch.
    spans.push(Span::styled(
        "▼ ".to_string(),
        Style::default().fg(fg_main).add_modifier(Modifier::BOLD),
    ));

    let header_text = if summary.uniform {
        let noun = group_noun(&summary.canonical);
        let verb = tool_label(&summary.canonical);
        format!("{count} {verb} {noun}")
    } else {
        format!("{count} calls")
    };
    spans.push(Span::styled(
        header_text,
        Style::default().fg(fg_main).add_modifier(Modifier::BOLD),
    ));

    // "expanded" tag — italic, dimmer than the header so it reads as
    // metadata rather than part of the verb phrase.
    spans.push(Span::styled(
        "   expanded".to_string(),
        Style::default()
            .fg(theme::color::FG_DIM)
            .add_modifier(Modifier::ITALIC),
    ));

    if batch_pending {
        let elapsed = app
            .active_tool_started_at
            .map(|t| t.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        let bar = group_progress_bar(progress, elapsed);
        let badge = breathing_count(progress.completed, progress.total, app.anim_tick);
        spans.push(Span::styled(
            format!("   {bar} {badge}"),
            Style::default().fg(fg_main),
        ));
    }

    // Trailing collapse affordance.
    spans.push(Span::styled(
        "   ⌃".to_string(),
        Style::default().fg(theme::color::FG_DIMMER),
    ));

    let line = Line::from(spans);
    let mut text = String::with_capacity(inner_w as usize);
    for s in &line.spans {
        text.push_str(s.content.as_ref());
    }
    let _ = inner_w;
    CollapsedTile {
        line,
        text,
        tall: false,
    }
}

/// Five-segment progress bar that fills by `completed/total` ratio,
/// floored by the wall-clock `elapsed_secs` fill. The ratio takes
/// priority so a 5-tool batch reads as 1/5, 2/5, ... while it streams
/// in — much more informative than time alone. Time still nudges the
/// bar forward when `completed == 0` (first tool still running) so the
/// tile doesn't sit at empty for the first 6 seconds of a slow call.
fn group_progress_bar(progress: GroupProgress, elapsed_secs: f32) -> String {
    const SEGMENTS: usize = 5;
    let ratio_fill = if progress.total == 0 {
        0.0
    } else {
        progress.completed as f32 / progress.total as f32
    };
    let time_fill = (elapsed_secs / 6.0).clamp(0.0, 1.0);
    let fill_ratio = ratio_fill.max(time_fill);
    let filled = (fill_ratio * SEGMENTS as f32)
        .round()
        .clamp(0.0, SEGMENTS as f32) as usize;
    let bar: String = (0..SEGMENTS)
        .map(|i| if i < filled { '▰' } else { '▱' })
        .collect();
    format!("{bar} {}/{}", progress.completed, progress.total)
}

/// Gerund form of a tool's verb for the in-flight header (`reading`
/// rather than `read 3 files`). Only consulted for uniform batches
/// where the count is already shown in the progress bar prefix.
fn gerund_verb(canonical: &str) -> &'static str {
    match canonical {
        "read_file" => "reading",
        "list_dir" => "listing",
        "find_files" => "finding",
        "edit_file" | "multi_edit" | "apply_patch" | "move_lines" | "delete_lines"
        | "insert_at" => "editing",
        "write_file" => "writing",
        "run_command" => "running",
        "save_memory" => "saving",
        "read_memory" => "recalling",
        "forget_memory" => "forgetting",
        "git_status" | "git_log" | "git_diff" | "git_show" => "querying git",
        "consult_specialist" => "consulting",
        "web_search" => "searching",
        "web_fetch" => "fetching",
        _ => "running",
    }
}

/// Render one inline permission card. Returns the visual `Line`s that
/// the chat renderer splices into its scroll buffer at the message's
/// position, plus the relative position of the Approve/Deny button row
/// so the caller can compute absolute hit-test rects.
///
///   ┌─ ⚠ CONFIRM ─────────────────────────┐
///   │ tool_head                             │
///   │ prompt line 1                         │
///   │ prompt line 2                         │
///   │   + added line                        │
///   │   - removed line                      │
///   │   …                                   │
///   │ [ ✓ approve ]   [ ✕ deny ]            │
///   │ (y) approve · (n) deny · click button │
///   └───────────────────────────────────────┘
///
/// Approve / Deny buttons are mouse-clickable. The caller (render_chat)
/// writes the absolute hit rects into `app.render.permission_btn_*`
/// so the mouse handler in `event.rs` can route clicks back to
/// `App::resolve_confirm`. y/n still work via `handle_confirm` — no
/// mouse required.
struct PermissionCard {
    lines: Vec<Line<'static>>,
    /// Logical-line index (within the returned `lines`) of the row
    /// containing the Approve/Deny buttons.
    button_row: u16,
    /// Column (relative to the chat inner area's left edge) where the
    /// Approve button starts.
    approve_col: u16,
    /// Column where the Deny button starts.
    deny_col: u16,
    /// Width of each button cell.
    btn_w: u16,
}

fn render_permission_card(msg: &crate::ollama::ChatMessage, width: u16) -> PermissionCard {
    use crate::tools::DiffLineKind;
    use crate::ui::theme::color as c;
    use ratatui::text::Span;

    let mut out: Vec<Line<'static>> = Vec::new();

    // The permission card uses a slightly different palette from the
    // rest of the chat — red border for destructive, yellow for
    // info-style prompts. Destructive = anything that ends in `:`
    // and starts with `run_command`, `write_file`, `edit_file`,
    // `forget_memory`, `SAVE memory`, `UPDATE memory`, `FORGET memory`.
    let head = msg.name.as_deref().unwrap_or("permission");
    let destructive = matches!(
        head,
        "run_command"
            | "write_file"
            | "edit_file"
            | "multi_edit"
            | "patch"
            | "delete_file"
            | "forget_memory"
            | "SAVE"
            | "UPDATE"
            | "FORGET"
    );
    let accent = if destructive {
        c::PERMISSION_DESTRUCTIVE
    } else {
        c::PERMISSION_INFO
    };
    let bg = if destructive {
        c::PERMISSION_DESTRUCTIVE_BG
    } else {
        c::BG_CARD
    };

    // Card width — leave 2 cols of breathing room on each side so the
    // border doesn't touch the chat panel border.
    let card_w = width.saturating_sub(4).max(20) as usize;
    let inner_w = card_w.saturating_sub(4); // 2 cols of horizontal pad inside border

    // ── Top border ──────────────────────────────────────────────────
    let top_title = format!(" ⚠ CONFIRM · {head} ");
    let title_len = top_title.chars().count();
    let top_dash = card_w.saturating_sub(title_len + 2);
    let top_left = "┌─";
    let top_right = "─┐";
    let top_dashes = "─".repeat(top_dash);
    out.push(Line::from(vec![
        Span::styled(top_left, Style::default().fg(accent)),
        Span::styled(
            top_title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(top_dashes, Style::default().fg(accent)),
        Span::styled(top_right, Style::default().fg(accent)),
    ]));

    // ── Prompt body ─────────────────────────────────────────────────
    // Wrap prompt text into inner_w-wide lines so a long prompt
    // doesn't blow past the card edge.
    let prompt_pad_l = "│  ";
    let prompt_pad_r = "  │";
    for paragraph in msg.content.split('\n') {
        if paragraph.is_empty() {
            // Empty row inside the card — keep the bg continuous.
            let pad = " ".repeat(inner_w);
            out.push(Line::from(vec![
                Span::styled(prompt_pad_l, Style::default().fg(accent).bg(bg)),
                Span::styled(pad, Style::default().fg(c::FG).bg(bg)),
                Span::styled(prompt_pad_r, Style::default().fg(accent).bg(bg)),
            ]));
            continue;
        }
        let segments = crate::ui::markdown::parse_inline_md(paragraph, Style::default().fg(c::FG));
        for chunk in crate::ui::markdown::wrap_styled_segments(segments, inner_w) {
            let used: usize = chunk.iter().map(|s| s.content.chars().count()).sum();
            let pad = " ".repeat(inner_w.saturating_sub(used));
            let mut spans = vec![Span::styled(
                prompt_pad_l,
                Style::default().fg(accent).bg(bg),
            )];
            for s in chunk {
                let mut styled = s.style;
                styled = styled.bg(bg);
                spans.push(Span::styled(s.content.to_string(), styled));
            }
            spans.push(Span::styled(pad, Style::default().bg(bg)));
            spans.push(Span::styled(
                prompt_pad_r,
                Style::default().fg(accent).bg(bg),
            ));
            out.push(Line::from(spans));
        }
    }

    // ── Diff body ───────────────────────────────────────────────────
    if let Some(diff) = msg.diff.as_ref() {
        // Blank separator before the diff so the body doesn't fuse
        // with the prompt header.
        let pad = " ".repeat(inner_w);
        out.push(Line::from(vec![
            Span::styled(prompt_pad_l, Style::default().fg(accent).bg(bg)),
            Span::styled(pad.clone(), Style::default().bg(bg)),
            Span::styled(prompt_pad_r, Style::default().fg(accent).bg(bg)),
        ]));
        for dl in diff {
            let fg = match dl.kind {
                DiffLineKind::Added => c::DIFF_ADD,
                DiffLineKind::Removed => c::DIFF_REMOVE,
                DiffLineKind::Context => c::FG_DIM,
                DiffLineKind::Summary => c::WARNING,
            };
            // Wrap long diff lines so they don't overflow the card.
            let segs = vec![(dl.text.clone(), Style::default().fg(fg))];
            for chunk in crate::ui::markdown::wrap_styled_segments(segs, inner_w) {
                let used: usize = chunk.iter().map(|s| s.content.chars().count()).sum();
                let pad = " ".repeat(inner_w.saturating_sub(used));
                let mut spans = vec![Span::styled(
                    prompt_pad_l,
                    Style::default().fg(accent).bg(bg),
                )];
                for s in chunk {
                    let mut styled = s.style;
                    styled = styled.bg(bg);
                    spans.push(Span::styled(s.content.to_string(), styled));
                }
                spans.push(Span::styled(pad, Style::default().bg(bg)));
                spans.push(Span::styled(
                    prompt_pad_r,
                    Style::default().fg(accent).bg(bg),
                ));
                out.push(Line::from(spans));
            }
        }
    }

    // ── Spacer ──────────────────────────────────────────────────────
    let pad = " ".repeat(inner_w);
    out.push(Line::from(vec![
        Span::styled(prompt_pad_l, Style::default().fg(accent).bg(bg)),
        Span::styled(pad.clone(), Style::default().bg(bg)),
        Span::styled(prompt_pad_r, Style::default().fg(accent).bg(bg)),
    ]));

    // ── Buttons ─────────────────────────────────────────────────────
    // [ ✓ approve ]   [ ✕ deny ] — clickable, recorded into
    // app.render.permission_btn_rects with the absolute (x, y, w, h)
    // hit rect so the mouse handler can resolve them.
    let btn_approve = "[ ✓ approve ]";
    let btn_deny = "[ ✕ deny ]";
    let btn_row = format!("{btn_approve}   {btn_deny}");
    // The visual row, padded to inner_w so the bg stays solid.
    let used = btn_row.chars().count();
    let pad = " ".repeat(inner_w.saturating_sub(used));
    // The visual button row uses two padded cells separated by a small
    // gap. Track the start column of each relative to the *chat inner*
    // left edge — the caller (render_chat) adds `inner.x + 2` to get
    // the absolute screen x. The `+2` accounts for the 2 cols of
    // `│  ` left padding inside the card border.
    let approve_start_col: u16 = 0;
    let deny_start_col: u16 = (btn_approve.chars().count() as u16) + 3; // 3 = "   " gap
    let button_row_index = out.len() as u16;

    out.push(Line::from(vec![
        Span::styled(prompt_pad_l, Style::default().fg(accent).bg(bg)),
        Span::styled(
            btn_approve,
            Style::default()
                .fg(c::SUCCESS)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("   ", Style::default().bg(bg)),
        Span::styled(
            btn_deny,
            Style::default()
                .fg(c::ERROR)
                .bg(bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(pad, Style::default().bg(bg)),
        Span::styled(prompt_pad_r, Style::default().fg(accent).bg(bg)),
    ]));

    // Spacer + hint line
    let pad = " ".repeat(inner_w);
    out.push(Line::from(vec![
        Span::styled(prompt_pad_l, Style::default().fg(accent).bg(bg)),
        Span::styled(pad.clone(), Style::default().bg(bg)),
        Span::styled(prompt_pad_r, Style::default().fg(accent).bg(bg)),
    ]));
    let hint = "(y) approve · (n) deny · click button";
    let pad = " ".repeat(inner_w.saturating_sub(hint.chars().count()));
    out.push(Line::from(vec![
        Span::styled(prompt_pad_l, Style::default().fg(accent).bg(bg)),
        Span::styled(
            hint,
            Style::default()
                .fg(c::FG_DIM)
                .bg(bg)
                .add_modifier(Modifier::ITALIC),
        ),
        Span::styled(pad, Style::default().bg(bg)),
        Span::styled(prompt_pad_r, Style::default().fg(accent).bg(bg)),
    ]));

    // ── Bottom border ───────────────────────────────────────────────
    let bottom = format!("└{}┘", "─".repeat(card_w.saturating_sub(2)));
    out.push(Line::from(Span::styled(
        bottom,
        Style::default().fg(accent),
    )));

    PermissionCard {
        lines: out,
        button_row: button_row_index,
        approve_col: approve_start_col,
        deny_col: deny_start_col,
        btn_w: btn_approve.chars().count() as u16,
    }
}
