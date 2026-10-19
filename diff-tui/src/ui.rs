use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};
use tui_kit::layout::{KeyHint, key_hint_lines};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, BranchPicker, Dialog, Focus},
    appearance::Palette,
    diff_view::{Cell, Row, SPLIT_GUTTER, UNIFIED_GUTTER, ViewMode},
    file_tree::{Entry, EntryKind},
    git::{Change, ChangeStatus, LineKind, Whitespace, escape_controls},
};

const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 10;
const SIDE_BY_SIDE_MIN_WIDTH: u16 = 90;
const VIEW_HINTS: &[KeyHint] = &[
    ("v", "보기"),
    ("Tab", "펼치기/돌아가기"),
    ("i", "들여쓰기"),
    ("I", "가이드"),
    ("W", "공백"),
    ("n/p", "파일"),
    ("/", "검색"),
    ("?", "도움말"),
    ("q", "종료"),
];
const COMPARISON_HINTS: &[KeyHint] = &[
    ("w", "작업 트리"),
    ("s", "스테이징"),
    ("u", "언스테이징"),
    ("r", "다시 읽기"),
];
const HELP: &str = "v / Tab       보기 전환 / 코드 펼치기\nEnter         파일 열기 / 폴더 접기·펼치기\nLeft / Right  폴더 이동 / 코드 가로 스크롤\nj / k, 화살표  현재 창에서 이동\nn / p         다음 / 이전 파일\nPgUp / PgDn   한 페이지 위 / 아래\nCtrl-u / d    반 페이지 위 / 아래\ng / G         처음 / 끝\n[ / ]         이전 / 다음 hunk\ni / I         들여쓰기 압축 / 들여쓰기 가이드\nW             공백 무시 켜기 / 끄기\nt             테스트 표시 / 숨기기\nT             다크 / 라이트 테마\n/             파일 경로 필터\nb             FROM, TO 브랜치 차례로 선택\nw / s / u     작업 트리 / 스테이징 / 언스테이징\nr             비교 다시 읽기\nq / Ctrl-c    종료\nEsc / ?       도움말 닫기";

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new("터미널을 40 x 10 이상으로 늘려 주세요.\nq 종료"),
            area,
        );
        return;
    }
    let footer_lines = footer_lines(app, area.width);
    let footer_height = (footer_lines.len() as u16).min(area.height - 5);
    let path_lines = wrap_name(&focused_path(app), usize::from(area.width));
    let header_height =
        (2 + path_lines.len()).min(usize::from(area.height - 3 - footer_height)) as u16;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(3),
        Constraint::Length(footer_height),
    ])
    .areas(area);
    draw_header(frame, app, header, path_lines);
    let empty_message = empty_message(app);
    let diff = match app.focus {
        Focus::Files => {
            let [files, diff] = split_files_and_diff(body);
            draw_file_list(frame, app, files, empty_message);
            diff
        }
        Focus::Diff => body,
    };
    draw_diff_pane(frame, app, diff, empty_message);
    frame.render_widget(Paragraph::new(footer_lines), footer);
    draw_dialog(frame, app, body);
}

fn focused_path(app: &App) -> String {
    match app.focus {
        Focus::Files => app
            .tree
            .current()
            .map(|entry| escape_controls(&entry.path.to_string_lossy())),
        Focus::Diff => app.selected().map(Change::label),
    }
    .unwrap_or_default()
}

fn empty_message(app: &App) -> Option<&'static str> {
    if !app.visible.is_empty() {
        None
    } else if app.snapshot.changes.is_empty() {
        Some("이 비교에는 변경 사항이 없습니다.")
    } else if !app.query.is_empty() {
        Some("일치하는 파일이 없습니다. Esc로 검색을 지웁니다.")
    } else {
        Some("변경된 파일이 모두 테스트입니다. t를 누르면 표시합니다.")
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect, path_lines: Vec<String>) {
    let root = app
        .repo
        .root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let whitespace = match app.whitespace {
        Whitespace::Compared => "",
        Whitespace::Ignored => "  [공백 무시]",
    };
    let mut heading = vec![
        Line::from(vec![
            "diff-tui  ".cyan().bold(),
            Span::raw(escape_controls(&root)),
            format!(
                "    파일 {}개, 숨긴 테스트 {}개",
                app.visible.len(),
                app.hidden_test_count()
            )
            .dim(),
        ]),
        Line::from(format!(
            "{}    [{}]  [{}]{whitespace}",
            escape_controls(&app.mode.label()),
            app.display.mode.label(),
            app.display.appearance.display_name(),
        )),
    ];
    heading.extend(path_lines.into_iter().map(|line| Line::from(line).dim()));
    frame.render_widget(Paragraph::new(heading), area);
}

