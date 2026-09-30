use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Position, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, Paragraph},
};
use tui_input::Input;

use crate::app::{App, Mode};
use crate::db::Todo;
use crate::tree::{self, ChildProgress};

const HELP_GROUPS: &[&[&str]] = &[
    &[
        "i 입력",
        "s 하위추가",
        "e 편집",
        "space 완료",
        "d 삭제",
        "u 되돌리기",
        "y 복사",
    ],
    &[
        "↑↓ 이동",
        "← 접기",
        "→ 펼치기",
        "Shift+← 넣기",
        "Shift+→ 빼기",
        "Shift+↑↓ 순서",
    ],
    &[
        "1-5 탭 전환",
        "n 새 탭",
        "x 탭 닫기",
        "r 탭 이름",
        "{} 탭 순서",
        "Tab+←→ 탭으로 보내기",
        "q 종료",
    ],
];

pub(crate) fn render(f: &mut Frame, app: &mut App) {
    let area = f.area();

    let inner = area.width.saturating_sub(2);
    let bottom = bottom_panel(app, inner);
    let bottom_height = bottom.line_count(inner).max(3) as u16;

    let [top, mid, bot] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(bottom_height),
    ])
    .areas(area);

    f.render_widget(title_bar(app), top);
    f.render_stateful_widget(todo_list(app, mid.width), mid, &mut app.list_state);
    f.render_widget(bottom, bot);

    let active_input = if let Some(popup) = &app.popup {
        let area = centered_rect(60, 3, f.area());
        f.render_widget(Clear, area);
        f.render_widget(
            input_box(popup.kind.label(), &popup.input, area.width),
            area,
        );
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

fn input_inner_width(box_width: u16) -> usize {
    const BORDERS_AND_CURSOR_WIDTH: u16 = 3;
    box_width.saturating_sub(BORDERS_AND_CURSOR_WIDTH) as usize
}

fn input_cursor(area: Rect, input: &Input) -> Position {
    let inner = input_inner_width(area.width);
    let scroll = input.visual_scroll(inner);
    let x = area.x + 1 + input.visual_cursor().saturating_sub(scroll) as u16;
    Position {
        x: x.min(area.right().saturating_sub(2)),
        y: area.y + 1,
    }
}

fn title_bar(app: &App) -> Paragraph<'static> {
    let tag = match app.mode {
        Mode::Insert => "-- INSERT --".green().bold(),
        Mode::Normal => "-- NORMAL --".blue().bold(),
    };
    let top_level = app.todos.iter().filter(|t| t.parent_id.is_none()).count();

    let mut spans = vec![" To-Do ".cyan().bold()];
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
    spans.push(tag);
    Paragraph::new(Line::from(spans)).block(Block::bordered())
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
        .block(Block::bordered().title(" 목록  [ ] 시각 내용 "))
        .highlight_style(Style::new().reversed().bold())
        .highlight_symbol("▶ ")
}

fn bottom_panel(app: &App, inner_width: u16) -> Paragraph<'static> {
    match app.mode {
        Mode::Insert => input_box(
            "새 할 일 (Enter 추가 · Esc 명령모드)",
            &app.input,
            inner_width + 2,
        ),
        Mode::Normal => {
            let mut lines = wrap_commands(HELP_GROUPS, inner_width);
            if !app.status.is_empty() {
                lines.push(Line::from(app.status.clone().yellow()));
            }
            Paragraph::new(lines)
                .dim()
                .block(Block::bordered().title(" 안내 "))
        }
    }
}

fn wrap_commands(groups: &[&[&str]], width: u16) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthStr;

    const SEP: &str = "   ";
    let width = width as usize;
    let sep_w = SEP.width();

    let mut lines = Vec::new();
    for group in groups {
        let mut cur = String::new();
        let mut cur_w = 0usize;
        for cmd in *group {
            let w = cmd.width();
            if cur.is_empty() {
                cur.push_str(cmd);
                cur_w = w;
            } else if cur_w + sep_w + w <= width {
                cur.push_str(SEP);
                cur.push_str(cmd);
                cur_w += sep_w + w;
            } else {
                lines.push(Line::from(std::mem::take(&mut cur)));
                cur.push_str(cmd);
                cur_w = w;
            }
        }
        if !cur.is_empty() {
            lines.push(Line::from(cur));
        }
    }
    if lines.is_empty() {
        lines.push(Line::from(""));
    }
    lines
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
    use unicode_width::UnicodeWidthStr;

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
    let chunks = wrap_width(text, avail);

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

fn wrap_width(text: &str, width: usize) -> Vec<String> {
    use textwrap::{Options, WordSeparator, WordSplitter};

    let keep_hangul_words_whole = WordSeparator::AsciiSpace;
    let options = Options::new(width.max(1))
        .word_separator(keep_hangul_words_whole)
        .word_splitter(WordSplitter::NoHyphenation);
    textwrap::wrap(text, options)
        .into_iter()
        .map(|line| line.into_owned())
        .collect()
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

fn input_box(label: &str, input: &Input, box_width: u16) -> Paragraph<'static> {
    let scroll = input.visual_scroll(input_inner_width(box_width));
    Paragraph::new(input.value().to_string())
        .scroll((0, scroll as u16))
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().yellow())
                .title(format!(" {label} ")),
        )
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [a] = vertical.areas(area);
    let [a] = horizontal.areas(a);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wrap_width_breaks_by_word_and_char() {
        assert_eq!(wrap_width("", 10), vec![""]);
        assert_eq!(wrap_width("짧음", 10), vec!["짧음"]);
        assert_eq!(
            wrap_width("본인인증 완료되면 이벤트 소싱", 10),
            vec!["본인인증", "완료되면", "이벤트", "소싱"]
        );
        assert_eq!(wrap_width("가나다라마", 4), vec!["가나", "다라", "마"]);
        use unicode_width::UnicodeWidthStr;
        for line in wrap_width("이벤트 소싱 카산드라 DB에서 PSQL로 마이그레이션", 12)
        {
            assert!(line.width() <= 12, "폭 초과: {line}");
        }
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

    #[test]
    fn help_wraps_by_command_and_group() {
        let wide = wrap_commands(HELP_GROUPS, 200);
        assert_eq!(wide.len(), HELP_GROUPS.len());

        let narrow = wrap_commands(HELP_GROUPS, 12);
        for cmd in HELP_GROUPS.iter().flat_map(|g| g.iter()) {
            assert!(
                narrow.iter().any(|l| line_text(l).contains(cmd)),
                "커맨드가 쪼개짐: {cmd}"
            );
        }
    }
}
