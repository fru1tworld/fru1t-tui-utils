use ratatui::{
    layout::{Position, Rect},
    style::{Style, Stylize},
    widgets::{Block, BorderType, Paragraph},
};
use tui_input::Input;
use unicode_width::UnicodeWidthStr;

/// Bordered, focused text box that scrolls to keep the cursor in view.
pub fn input_box(title: &str, input: &Input, area: Rect) -> Paragraph<'static> {
    Paragraph::new(input.value().to_owned())
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().yellow())
                .title(format!(" {title} ").yellow().bold()),
        )
        .scroll(input_scroll(area, input))
}

pub fn input_cursor(area: Rect, input: &Input) -> Position {
    let (row, column) = input_position(input);
    let (vertical_scroll, horizontal_scroll) = input_scroll(area, input);
    Position {
        x: (area.x + 1 + column.saturating_sub(usize::from(horizontal_scroll)) as u16)
            .min(area.right().saturating_sub(2)),
        y: (area.y + 1 + row.saturating_sub(usize::from(vertical_scroll)) as u16)
            .min(area.bottom().saturating_sub(2)),
    }
}

fn input_position(input: &Input) -> (usize, usize) {
    let before_cursor = input
        .value()
        .chars()
        .take(input.cursor())
        .collect::<String>();
    let row = before_cursor.matches('\n').count();
    let column = before_cursor.rsplit('\n').next().unwrap_or("").width();
    (row, column)
}

fn input_scroll(area: Rect, input: &Input) -> (u16, u16) {
    let (row, column) = input_position(input);
    let inner_height = usize::from(area.height.saturating_sub(2).max(1));
    let inner_width = usize::from(area.width.saturating_sub(2).max(1));
    (
        row.saturating_sub(inner_height - 1) as u16,
        column.saturating_sub(inner_width - 1) as u16,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_cursor_uses_the_current_lines_unicode_width() {
        let input = Input::new("첫 줄\nLens 🔍".into());

        assert_eq!(input_position(&input), (1, 7));
    }

    #[test]
    fn multiline_input_scrolls_to_keep_the_cursor_visible() {
        let input = Input::new("1\n2\n3\n4\n5\n6\n7".into());
        let area = Rect::new(0, 0, 20, 6);

        assert_eq!(input_scroll(area, &input), (3, 0));
        assert_eq!(input_cursor(area, &input), Position { x: 2, y: 4 });
    }

    #[test]
    fn single_line_input_scrolls_horizontally() {
        let input = Input::new("가".repeat(10));
        let area = Rect::new(0, 0, 12, 3);

        assert_eq!(input_scroll(area, &input), (0, 11));
        assert_eq!(input_cursor(area, &input), Position { x: 10, y: 1 });
    }
}