fn split_files_and_diff(body: Rect) -> [Rect; 2] {
    if body.width >= SIDE_BY_SIDE_MIN_WIDTH {
        Layout::horizontal([
            Constraint::Length((body.width / 3).clamp(28, 60)),
            Constraint::Min(0),
        ])
        .areas(body)
    } else {
        Layout::vertical([
            Constraint::Length((body.height / 3).max(3)),
            Constraint::Min(0),
        ])
        .areas(body)
    }
}

fn draw_file_list(frame: &mut Frame, app: &mut App, area: Rect, empty_message: Option<&str>) {
    let title = if app.query.is_empty() {
        " 파일 (Tab 코드) ".into()
    } else {
        format!(" 파일 /{} ", escape_controls(&app.query))
    };
    let block = panel(&title, true);
    if let Some(message) = empty_message {
        frame.render_widget(Paragraph::new(message).block(block), area);
        return;
    }
    let items: Vec<_> = app
        .tree
        .entries
        .iter()
        .map(|entry| tree_item(entry, &app.snapshot.changes, area.width.saturating_sub(4)))
        .collect();
    let list = List::new(items)
        .block(block)
        .highlight_style(
            Style::new()
                .bg(app.display.appearance.palette().selection)
                .bold(),
        )
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, area, &mut app.tree.state);
}

fn draw_diff_pane(frame: &mut Frame, app: &mut App, area: Rect, empty_message: Option<&str>) {
    app.layout_diff(area.width, area.height);
    let block = panel(" Diff ", app.focus == Focus::Diff);
    if let Some(message) = empty_message {
        frame.render_widget(Paragraph::new(message).block(block), area);
    } else if app.display.rows.is_empty() {
        let message = match app.whitespace {
            Whitespace::Ignored => {
                "공백을 무시하면 변경 사항이 없습니다. W를 누르면 모두 표시합니다."
            }
            Whitespace::Compared => "diff 내용이 없습니다. r을 누르면 다시 읽습니다.",
        };
        frame.render_widget(Paragraph::new(message).block(block), area);
    } else {
        render_diff(frame, app, area);
    }
}

fn footer_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let status = if let Some(error) = app.error.as_ref().or(app.refresh_error.as_ref()) {
        Some(Line::from(error.clone().red()))
    } else if app.searching {
        Some(Line::from(
            format!(
                "/{}  (입력해 필터 · Enter 유지 · Esc 지우기)",
                escape_controls(&app.query)
            )
            .yellow(),
        ))
    } else {
        None
    };
    let mut lines = match status {
        Some(status) => {
            let mut lines = vec![status];
            lines.extend(key_hint_lines(&[COMPARISON_HINTS], width));
            lines
        }
        None => key_hint_lines(&[VIEW_HINTS, COMPARISON_HINTS], width),
    };
    let position = match app.focus {
        Focus::Files => format!(
            "항목 {}/{}",
            app.tree.state.selected().map_or(0, |index| index + 1),
            app.tree.entries.len()
        ),
        Focus::Diff => {
            let row = if app.display.rows.is_empty() {
                0
            } else {
                app.scroll + 1
            };
            format!("행 {row}/{}", app.display.rows.len())
        }
    };
    lines.push(Line::from(format!("자동 새로고침 1초 · {position}")).dim());
    lines
}

fn draw_dialog(frame: &mut Frame, app: &mut App, body: Rect) {
    let selection = app.display.appearance.palette().selection;
    let Some(dialog) = &mut app.dialog else {
        return;
    };
    let area = popup(body);
    frame.render_widget(Clear, area);
    match dialog {
        Dialog::Help => {
            frame.render_widget(Paragraph::new(HELP).block(panel(" 키 ", true)), area);
        }
        Dialog::Branches(picker) => draw_branch_picker(frame, picker, area, selection),
    }
}

