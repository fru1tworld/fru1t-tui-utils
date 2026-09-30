use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{App, BranchPicker, Dialog, Focus},
    diff_view::{Cell, Row, SPLIT_GUTTER, UNIFIED_GUTTER, ViewMode},
    file_tree::{Entry, EntryKind},
    git::{Change, ChangeStatus, LineKind, Whitespace, escape_controls},
};

const MIN_WIDTH: u16 = 40;
const MIN_HEIGHT: u16 = 10;
const SIDE_BY_SIDE_MIN_WIDTH: u16 = 90;
const KEY_HINTS: &str = "v: view  Tab: expand/back  i: indent  I: guides  W: whitespace  n/p: file  /: search  ?: help  q: quit";
const HELP: &str = "v / Tab       Switch view / expand code\nEnter         Open file / toggle folder\nLeft / Right  Folder navigation / code scroll\nj / k, arrows  Move in focused pane\nn / p         Next / previous file\nPgUp / PgDn   Page up / down\nCtrl-u / d    Half page up / down\ng / G         First / last\n[ / ]         Previous / next hunk\ni / I         Compact indent / indent guides\nW             Ignore whitespace on / off\nt             Show / hide tests\n/             Filter file paths\nb             Choose FROM, then TO branch\nw / s / u     Working / staged / unstaged\nr             Reload comparison\nq / Ctrl-c    Quit\nEsc / ?       Close help";

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        frame.render_widget(
            Paragraph::new("Resize terminal to at least 40 x 10.\nq: quit"),
            area,
        );
        return;
    }
    let path_lines = wrap_name(&focused_path(app), usize::from(area.width));
    let header_height = (2 + path_lines.len()).min(usize::from(area.height - 5)) as u16;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(3),
        Constraint::Length(2),
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
    draw_footer(frame, app, footer);
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
        Some("No changes in this comparison.")
    } else if !app.query.is_empty() {
        Some("No matching files. Esc clears the search.")
    } else {
        Some("All changed files are tests. Press t to show them.")
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
        Whitespace::Ignored => "  [ignore whitespace]",
    };
    let mut heading = vec![
        Line::from(vec![
            "diff-tui  ".cyan().bold(),
            Span::raw(escape_controls(&root)),
            format!(
                "    {} files, {} tests hidden",
                app.visible.len(),
                app.hidden_test_count()
            )
            .dim(),
        ]),
        Line::from(format!(
            "{}    [{}]{whitespace}",
            escape_controls(&app.mode.label()),
            app.display.mode.label(),
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
        " Files (Tab: code) ".into()
    } else {
        format!(" Files /{} ", escape_controls(&app.query))
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
        .highlight_style(Style::new().bg(Color::DarkGray).bold())
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
            Whitespace::Ignored => "No changes after ignoring whitespace. W shows all changes.",
            Whitespace::Compared => "No diff content. Press r to refresh.",
        };
        frame.render_widget(Paragraph::new(message).block(block), area);
    } else {
        render_diff(frame, app, area);
    }
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let status = if let Some(error) = app.error.as_ref().or(app.refresh_error.as_ref()) {
        Line::from(error.as_str().red())
    } else if app.searching {
        Line::from(
            format!(
                "/{}  (type to filter, Enter: keep, Esc: clear)",
                escape_controls(&app.query)
            )
            .yellow(),
        )
    } else {
        Line::from(KEY_HINTS.dim())
    };
    let position = match app.focus {
        Focus::Files => format!(
            "item {}/{}",
            app.tree.state.selected().map_or(0, |index| index + 1),
            app.tree.entries.len()
        ),
        Focus::Diff => {
            let row = if app.display.rows.is_empty() {
                0
            } else {
                app.scroll + 1
            };
            format!("row {row}/{}", app.display.rows.len())
        }
    };
    frame.render_widget(
        Paragraph::new(vec![
            status,
            Line::from(format!(
                "w: working  s: staged  u: unstaged  r: refresh  auto: 1s    {position}",
            ))
            .dim(),
        ]),
        area,
    );
}

fn draw_dialog(frame: &mut Frame, app: &mut App, body: Rect) {
    let Some(dialog) = &mut app.dialog else {
        return;
    };
    let area = popup(body);
    frame.render_widget(Clear, area);
    match dialog {
        Dialog::Help => {
            frame.render_widget(Paragraph::new(HELP).block(panel(" Keys ", true)), area);
        }
        Dialog::Branches(picker) => draw_branch_picker(frame, picker, area),
    }
}

fn draw_branch_picker(frame: &mut Frame, picker: &mut BranchPicker, area: Rect) {
    let title = match &picker.from {
        Some(from) => format!(" TO branch (FROM: {}) ", escape_controls(from)),
        None => " FROM branch ".into(),
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
        Paragraph::new(format!("Search: {}", escape_controls(&picker.query))).yellow(),
        input,
    );
    let mut items: Vec<_> = picker
        .matching()
        .map(|name| ListItem::new(escape_controls(name)))
        .collect();
    if items.is_empty() {
        items.push(ListItem::new("No matching branches"));
    }
    let list = List::new(items)
        .highlight_style(Style::new().bg(Color::DarkGray).bold())
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, choices, &mut picker.state);
    frame.render_widget(
        Paragraph::new("Type: filter  Up/Down: select  Enter: choose  Esc: cancel").dim(),
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
                &format!(" Before | {language} "),
            );
            render_code_panel(
                frame,
                app,
                after,
                |row| row.right.as_ref(),
                SPLIT_GUTTER,
                &format!(" After | {language} "),
            );
        }
        ViewMode::Unified => render_code_panel(
            frame,
            app,
            area,
            |row| row.left.as_ref(),
            UNIFIED_GUTTER,
            &format!(" Unified | {language} "),
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
            render_cell(frame, cell, area, gutter, app.horizontal_scroll);
        }
    }
}

fn render_cell(frame: &mut Frame, cell: &Cell, area: Rect, gutter: u16, horizontal_scroll: u16) {
    let [numbers, code] = Layout::horizontal([
        Constraint::Length(gutter.min(area.width)),
        Constraint::Min(0),
    ])
    .areas(area);
    let style = match cell.kind {
        LineKind::Added => Style::new().bg(Color::Rgb(24, 48, 34)),
        LineKind::Removed => Style::new().bg(Color::Rgb(54, 29, 35)),
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
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("No changes in this comparison."));
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
