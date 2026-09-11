use std::ops::Range;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub struct Code {
    pub text: String,
    tokens: Vec<Token>,
}

#[derive(Clone, Debug)]
struct Token {
    spans: Vec<Span<'static>>,
    width: usize,
    whitespace: bool,
}

impl Code {
    pub fn indent(&self) -> usize {
        self.text.bytes().take_while(|&ch| ch == b' ').count()
    }

    pub fn emphasize(&mut self, ranges: &[Range<usize>], style: Style) {
        let mut offset = 0;
        for token in &mut self.tokens {
            let mut spans = Vec::new();
            for span in &token.spans {
                let end = offset + span.content.len();
                let mut start = offset;
                for range in ranges
                    .iter()
                    .filter(|range| range.start < end && range.end > offset)
                {
                    let from = range.start.max(start);
                    let to = range.end.min(end);
                    if start < from {
                        spans.push(Span::styled(
                            span.content[start - offset..from - offset].to_owned(),
                            span.style,
                        ));
                    }
                    spans.push(Span::styled(
                        span.content[from - offset..to - offset].to_owned(),
                        span.style.patch(style),
                    ));
                    start = to;
                }
                if start < end {
                    spans.push(Span::styled(
                        span.content[start - offset..].to_owned(),
                        span.style,
                    ));
                }
                offset = end;
            }
            token.spans = spans;
        }
    }

    pub fn review(&self, width: usize, trim: usize, guides: bool) -> Vec<Line<'static>> {
        let trim = trim.min(self.indent());
        let mut skip = trim;
        let spans = self
            .tokens
            .iter()
            .flat_map(|token| &token.spans)
            .filter_map(|span| {
                let cut = skip.min(span.content.len());
                skip -= cut;
                (cut < span.content.len())
                    .then(|| Span::styled(span.content[cut..].to_owned(), span.style))
            })
            .collect();
        let mut rows = Self::styled(spans).wrap(width);
        if guides && let Some(first) = rows.first_mut() {
            let mut spans = Vec::new();
            let mut column = trim;
            let mut leading = true;
            for span in &first.spans {
                let spaces = if leading {
                    span.content.bytes().take_while(|&ch| ch == b' ').count()
                } else {
                    0
                };
                for _ in 0..spaces {
                    let guide = column > 0 && column.is_multiple_of(4);
                    spans.push(Span::styled(
                        if guide { "│" } else { " " },
                        if guide {
                            span.style.fg(Color::DarkGray)
                        } else {
                            span.style
                        },
                    ));
                    column += 1;
                }
                if spaces < span.content.len() {
                    leading = false;
                    spans.push(Span::styled(span.content[spaces..].to_owned(), span.style));
                }
            }
            first.spans = spans;
        }
        rows
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self::styled(vec![Span::raw(text.into())])
    }

    pub fn styled(spans: Vec<Span<'static>>) -> Self {
        let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
        let mut offset = 0;
        let pieces: Vec<_> = spans
            .iter()
            .map(|span| {
                let range = offset..offset + span.content.len();
                offset = range.end;
                (range, span)
            })
            .collect();
        let tokens = token_ranges(&text)
            .into_iter()
            .map(|range| {
                let start = pieces.partition_point(|(piece, _)| piece.end <= range.start);
                let spans = pieces[start..]
                    .iter()
                    .take_while(|(piece, _)| piece.start < range.end)
                    .map(|(piece, span)| {
                        let start = range.start.max(piece.start) - piece.start;
                        let end = range.end.min(piece.end) - piece.start;
                        Span::styled(span.content[start..end].to_owned(), span.style)
                    })
                    .collect();
                Token {
                    spans,
                    width: text[range.clone()].width(),
                    whitespace: text[range].chars().all(char::is_whitespace),
                }
            })
            .collect();
        Self { text, tokens }
    }

    pub fn wrap(&self, width: usize) -> Vec<Line<'static>> {
        let width = width.max(1);
        let mut lines = Vec::new();
        let mut spans = Vec::new();
        let mut used = 0;
        let mut has_code = false;
        for token in &self.tokens {
            if !token.whitespace && has_code && used + token.width > width {
                lines.push(Line::from(std::mem::take(&mut spans)));
                used = 0;
                has_code = false;
            }
            spans.extend(token.spans.iter().cloned());
            used += token.width;
            has_code |= !token.whitespace;
        }
        if !spans.is_empty() || lines.is_empty() {
            lines.push(Line::from(spans));
        }
        lines
    }
}