fn draw_branch_picker(frame: &mut Frame, picker: &mut BranchPicker, area: Rect, selection: Color) {
    let title = match &picker.from {
        Some(from) => format!(" TO 브랜치 (FROM: {}) ", escape_controls(from)),
        None => " FROM 브랜치 ".into(),
    };
    let block = panel(&title, true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [input, choices, hint] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(format!("검색: {}", escape_controls(&picker.query))).yellow(),
        input,
    );
    let mut items: Vec<_> = picker
        .matching()
        .map(|name| ListItem::new(escape_controls(name)))
        .collect();
    if items.is_empty() {
        items.push(ListItem::new("일치하는 브랜치가 없습니다"));
    }
    let list = List::new(items)
        .highlight_style(Style::new().bg(selection).bold())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, choices, &mut picker.state);
    frame.render_widget(
        Paragraph::new("입력 필터  ↑↓ 이동  Enter 선택  Esc 취소").dim(),
        hint,
    );
}

fn status_style(status: ChangeStatus) -> Style {
    match status {
        ChangeStatus::Added => Style::new().green(),
        ChangeStatus::Deleted => Style::new().red(),
        ChangeStatus::Renamed => Style::new().cyan(),
        _ => Style::new().yellow(),
    }
}

fn tree_item(entry: &Entry, changes: &[Change], width: u16) -> ListItem<'static> {
    let (marker, style) = match entry.kind {
        EntryKind::File(index) => {
            let status = changes[index.0].status;
            (status.marker(), status_style(status))
        }
        EntryKind::Directory { collapsed } => {
            (if collapsed { '▸' } else { '▾' }, Style::new().cyan())
        }
    };
    let indent = " ".repeat((entry.depth * 2).min(usize::from(width.saturating_sub(8))));
    let prefix = format!("{indent}{marker} ");
    let continuation = " ".repeat(prefix.width());
    let available = usize::from(width).saturating_sub(prefix.width()).max(1);
    let lines: Vec<_> = wrap_name(&entry.label, available)
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            let lead = if index == 0 { &prefix } else { &continuation };
            Line::from(vec![Span::styled(lead.clone(), style), Span::raw(part)])
        })
        .collect();
    ListItem::new(lines)
}

fn wrap_name(name: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for grapheme in name.graphemes(true) {
        if !line.is_empty() && used + grapheme.width() > width {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push_str(grapheme);
        used += grapheme.width();
    }
    lines.push(line);
    lines
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    let language = &app.display.language;
    match app.display.mode {
        ViewMode::Split => {
            let [before, after] =
                Layout::horizontal([Constraint::Length(area.width / 2), Constraint::Min(0)])
                    .areas(area);
            render_code_panel(
                frame,
                app,
                before,
                |row| row.left.as_ref(),
                SPLIT_GUTTER,
                &format!(" 변경 전 | {language} "),
            );
            render_code_panel(
                frame,
                app,
                after,
                |row| row.right.as_ref(),
                SPLIT_GUTTER,
                &format!(" 변경 후 | {language} "),
            );
        }
        ViewMode::Unified => render_code_panel(
            frame,
            app,
            area,
            |row| row.left.as_ref(),
            UNIFIED_GUTTER,
            &format!(" 통합 | {language} "),
        ),
    }
}

fn render_code_panel(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    column: fn(&Row) -> Option<&Cell>,
    gutter: u16,
    title: &str,
) {
    let block = panel(title, app.focus == Focus::Diff);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    for (offset, row) in app
        .display
        .rows
        .iter()
        .skip(app.scroll)
        .take(usize::from(inner.height))
        .enumerate()
    {
        if let Some(cell) = column(row) {
            let area = Rect::new(inner.x, inner.y + offset as u16, inner.width, 1);
            render_cell(
                frame,
                cell,
                area,
                gutter,
                app.horizontal_scroll,
                app.display.appearance.palette(),
            );
        }
    }
}

fn render_cell(
    frame: &mut Frame,
    cell: &Cell,
    area: Rect,
    gutter: u16,
    horizontal_scroll: u16,
    palette: &Palette,
) {
    let [numbers, code] = Layout::horizontal([
        Constraint::Length(gutter.min(area.width)),
        Constraint::Min(0),
    ])
    .areas(area);
    let style = match cell.kind {
        LineKind::Added => Style::new().bg(palette.added),
        LineKind::Removed => Style::new().bg(palette.removed),
        LineKind::Hunk => Style::new().cyan(),
        LineKind::Header => Style::new().dark_gray(),
        LineKind::Context => Style::default(),
    };
    let number_style = match cell.kind {
        LineKind::Added => style.green(),
        LineKind::Removed => style.red(),
        _ => style.dark_gray(),
    };
    frame.render_widget(
        Paragraph::new(cell.gutter.as_str()).style(number_style),
        numbers,
    );
    frame.render_widget(
        Paragraph::new(cell.code.clone())
            .style(style)
            .scroll((0, horizontal_scroll)),
        code,
    );
}

fn panel(title: &str, focused: bool) -> Block<'_> {
    let border = if focused {
        Style::new().cyan()
    } else {
        Style::new().dark_gray()
    };
    Block::bordered().title(title).border_style(border)
}

