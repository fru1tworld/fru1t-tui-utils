use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::Stylize,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

const HINT_SEPARATOR: &str = "   ";

/// A key and what it does, rendered as `key description` in the help panel.
pub type KeyHint = (&'static str, &'static str);

pub fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let [horizontal] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(area);
    let [vertical] = Layout::vertical([Constraint::Length(height.min(area.height))])
        .flex(Flex::Center)
        .areas(horizontal);
    vertical
}

/// Renders `message` instead of the app when the terminal is smaller than `min`.
pub fn render_if_too_small(
    frame: &mut Frame,
    (min_width, min_height): (u16, u16),
    message: &str,
) -> bool {
    let area = frame.area();
    if area.width >= min_width && area.height >= min_height {
        return false;
    }
    frame.render_widget(Paragraph::new(message.to_owned()).dim(), area);
    true
}

/// Lays out key hints so that every group starts on a new line and no hint is split.
pub fn key_hint_lines(groups: &[&[KeyHint]], width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let mut lines = Vec::new();
    for group in groups {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut line_width = 0;
        for &(key, description) in *group {
            let hint_width = key.width() + 1 + description.width();
            if !spans.is_empty() {
                if line_width + HINT_SEPARATOR.width() + hint_width > width {
                    lines.push(Line::from(std::mem::take(&mut spans)));
                    line_width = 0;
                } else {
                    spans.push(Span::raw(HINT_SEPARATOR));
                    line_width += HINT_SEPARATOR.width();
                }
            }
            spans.push(key.cyan().bold());
            spans.push(format!(" {description}").dim());
            line_width += hint_width;
        }
        if !spans.is_empty() {
            lines.push(Line::from(spans));
        }
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    const GROUPS: &[&[KeyHint]] = &[
        &[("i", "입력"), ("e", "편집"), ("Shift+←→", "날짜 이동")],
        &[("q", "종료")],
    ];

    fn text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn groups_start_new_lines_and_hints_are_never_split() {
        let wide = key_hint_lines(GROUPS, 200);
        assert_eq!(wide.len(), GROUPS.len());
        assert_eq!(text(&wide[0]), "i 입력   e 편집   Shift+←→ 날짜 이동");

        let narrow = key_hint_lines(GROUPS, 10);
        for (key, description) in GROUPS.iter().flat_map(|group| group.iter()) {
            let hint = format!("{key} {description}");
            assert!(
                narrow.iter().any(|line| text(line).contains(&hint)),
                "{hint}"
            );
        }
    }

    #[test]
    fn centered_rect_never_exceeds_the_area() {
        let area = Rect::new(0, 0, 40, 5);
        let centered = centered_rect(50, 8, area);
        assert_eq!((centered.width, centered.height), (20, 5));
    }
}
