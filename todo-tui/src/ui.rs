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

use crate::app::App;
use crate::domain::Todo;
use crate::tree::{self, ChildProgress};

const MIN_SIZE: (u16, u16) = (40, 10);
const KEY_HINTS: &[&[KeyHint]] = &[
    &[
        ("i", "입력"),
        ("s", "하위추가"),
        ("e", "편집"),
        ("space", "완료"),
        ("d", "삭제"),
        ("u", "되돌리기"),
        ("y", "복사"),
    ],
    &[
        ("↑↓", "이동"),
        ("←", "접기"),
        ("→", "펼치기"),
        ("Shift+←", "넣기"),
        ("Shift+→", "빼기"),
        ("Shift+↑↓", "순서"),
    ],
    &[
        ("1-5", "탭 전환"),
        ("n", "새 탭"),
        ("x", "탭 닫기"),
        ("r", "탭 이름"),
        ("{}", "탭 순서"),
        ("Tab+←→", "탭으로 보내기"),
        ("q", "종료"),
    ],
];

pub(crate) fn render(f: &mut Frame, app: &mut App) {
    if render_if_too_small(f, MIN_SIZE, "터미널을 40 x 10 이상으로 늘려 주세요") {
        return;
    }
    let area = f.area();
    let inner = area.width.saturating_sub(2);
    let bottom_height = match app.mode {
        Mode::Insert => 3,
        Mode::Normal => key_hint_lines(KEY_HINTS, inner).len() as u16 + 3,
    };

    let [top, mid, bot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(bottom_height),
    ])
    .areas(area);

    f.render_widget(title_bar(app), top);
    f.render_stateful_widget(todo_list(app, mid.width), mid, &mut app.list_state);
    match app.mode {
        Mode::Insert => f.render_widget(
            input_box("새 할 일 · Enter 추가 · Esc 명령모드", &app.input, bot),
            bot,
        ),
        Mode::Normal => f.render_widget(help_panel(app, inner), bot),
    }

    let active_input = if let Some(popup) = &app.popup {
        let area = centered_rect(70, popup.kind.height(), f.area());
        f.render_widget(Clear, area);
        f.render_widget(input_box(popup.kind.label(), &popup.input, area), area);
        Some((area, &popup.input))
    } else if app.mode == Mode::Insert {
        Some((bot, &app.input))
    } else {
        None
    };
    if let Some((area, input)) = active_input {
        f.set_cursor_position(input_cursor(area, input));
    }
}

fn title_bar(app: &App) -> Paragraph<'static> {
    let top_level = app.todos.iter().filter(|t| t.parent_id.is_none()).count();

    let mut spans = vec![" To-Do ".cyan().bold(), app.mode.badge(), Span::raw("  ")];
    for (i, p) in app.projects.iter().enumerate() {
        spans.push(if p.id == app.active_project_id {
            format!(" {} {} ({top_level}개) ", i + 1, p.name)
                .cyan()
                .bold()
                .reversed()
        } else {
            format!(" {} {} ", i + 1, p.name).dim()
        });
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(" "));
    if app.tab_held {
        spans.push("⇥ ←→ 보내기  ".yellow().bold());
    }
    Paragraph::new(Line::from(spans)).block(Block::bordered().border_type(BorderType::Rounded))
}

fn todo_list(app: &App, width: u16) -> List<'static> {
    const BORDERS_AND_HIGHLIGHT_SYMBOL_WIDTH: u16 = 4;
    let content_width = width.saturating_sub(BORDERS_AND_HIGHLIGHT_SYMBOL_WIDTH) as usize;
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| todo_item(app, &app.todos[i], content_width))
        .collect();

    List::new(items)
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .title(" 목록 "),
        )
        .highlight_style(Style::new().reversed().bold())
        .highlight_symbol("▶ ")
}

fn help_panel(app: &App, inner_width: u16) -> Paragraph<'static> {
    let mut lines = key_hint_lines(KEY_HINTS, inner_width);
    lines.push(Line::from(app.status.clone().yellow()));
    Paragraph::new(lines).block(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .title(" 안내 "),
    )
}

fn todo_item(app: &App, todo: &Todo, width: usize) -> ListItem<'static> {
    let depth = app.depth_of(todo.id);
    let progress = tree::child_progress(&app.todos, todo.id);
    let has_children = progress.total > 0;
    let mut prefix = Vec::new();

    if let Some(parent_id) = todo.parent_id {
        let is_last_sibling = app
            .todos
            .iter()
            .rfind(|c| c.parent_id == Some(parent_id))
            .is_some_and(|c| c.id == todo.id);
        let branch = if is_last_sibling { "└ " } else { "├ " };
        prefix.push(format!("{}{branch}", "    ".repeat(depth)).dim());
    }
    if has_children {
        let caret = if todo.collapsed { "▸ " } else { "▾ " };
        prefix.push(caret.dim());
    } else if depth == 0 {
        prefix.push(Span::raw("  "));
    }

    prefix.push(checkbox(todo.done));
    prefix.push(format!(" {} ", todo.created_at_string()).dim());
    prefix.push(Span::raw(" "));

    let mut suffix = Vec::new();
    if has_children {
        suffix.push(progress_badge(progress));
    }
    ListItem::new(wrapped_lines(
        prefix,
        &todo.text,
        content_style(todo.done),
        suffix,
        width,
    ))
}

fn progress_badge(ChildProgress { done, total }: ChildProgress) -> Span<'static> {
    let badge = format!("  ({done}/{total})");
    if done == total {
        badge.green()
    } else {
        badge.dim()
    }
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.content.as_ref().width()).sum()
}

fn wrapped_lines(
    prefix: Vec<Span<'static>>,
    text: &str,
    text_style: Style,
    suffix: Vec<Span<'static>>,
    width: usize,
) -> Vec<Line<'static>> {
    let prefix_w = spans_width(&prefix);
    let avail = width.saturating_sub(prefix_w).max(8);
    let chunks = tui_kit::text::wrap(text, avail);

    let mut lines: Vec<Line> = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.into_iter().enumerate() {
        let mut spans = if i == 0 {
            prefix.clone()
        } else {
            vec![Span::raw(" ".repeat(prefix_w))]
        };
        spans.push(Span::styled(chunk, text_style));
        lines.push(Line::from(spans));
    }

    if !suffix.is_empty() {
        let suffix_w = spans_width(&suffix);
        if let Some(last) = lines.last_mut()
            && spans_width(&last.spans) + suffix_w <= width
        {
            last.spans.extend(suffix);
        } else {
            let mut spans = vec![Span::raw(" ".repeat(prefix_w))];
            spans.extend(suffix);
            lines.push(Line::from(spans));
        }
    }
    lines
}

fn checkbox(done: bool) -> Span<'static> {
    if done { "[x] ".green() } else { "[ ] ".dim() }
}

fn content_style(done: bool) -> Style {
    if done {
        Style::new().dim().crossed_out()
    } else {
        Style::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wrapped_lines_indent_continuation_and_carry_suffix() {
        let prefix = vec![Span::raw("  "), Span::raw("[ ] ")];
        let lines = wrapped_lines(
            prefix,
            "aaaa bbbb cccc",
            Style::default(),
            vec![Span::raw("  (1/2)")],
            12,
        );
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(texts.len() > 1);
        assert_eq!(texts[0], "  [ ] aaaa");
        assert!(texts[1].starts_with("      bbbb"));
        assert!(texts.last().unwrap().contains("(1/2)"));
    }
}