fn popup(area: Rect) -> Rect {
    let width = area.width.min(82);
    let height = area.height.min(21);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// Screen contents as one string, skipping the filler cell after each wide character.
#[cfg(test)]
pub fn screen_text(buffer: &ratatui::buffer::Buffer) -> String {
    let mut text = String::new();
    let mut skip = 0;
    for cell in &buffer.content {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        text.push_str(cell.symbol());
        skip = cell.symbol().width().saturating_sub(1);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::TestFiles,
        git::{ChangeIndex, Comparison, Mode, Repository, Snapshot},
    };
    use ratatui::{Terminal, backend::TestBackend, widgets::ListState};

    #[test]
    fn empty_filtered_and_small_screens_render() {
        let mut app = App::from_snapshot(
            Repository {
                root: "/example".into(),
            },
            Mode::Working,
            TestFiles::Hidden,
            Snapshot {
                comparison: Comparison::Unstaged,
                changes: vec![],
            },
        )
        .unwrap();
        for (width, height) in [(120, 32), (60, 20), (40, 10), (15, 5), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            app.dialog = Some(Dialog::Help);
            terminal.draw(|frame| draw(frame, &mut app)).unwrap();
            app.dialog = None;
        }
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let text = screen_text(terminal.backend().buffer());
        assert!(text.contains("이 비교에는 변경 사항이 없습니다."));
    }

    #[test]
    fn long_filenames_and_cjk_names_remain_complete_in_the_file_list() {
        let name = "RequestHandlerServiceV2Logic.kt";
        let change = Change {
            status: ChangeStatus::Modified,
            path: format!("project/application/src/main/kotlin/server/handler/{name}").into(),
            old_path: None,
        };
        let entry = Entry {
            path: change.path.clone(),
            label: name.into(),
            depth: 0,
            kind: EntryKind::File(ChangeIndex(0)),
        };
        let mut terminal = Terminal::new(TestBackend::new(34, 10)).unwrap();
        let mut state = ListState::default().with_selected(Some(0));
        terminal
            .draw(|frame| {
                frame.render_stateful_widget(
                    List::new([tree_item(&entry, std::slice::from_ref(&change), 30)])
                        .block(Block::bordered())
                        .highlight_symbol("> "),
                    frame.area(),
                    &mut state,
                )
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let first: String = (5..33).map(|x| buffer[(x, 1)].symbol()).collect();
        let second: String = (5..33).map(|x| buffer[(x, 2)].symbol()).collect();
        assert_eq!(format!("{}{}", first.trim_end(), second.trim_end()), name);
        let name = "한글漢字あアcafe\u{301}.kt";
        let lines = wrap_name(name, 7);
        assert_eq!(lines.concat(), name);
        assert!(lines.iter().all(|line| line.width() <= 7));
        assert!(lines.iter().any(|line| line.contains("e\u{301}")));
    }
}