// Lexical boundaries are independent of theme colors: a string may have several colors
// and consecutive identifiers may share one color. Never split a grapheme or lexeme.
pub(crate) fn token_ranges(text: &str) -> Vec<Range<usize>> {
    let graphemes: Vec<_> = text.grapheme_indices(true).collect();
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < graphemes.len() {
        let start = i;
        let current = graphemes[i].1;
        if matches!(current, "\"" | "'" | "`") {
            let triple = i + 2 < graphemes.len()
                && graphemes[i + 1].1 == current
                && graphemes[i + 2].1 == current;
            let delimiter = if triple { 3 } else { 1 };
            let mut end = i + delimiter;
            let mut closed = false;
            while end < graphemes.len() {
                if graphemes[end].1 == "\\" {
                    end += 2;
                    continue;
                }
                if (0..delimiter).all(|n| {
                    graphemes
                        .get(end + n)
                        .is_some_and(|(_, value)| *value == current)
                }) {
                    end += delimiter;
                    closed = true;
                    break;
                }
                end += 1;
            }
            i = if closed || current != "'" {
                end.min(graphemes.len())
            } else {
                i + 1
            };
        } else if current.chars().all(char::is_whitespace) {
            i += 1;
            while i < graphemes.len() && graphemes[i].1.chars().all(char::is_whitespace) {
                i += 1;
            }
        } else if current.chars().all(|ch| ch.is_ascii_digit()) {
            i += 1;
            while i < graphemes.len() {
                let next = graphemes[i].1;
                let exponent_sign = matches!(next, "+" | "-")
                    && matches!(graphemes[i - 1].1, "e" | "E" | "p" | "P");
                let decimal = next == "."
                    && graphemes
                        .get(i + 1)
                        .is_some_and(|(_, next)| next.chars().all(|ch| ch.is_ascii_digit()));
                if identifier(next) || exponent_sign || decimal {
                    i += 1;
                } else {
                    break;
                }
            }
        } else if identifier(current) {
            i += 1;
            while i < graphemes.len() && identifier(graphemes[i].1) {
                i += 1;
            }
        } else if operator(current) {
            i += 1;
            while i < graphemes.len() && operator(graphemes[i].1) {
                i += 1;
            }
        } else {
            i += 1;
        }
        let end = graphemes
            .get(i)
            .map(|(offset, _)| *offset)
            .unwrap_or(text.len());
        ranges.push(graphemes[start].0..end);
    }
    ranges
}

fn identifier(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
}

fn operator(grapheme: &str) -> bool {
    grapheme.len() == 1 && "+-*/%=!<>:&|^?~.".contains(grapheme)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn wraps_tokens_independently_of_highlight_spans() {
        let source = "val paymentId = \"hello world\" ?: fallbackValue";
        let code = Code::styled(vec![
            Span::styled("val payment", Style::new().red()),
            Span::styled("Id = \"hello", Style::new().green()),
            Span::styled(" world\" ?: fallbackValue", Style::new().blue()),
        ]);
        let lines = code.wrap(12);
        assert_eq!(lines.iter().map(text).collect::<String>(), source);
        for token in ["paymentId", "\"hello world\"", "?:", "fallbackValue"] {
            assert!(
                lines.iter().any(|line| text(line).contains(token)),
                "{token}"
            );
        }
        assert!(
            lines
                .iter()
                .flat_map(|line| &line.spans)
                .any(|span| span.style.fg == Style::new().green().fg)
        );
    }

    #[test]
    fn preserves_unicode_graphemes_numbers_and_overlong_tokens() {
        let code = Code::plain("결제금액 cafe\u{301} 👩‍💻 12.25e-3 veryLongIdentifier");
        let lines = code.wrap(7);
        assert_eq!(lines.iter().map(text).collect::<String>(), code.text);
        for token in [
            "결제금액",
            "cafe\u{301}",
            "👩‍💻",
            "12.25e-3",
            "veryLongIdentifier",
        ] {
            assert!(
                lines.iter().any(|line| text(line).contains(token)),
                "{token}"
            );
        }
        assert!(lines.iter().any(|line| line.width() > 7));
        assert_eq!(Code::plain("").wrap(0).len(), 1);
    }
}
