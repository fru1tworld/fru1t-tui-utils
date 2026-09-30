use std::ops::Range;

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const INDENT_GUIDE_STEP: usize = 4;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum IndentGuides {
    #[default]
    Shown,
    Hidden,
}

impl IndentGuides {
    pub fn toggled(self) -> Self {
        match self {
            Self::Shown => Self::Hidden,
            Self::Hidden => Self::Shown,
        }
    }
}

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
        leading_spaces(&self.text)
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

    pub fn wrap_indented(
        &self,
        width: usize,
        trim: usize,
        guides: IndentGuides,
    ) -> Vec<Line<'static>> {
        let trim = trim.min(self.indent());
        let mut remaining_trim = trim;
        let spans = self
            .tokens
            .iter()
            .flat_map(|token| &token.spans)
            .filter_map(|span| {
                let cut = remaining_trim.min(span.content.len());
                remaining_trim -= cut;
                (cut < span.content.len())
                    .then(|| Span::styled(span.content[cut..].to_owned(), span.style))
            })
            .collect();
        let mut rows = Self::styled(spans).wrap(width);
        if guides == IndentGuides::Shown
            && let Some(first) = rows.first_mut()
        {
            draw_indent_guides(first, trim);
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
        let tokens = lexeme_ranges(&text)
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

fn leading_spaces(text: &str) -> usize {
    text.bytes().take_while(|&byte| byte == b' ').count()
}

fn draw_indent_guides(line: &mut Line<'static>, first_column: usize) {
    let mut spans = Vec::new();
    let mut column = first_column;
    let mut in_indent = true;
    for span in &line.spans {
        let spaces = if in_indent {
            leading_spaces(&span.content)
        } else {
            0
        };
        for _ in 0..spaces {
            let guide = column > 0 && column.is_multiple_of(INDENT_GUIDE_STEP);
            spans.push(if guide {
                Span::styled("│", span.style.fg(Color::DarkGray))
            } else {
                Span::styled(" ", span.style)
            });
            column += 1;
        }
        if spaces < span.content.len() {
            in_indent = false;
            spans.push(Span::styled(span.content[spaces..].to_owned(), span.style));
        }
    }
    line.spans = spans;
}

pub(crate) fn lexeme_ranges(text: &str) -> Vec<Range<usize>> {
    let graphemes: Vec<_> = text.grapheme_indices(true).collect();
    let mut ranges = Vec::new();
    let mut cursor = 0;
    while cursor < graphemes.len() {
        let start = cursor;
        let current = graphemes[start].1;
        cursor = if matches!(current, "\"" | "'" | "`") {
            match quoted_literal_end(&graphemes, start) {
                Some(end) => end,
                None if current == "'" => start + 1,
                None => graphemes.len(),
            }
        } else if is_whitespace(current) {
            advance_while(&graphemes, start + 1, is_whitespace)
        } else if is_digit(current) {
            number_end(&graphemes, start)
        } else if is_identifier(current) {
            advance_while(&graphemes, start + 1, is_identifier)
        } else if is_operator(current) {
            advance_while(&graphemes, start + 1, is_operator)
        } else {
            start + 1
        };
        let end = graphemes
            .get(cursor)
            .map_or(text.len(), |(offset, _)| *offset);
        ranges.push(graphemes[start].0..end);
    }
    ranges
}

fn quoted_literal_end(graphemes: &[(usize, &str)], start: usize) -> Option<usize> {
    let quote = graphemes[start].1;
    let is_quote_at = |index: usize| {
        graphemes
            .get(index)
            .is_some_and(|(_, value)| *value == quote)
    };
    let delimiter = if is_quote_at(start + 1) && is_quote_at(start + 2) {
        3
    } else {
        1
    };
    let mut cursor = start + delimiter;
    while cursor < graphemes.len() {
        if graphemes[cursor].1 == "\\" {
            cursor += 2;
        } else if (0..delimiter).all(|offset| is_quote_at(cursor + offset)) {
            return Some(cursor + delimiter);
        } else {
            cursor += 1;
        }
    }
    None
}

fn number_end(graphemes: &[(usize, &str)], start: usize) -> usize {
    let mut cursor = start + 1;
    while let Some(&(_, next)) = graphemes.get(cursor) {
        let exponent_sign =
            matches!(next, "+" | "-") && matches!(graphemes[cursor - 1].1, "e" | "E" | "p" | "P");
        let decimal_point = next == "."
            && graphemes
                .get(cursor + 1)
                .is_some_and(|(_, after)| is_digit(after));
        if is_identifier(next) || exponent_sign || decimal_point {
            cursor += 1;
        } else {
            break;
        }
    }
    cursor
}

fn advance_while(
    graphemes: &[(usize, &str)],
    mut cursor: usize,
    accepts: fn(&str) -> bool,
) -> usize {
    while graphemes
        .get(cursor)
        .is_some_and(|(_, grapheme)| accepts(grapheme))
    {
        cursor += 1;
    }
    cursor
}

fn is_whitespace(grapheme: &str) -> bool {
    grapheme.chars().all(char::is_whitespace)
}

fn is_digit(grapheme: &str) -> bool {
    grapheme.chars().all(|ch| ch.is_ascii_digit())
}

fn is_identifier(grapheme: &str) -> bool {
    grapheme
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
}

fn is_operator(grapheme: &str) -> bool {
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
        let source = "val requestId = \"hello world\" ?: fallbackValue";
        let code = Code::styled(vec![
            Span::styled("val request", Style::new().red()),
            Span::styled("Id = \"hello", Style::new().green()),
            Span::styled(" world\" ?: fallbackValue", Style::new().blue()),
        ]);
        let lines = code.wrap(12);
        assert_eq!(lines.iter().map(text).collect::<String>(), source);
        for token in ["requestId", "\"hello world\"", "?:", "fallbackValue"] {
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
        let code = Code::plain("합계금액 cafe\u{301} 👩‍💻 12.25e-3 veryLongIdentifier");
        let lines = code.wrap(7);
        assert_eq!(lines.iter().map(text).collect::<String>(), code.text);
        for token in [
            "합계금액",
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
