use chrono::{Datelike, Weekday};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, Paragraph},
};
use tui_kit::input::{input_box, input_cursor};
use tui_kit::layout::{KeyHint, centered_rect, key_hint_lines, render_if_too_small};
use tui_kit::mode::Mode;
use unicode_width::UnicodeWidthStr;

use crate::app::{App, VisibleRow};
use crate::domain::TilEntry;

const MIN_SIZE: (u16, u16) = (40, 12);
const INSERT_PANEL_HEIGHT: u16 = 8;
const KEY_HINTS: &[&[KeyHint]] = &[
    &[
        ("i", "입력"),
        ("e", "편집"),
        ("d", "삭제"),
        ("u", "되돌리기"),
        ("y", "복사"),
        ("o", "출력"),
    ],
    &[
        ("↑↓", "선택"),
        ("←→", "주 이동"),
        ("t", "오늘"),
        ("Enter", "접기/펼치기"),
        ("Shift+←→", "기록 날짜 이동"),
        ("q", "종료"),
    ],
];

pub(crate) fn render(frame: &mut Frame, app: &mut App) {
    if render_if_too_small(frame, MIN_SIZE, "터미널을 40 x 12 이상으로 늘려 주세요") {
        return;
    }
    let inner_width = frame.area().width.saturating_sub(2);
    let hints = key_hint_lines(KEY_HINTS, inner_width);
    let bottom_height = match app.mode {
        Mode::Insert => INSERT_PANEL_HEIGHT,
        Mode::Normal => hints.len() as u16 + 3,
    };
    let [top, middle, bottom] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(2),
        Constraint::Length(bottom_height),
    ])
    .areas(frame.area());

    frame.render_widget(title(app), top);
    frame.render_stateful_widget(entry_list(app, middle.width), middle, &mut app.selection);
    match app.mode {
        Mode::Insert => frame.render_widget(
            input_box(
                "배운 내용 · Enter 추가 · Shift+Enter 개행 · ←→ 주 이동 · 'out' 출력 · Esc 명령모드",
                &app.input,
                bottom,
            ),
            bottom,
        ),
        Mode::Normal => frame.render_widget(help_panel(app, hints), bottom),
    }

    let focused_input = if let Some(dialog) = &app.edit_dialog {
        let area = centered_rect(70, 8, frame.area());
        frame.render_widget(Clear, area);
        frame.render_widget(
            input_box(
                "기록 편집 · Enter 저장 · Shift+Enter 개행 · Esc 취소",
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
    if let Some((area, input)) = focused_input {
        frame.set_cursor_position(input_cursor(area, input));
    }
}

fn title(app: &App) -> Paragraph<'static> {
    let today = chrono::Local::now().date_naive();
    let selected_date = if app.selected_date == today {
        format!("{} (오늘)", app.selected_date)
    } else {
        app.selected_date.to_string()
    };
    let week = app.calendar_week();
    Paragraph::new(Line::from(vec![
        " TIL ".cyan().bold(),
        app.mode.badge(),
        Span::raw(" "),
        format!(
            " {} · {}~{} ",
            week.label(),
            week.start().format("%m.%d"),
            week.end().format("%m.%d")
        )
        .cyan()
        .bold()
        .reversed(),
        format!(" 선택 {selected_date} · {}개", app.week_entry_count()).dim(),
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
    let is_today = group.date == chrono::Local::now().date_naive();
    let today = if is_today { " (오늘)" } else { "" };
    let weekday = match group.date.weekday() {
        Weekday::Mon => "월",
        Weekday::Tue => "화",
        Weekday::Wed => "수",
        Weekday::Thu => "목",
        Weekday::Fri => "금",
        Weekday::Sat => "토",
        Weekday::Sun => "일",
    };
    let heading = format!("{caret} {} ({weekday}){today}", group.date);
    let heading = if is_today {
        heading.yellow().bold()
    } else if group.entries.is_empty() {
        heading.dim()
    } else {
        heading.cyan().bold()
    };
    ListItem::new(Line::from(vec![
        heading,
        format!("  {}개", group.entries.len()).dim(),
    ]))
}

fn entry_item(entry: &TilEntry, width: usize, is_last: bool) -> ListItem<'static> {
    let branch = if is_last { "└" } else { "├" };
    let prefix = format!("  {branch}─ {}  ", entry.time_label());
    let prefix_width = prefix.width();
    let continuation = " ".repeat(prefix_width);
    let available = width.saturating_sub(prefix_width).max(8);

    let lines = tui_kit::text::wrap(&entry.content, available)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let line_prefix = if index == 0 {
                prefix.clone()
            } else {
                continuation.clone()
            };
            Line::from(vec![line_prefix.dim(), Span::raw(chunk)])
        })
        .collect::<Vec<_>>();
    ListItem::new(lines)
}

fn help_panel(app: &App, mut lines: Vec<Line<'static>>) -> Paragraph<'static> {
    lines.push(Line::from(app.status.clone().yellow()));
    Paragraph::new(lines).block(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(" 안내 "),
    )
}
