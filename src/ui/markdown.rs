//! Inline markdown parser + word-wrap.
//!
//! Used by chat (every assistant/user line) and by popups (add-model body,
//! confirm prompt header). Pure functions, no state.

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

/// Minimal inline-markdown parser: handles `**bold**` and `` `code` ``.
/// Returns a list of (text, style) segments preserving inline formatting.
pub(super) fn parse_inline_md(text: &str, base: Style) -> Vec<(String, Style)> {
    let bold_style = base.add_modifier(Modifier::BOLD);
    let code_style = Style::default()
        .fg(Color::LightYellow)
        .add_modifier(Modifier::DIM);

    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<(String, Style)> = Vec::new();
    let mut buf = String::new();
    let mut i = 0;

    while i < chars.len() {
        // **bold**
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            let mut j = i + 2;
            let mut found = None;
            while j + 1 < chars.len() {
                if chars[j] == '*' && chars[j + 1] == '*' {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                if end > i + 2 {
                    if !buf.is_empty() {
                        out.push((std::mem::take(&mut buf), base));
                    }
                    let inner: String = chars[i + 2..end].iter().collect();
                    out.push((inner, bold_style));
                    i = end + 2;
                    continue;
                }
            }
        }
        // `code`
        if chars[i] == '`' {
            let mut j = i + 1;
            let mut found = None;
            while j < chars.len() {
                if chars[j] == '`' {
                    found = Some(j);
                    break;
                }
                j += 1;
            }
            if let Some(end) = found {
                if end > i + 1 {
                    if !buf.is_empty() {
                        out.push((std::mem::take(&mut buf), base));
                    }
                    let inner: String = chars[i + 1..end].iter().collect();
                    out.push((inner, code_style));
                    i = end + 1;
                    continue;
                }
            }
        }
        buf.push(chars[i]);
        i += 1;
    }
    if !buf.is_empty() {
        out.push((buf, base));
    }
    out
}

/// Wrap a list of styled segments to fit `width` columns, preserving styles
/// across line breaks. Word-splits at whitespace; words longer than width
/// overflow rather than being broken mid-word.
pub(super) fn wrap_styled_segments(
    segments: Vec<(String, Style)>,
    width: usize,
) -> Vec<Vec<Span<'static>>> {
    enum Tok {
        Word(String, Style),
        Ws(String, Style),
    }
    let mut tokens: Vec<Tok> = Vec::new();
    for (text, style) in segments {
        let mut cur = String::new();
        let mut cur_ws: Option<bool> = None;
        for c in text.chars() {
            let is_ws = c.is_whitespace();
            if let Some(prev) = cur_ws {
                if prev != is_ws {
                    let chunk = std::mem::take(&mut cur);
                    if prev {
                        tokens.push(Tok::Ws(chunk, style));
                    } else {
                        tokens.push(Tok::Word(chunk, style));
                    }
                }
            }
            cur.push(c);
            cur_ws = Some(is_ws);
        }
        if let Some(ws) = cur_ws {
            if ws {
                tokens.push(Tok::Ws(cur, style));
            } else {
                tokens.push(Tok::Word(cur, style));
            }
        }
    }

    let width = width.max(1);
    let mut lines: Vec<Vec<Span<'static>>> = vec![vec![]];
    let mut col = 0usize;
    for tok in tokens {
        match tok {
            Tok::Ws(text, style) => {
                if col == 0 {
                    continue; // drop leading whitespace on a fresh line
                }
                let len = text.chars().count();
                if col + len > width {
                    lines.push(vec![]);
                    col = 0;
                    continue;
                }
                lines.last_mut().unwrap().push(Span::styled(text, style));
                col += len;
            }
            Tok::Word(text, style) => {
                let len = text.chars().count();
                if len > width {
                    // Single word longer than the wrap width — without
                    // intervention it would overflow the chat column
                    // and bleed into the adjacent inspector panel.
                    // Hard-break it at `width` chars (joined with no
                    // hyphen; this is rare content like long paths
                    // / URLs from tool output, not prose).
                    if col > 0 {
                        lines.push(vec![]);
                        col = 0;
                    }
                    let chars: Vec<char> = text.chars().collect();
                    for chunk in chars.chunks(width) {
                        let piece: String = chunk.iter().collect();
                        let piece_len = chunk.len();
                        if col > 0 {
                            lines.push(vec![]);
                            col = 0;
                        }
                        lines.last_mut().unwrap().push(Span::styled(piece, style));
                        col += piece_len;
                        if col >= width {
                            lines.push(vec![]);
                            col = 0;
                        }
                    }
                } else if col + len > width && col > 0 {
                    lines.push(vec![]);
                    col = 0;
                    lines.last_mut().unwrap().push(Span::styled(text, style));
                    col += len;
                } else {
                    lines.last_mut().unwrap().push(Span::styled(text, style));
                    col += len;
                }
            }
        }
    }

    if lines.last().is_some_and(|l| l.is_empty()) && lines.len() > 1 {
        lines.pop();
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::wrap_styled_segments;
    use ratatui::style::Style;

    #[test]
    fn single_long_word_is_hard_broken() {
        // 60-char word in a 20-col wrap width — must split across
        // multiple lines, never overflow a single line.
        let long = "a".repeat(60);
        let lines = wrap_styled_segments(
            vec![(long.clone(), Style::default())],
            20,
        );
        for line in &lines {
            assert!(
                line.iter().map(|s| s.content.chars().count()).sum::<usize>() <= 20,
                "overflowing line: {:?}",
                line,
            );
        }
        // Reassembled text == input (no chars lost, just split).
        let joined: String = lines
            .iter()
            .flat_map(|l| l.iter().map(|s| s.content.clone()))
            .collect();
        assert_eq!(joined, long);
    }

    #[test]
    fn normal_sentence_still_wraps_at_word_boundaries() {
        let lines = wrap_styled_segments(
            vec![("hello world this is a test".to_string(), Style::default())],
            10,
        );
        // Each line should be <= 10 chars and contain whole words.
        for line in &lines {
            assert!(line.iter().map(|s| s.content.chars().count()).sum::<usize>() <= 10);
        }
        // Reassembled content should contain all the input words.
        let joined: String = lines
            .iter()
            .flat_map(|l| l.iter().map(|s| s.content.clone()))
            .collect::<Vec<_>>()
            .join("");
        assert!(joined.contains("hello") && joined.contains("world") && joined.contains("test"));
    }
}
