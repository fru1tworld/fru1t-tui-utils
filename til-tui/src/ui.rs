use chrono::{Datelike, Weekday};
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Position, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, Paragraph},
};
use tui_input::Input;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, Mode, VisibleRow};
use crate::domain::TilEntry;

pub(crate) fn ui(frame: &mut Frame, app: &mut App) {
    let [top, middle, bottom] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(2),
        Constraint::Length(8),
    ])
    .areas(frame.area());

    frame.render_widget(title(app), top);
    frame.render_stateful_widget(entry_list(app, middle.width), middle, &mut app.selection);
    frame.render_widget(bottom_panel(app, bottom), bottom);

    let editing = if let Some(dialog) = &app.edit_dialog {
        let area = centered_rect(70, 8, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(
            input_box(
                "기록 편집 (Enter 저장 · Shift+Enter 개행 · Esc 취소)",
                &dialog.input,
                area,
            ),
            area,
        );
        Some((area, &dialog.input))
    } else if app.mode == Mode::Insert {
        Some((bottom, &app.input))
    } else {
        None
    };
    if let Some((area, input)) = editing {
        frame.set_cursor_position(input_cursor(area, input));
    }
}

fn title(app: &App) -> Paragraph<'static> {
    let mode = match app.mode {
        Mode::Insert => "-- INSERT --".green().bold(),
        Mode::Normal => "-- NORMAL --".blue().bold(),
    };
    let today = chrono::Local::now().date_naive();
    let selected_date = if app.selected_date == today {
        format!("{} (오늘)", app.selected_date)
    } else {
        app.selected_date.to_string()
    };
    let week = app.calendar_week();
    Paragraph::new(Line::from(vec![
        " TIL ".cyan().bold(),
        format!(
            " {} · {}~{} ",
            week.label(),
            week.start().format("%m.%d"),
            week.end().format("%m.%d")
        )
        .cyan()
        .bold()
        .reversed(),
        format!(" 선택 {selected_date} · {}개  ", app.week_entry_count()).dim(),
        mode,
    ]))
    .block(Block::bordered().border_type(BorderType::Rounded))
}

fn entry_list(app: &App, width: u16) -> List<'static> {
    let items = app
        .visible_rows
        .iter()
        .map(|row| match *row {
            VisibleRow::Date { group_index } => date_item(app, group_index),
            VisibleRow::Entry {
                group_index,
                entry_index,
            } => {
                let group = &app.date_groups[group_index];
                entry_item(
                    &group.entries[entry_index],
                    width.saturating_sub(4) as usize,
                    entry_index + 1 == group.entries.len(),
                )
            }
        })
        .collect::<Vec<_>>();

    List::new(items)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" 주간 배운 내용 "),
        )
        .highlight_style(Style::new().reversed().bold())
        .highlight_symbol("▶ ")
}

fn date_item(app: &App, group_index: usize) -> ListItem<'static> {
    let group = &app.date_groups[group_index];
    let caret = if app.is_collapsed(group.date) {
        "▸"
    } else {
        "▾"
    };
    let today = if group.date == chrono::Local::now().date_naive() {
        " (오늘)"
    } else {
        ""
    };
    let weekday = match group.date.weekday() {
        Weekday::Mon => "월",
        Weekday::Tue => "화",
        Weekday::Wed => "수",
        Weekday::Thu => "목",
        Weekday::Fri => "금",
        Weekday::Sat => "토",
        Weekday::Sun => "일",
    };
    ListItem::new(Line::from(vec![
        format!("{caret} {} ({weekday}){today}", group.date)
            .cyan()
            .bold(),
        format!("  {}개", group.entries.len()).dim(),
    ]))
}

fn entry_item(entry: &TilEntry, width: usize, is_last: bool) -> ListItem<'static> {
    let branch = if is_last { "└" } else { "├" };
    let prefix = format!("  {branch}─ {}  ", entry.time_label());
    let prefix_width = prefix.width();
    let available = width.saturating_sub(prefix_width).max(8);
    let mut lines = Vec::new();

    for source_line in entry.content.split('\n') {
        let chunks = textwrap::wrap(source_line.trim_end_matches('\r'), available);
        if chunks.is_empty() {
            lines.push(Line::from(vec![
                if lines.is_empty() {
                    prefix.clone()
                } else {
                    " ".repeat(prefix_width)
                }
                .dim(),
                Span::raw(""),
            ]));
            continue;
        }
        for chunk in chunks {
            let line_prefix = if lines.is_empty() {
                prefix.clone()
            } else {
                " ".repeat(prefix_width)
            };
            lines.push(Line::from(vec![
                line_prefix.dim(),
                Span::raw(chunk.into_owned()),
            ]));
        }
    }
    ListItem::new(lines)
}

fn bottom_panel(app: &App, area: Rect) -> Paragraph<'_> {
    match app.mode {
        Mode::Insert => input_box(
            "배운 내용 (←→ 주 이동 · Enter 추가 · Shift+Enter 개행 · 'out' 출력 · Esc 명령모드)",
            &app.input,
            area,
        ),
        Mode::Normal => {
            let mut lines = vec![
                Line::from("i 입력   e 편집   d 삭제   u 되돌리기   y 복사   o 출력"),
                Line::from(
                    "←→ 주 이동   Enter 날짜 접기/펼치기   t 오늘   Shift+←→ 기록 날짜 이동   ↑↓ 선택   q 종료",
                ),
            ];
            if !app.status.is_empty() {
                lines.push(Line::from(app.status.clone().yellow()));
            }
            Paragraph::new(lines).dim().block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .title(" 안내 "),
            )
        }
    }
}

fn input_box<'a>(title: &'a str, input: &'a Input, area: Rect) -> Paragraph<'a> {
    let (vertical_scroll, horizontal_scroll) = input_scroll(area, input);
    Paragraph::new(input.value())
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(format!(" {title} ")),
        )
        .scroll((vertical_scroll, horizontal_scroll))
}

fn input_cursor(area: Rect, input: &Input) -> Position {
    let (row, column) = input_position(input);
    let (vertical_scroll, horizontal_scroll) = input_scroll(area, input);
    Position {
        x: (area.x + 1 + column.saturating_sub(horizontal_scroll as usize) as u16)
            .min(area.right().saturating_sub(2)),
        y: (area.y + 1 + row.saturating_sub(vertical_scroll as usize) as u16)
            .min(area.bottom().saturating_sub(2)),
    }
}

fn input_position(input: &Input) -> (usize, usize) {
    let before_cursor = input
        .value()
        .chars()
        .take(input.cursor())
        .collect::<String>();
    let row = before_cursor
        .chars()
        .filter(|character| *character == '\n')
        .count();
    let column = before_cursor.rsplit('\n').next().unwrap_or("").width();
    (row, column)
}

fn input_scroll(area: Rect, input: &Input) -> (u16, u16) {
    let (row, column) = input_position(input);
    let inner_height = area.height.saturating_sub(2).max(1) as usize;
    let inner_width = area.width.saturating_sub(2).max(1) as usize;
    (
        row.saturating_sub(inner_height - 1) as u16,
        column.saturating_sub(inner_width - 1) as u16,
    )
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let [horizontal] = Layout::horizontal([Constraint::Percentage(percent_x)])
        .flex(Flex::Center)
        .areas(area);
    let [vertical] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(horizontal);
    vertical
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
}
