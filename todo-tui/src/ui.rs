use chrono::Local;
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

const HELP_GROUPS: &[&[&str]] = &[
    &[
        "i 입력",
        "s 하위추가",
        "e 편집",
        "t 마감",
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

pub(crate) fn ui(f: &mut Frame, app: &mut App) {
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
    f.render_stateful_widget(todo_list(app, mid.width), mid, &mut app.state);
    f.render_widget(bottom, bot);

    // 편집 중일 때만 실제 터미널 커서를 입력 위치에 둔다(네이티브 커서·한글 조합).
    let editing = if let Some(popup) = &app.popup {
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
    if let Some((area, input)) = editing {
        f.set_cursor_position(input_cursor(area, input));
    }
}

/// 입력 박스 내부 폭(테두리 2 + 커서 여유 1 제외).
fn input_inner_width(box_width: u16) -> usize {
    box_width.saturating_sub(3) as usize
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
        // 개수는 선택된 프로젝트 것이므로 그 탭 안에 붙인다.
        spans.push(if p.id == app.project_id {
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
    let now = Local::now().timestamp();
    // 좌우 테두리(2) + 선택 표시 "▶ "(2)를 뺀 실제 내용 폭
    let inner = width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .visible
        .iter()
        .map(|&i| {
            let t = &app.todos[i];
            let depth = app.depth_of(t.id);
            let (done, total) = app.children_done(t.id);
            let is_last = t.parent_id.is_some_and(|pid| {
                app.todos
                    .iter()
                    .rfind(|c| c.parent_id == Some(pid))
                    .is_some_and(|c| c.id == t.id)
            });
            todo_line(t, now, depth, is_last, done, total, inner)
        })
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

/// 깊이에 따라 들여쓰고, 자식이 있으면 접힘 캐럿과 (완료/전체) 배지를 붙인다.
fn todo_line(
    t: &Todo,
    now: i64,
    depth: usize,
    is_last: bool,
    done_children: usize,
    total_children: usize,
    width: usize,
) -> ListItem<'static> {
    let has_children = total_children > 0;
    let mut prefix = Vec::new();

    if depth > 0 {
        let branch = if is_last { "└ " } else { "├ " };
        prefix.push(format!("{}{branch}", "    ".repeat(depth)).dim());
    }
    if has_children {
        let caret = if t.collapsed { "▸ " } else { "▾ " };
        prefix.push(caret.dim());
    } else if depth == 0 {
        prefix.push(Span::raw("  "));
    }

    prefix.push(checkbox(t.done));
    prefix.push(timestamp_badge(t.created_at_string()));
    prefix.push(Span::raw(" "));

    let mut suffix = Vec::new();
    if has_children {
        let badge = format!("  ({done_children}/{total_children})");
        suffix.push(if done_children == total_children {
            badge.green()
        } else {
            badge.dim()
        });
    }
    push_due(&mut suffix, t, now);

    wrapped_item(prefix, &t.text, content_style(t.done), suffix, width)
}

fn wrapped_item(
    prefix: Vec<Span<'static>>,
    text: &str,
    text_style: Style,
    suffix: Vec<Span<'static>>,
    width: usize,
) -> ListItem<'static> {
    ListItem::new(wrapped_lines(prefix, text, text_style, suffix, width))
}

/// 내용이 폭을 넘으면 줄바꿈하고, 이어지는 줄은 텍스트 시작 위치에 맞춰 들여쓴다.
fn wrapped_lines(
    prefix: Vec<Span<'static>>,
    text: &str,
    text_style: Style,
    suffix: Vec<Span<'static>>,
    width: usize,
) -> Vec<Line<'static>> {
    use unicode_width::UnicodeWidthStr;

    let prefix_w: usize = prefix.iter().map(|s| s.content.as_ref().width()).sum();
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
        let suffix_w: usize = suffix.iter().map(|s| s.content.as_ref().width()).sum();
        let last = lines.last_mut().unwrap();
        let last_w: usize = last.spans.iter().map(|s| s.content.as_ref().width()).sum();
        if last_w + suffix_w <= width {
            last.spans.extend(suffix);
        } else {
            let mut spans = vec![Span::raw(" ".repeat(prefix_w))];
            spans.extend(suffix);
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// 표시 폭 기준 줄바꿈(textwrap). 공백 단위로 자르되, 한 단어가 폭보다 길면 글자 단위로 자른다.
fn wrap_width(text: &str, width: usize) -> Vec<String> {
    use textwrap::{Options, WordSeparator, WordSplitter};

    // 기본 unicode-linebreak 규칙은 한글을 음절 단위로 끊으므로 공백 기준으로 고정한다.
    let options = Options::new(width.max(1))
        .word_separator(WordSeparator::AsciiSpace)
        .word_splitter(WordSplitter::NoHyphenation);
    textwrap::wrap(text, options)
        .into_iter()
        .map(|line| line.into_owned())
        .collect()
}

fn checkbox(done: bool) -> Span<'static> {
    if done { "[x] ".green() } else { "[ ] ".dim() }
}

fn timestamp_badge(ts: String) -> Span<'static> {
    format!(" {ts} ").dim()
}

fn content_style(done: bool) -> Style {
    if done {
        Style::new().dim().crossed_out()
    } else {
        Style::new()
    }
}

fn push_due(spans: &mut Vec<Span<'static>>, t: &Todo, now: i64) {
    if let Some(due) = t.due_string() {
        let badge = format!("   ⏳ {due}");
        spans.push(if t.is_overdue(now) {
            badge.red().bold()
        } else {
            badge.dim()
        });
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
        // 공백 없는 긴 단어는 글자 단위로 잘림 (한글은 폭 2)
        assert_eq!(wrap_width("가나다라마", 4), vec!["가나", "다라", "마"]);
        // 표시 폭이 넘치는 줄이 없어야 함
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
            vec![Span::raw("  ⏳ 2026-07-02")],
            12,
        );
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        // 첫 줄은 프리픽스, 이후 줄은 같은 폭의 공백 들여쓰기
        assert!(texts.len() > 1);
        assert_eq!(texts[0], "  [ ] aaaa");
        assert!(texts[1].starts_with("      bbbb"));
        // 마감 배지는 잘리지 않고 마지막 줄(또는 새 줄)에 표시됨
        assert!(texts.last().unwrap().contains("⏳ 2026-07-02"));
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
