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
    app::{App, Dialog, Focus},
    diff_view::{Cell, SPLIT_GUTTER, UNIFIED_GUTTER, ViewMode},
    file_tree::Entry,
    git::{Change, LineKind, clean},
};

pub fn draw(frame: &mut Frame, app: &mut App) {
    if frame.area().width < 40 || frame.area().height < 10 {
        frame.render_widget(
            Paragraph::new("Resize terminal to at least 40 x 10.\nq: quit"),
            frame.area(),
        );
        return;
    }
    let current_path = if app.focus == Focus::Files {
        app.tree
            .current()
            .map(|entry| clean(&entry.path.to_string_lossy()))
    } else {
        app.selected().map(|change| change.label())
    }
    .unwrap_or_default();
    let path_lines = wrap_name(&current_path, usize::from(frame.area().width));
    let header_height = (2 + path_lines.len()).min(usize::from(frame.area().height - 5)) as u16;
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .areas(frame.area());
    let root = app
        .repo
        .root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let mut heading = vec![
        Line::from(vec![
            "diff-tui  ".cyan().bold(),
            Span::raw(clean(&root)),
            format!(
                "    {} files, {} tests hidden",
                app.visible.len(),
                app.hidden_count()
            )
            .dim(),
        ]),
        Line::from(format!(
            "{}    [{}]{}",
            clean(&app.mode.label()),
            app.display.mode.label(),
            if app.ignore_whitespace {
                "  [ignore whitespace]"
            } else {
                ""
            },
        )),
    ];
    heading.extend(path_lines.into_iter().map(|line| Line::from(line).dim()));
    frame.render_widget(Paragraph::new(heading), header);

    let empty_message = app.visible.is_empty().then_some({
        if app.snapshot.changes.is_empty() {
            "No changes in this comparison."
        } else if !app.query.is_empty() {
            "No matching files. Esc clears the search."
        } else {
            "All changed files are tests. Press t to show them."
        }
    });
    let (files, diff) = if app.focus == Focus::Files {
        let [files, diff] = if body.width >= 90 {
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
        };
        (files, diff)
    } else {
        (Rect::default(), body)
    };
    if app.focus == Focus::Files {
        let title = if app.query.is_empty() {
            " Files (Tab: code) ".into()
        } else {
            format!(" Files /{} ", clean(&app.query))
        };
        let block = panel(&title, true);
        if let Some(message) = empty_message {
            frame.render_widget(Paragraph::new(message).block(block), files);
        } else {
            let items: Vec<_> = app
                .tree
                .entries
                .iter()
                .map(|entry| tree_item(entry, &app.snapshot.changes, files.width.saturating_sub(4)))
                .collect();
            let list = List::new(items)
                .block(block)
                .highlight_style(Style::new().bg(Color::DarkGray).bold())
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, files, &mut app.tree.state);
        }
    }
    app.layout_diff(diff.width, diff.height);
    let diff_block = panel(" Diff ", app.focus == Focus::Diff);
    if let Some(message) = empty_message {
        frame.render_widget(Paragraph::new(message).block(diff_block), diff);
    } else if app.display.rows.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.ignore_whitespace {
                "No changes after ignoring whitespace. W shows all changes."
            } else {
                "No diff content. Press r to refresh."
            })
            .block(diff_block),
            diff,
        );
    } else {
        render_diff(frame, app, diff);
    }

    let status = if let Some(error) = app.error.as_ref().or(app.refresh_error.as_ref()) {
        Line::from(error.as_str().red())
    } else if app.searching {
        Line::from(
            format!(
                "/{}  (type to filter, Enter: keep, Esc: clear)",
                clean(&app.query)
            )
            .yellow(),
        )
    } else {
        Line::from(
            "v: view  Tab: expand/back  i: indent  I: guides  W: whitespace  n/p: file  /: search  ?: help  q: quit"
                .dim(),
        )
    };
    let position = if app.focus == Focus::Files {
        format!(
            "item {}/{}",
            app.tree.state.selected().map_or(0, |i| i + 1),
            app.tree.entries.len()
        )
    } else {
        format!(
            "row {}/{}",
            if app.display.rows.is_empty() {
                0
            } else {
                app.scroll + 1
            },
            app.display.rows.len()
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            status,
            Line::from(format!(
                "w: working  s: staged  u: unstaged  r: refresh  auto: 1s    {position}",
            ))
            .dim(),
        ]),
        footer,
    );

    if let Some(dialog) = &mut app.dialog {
        let area = popup(body);
        frame.render_widget(Clear, area);
        match dialog {
            Dialog::Help => {
                frame.render_widget(Paragraph::new(
                    "v / Tab       Switch view / expand code\nEnter         Open file / toggle folder\nLeft / Right  Folder navigation / code scroll\nj / k, arrows  Move in focused pane\nn / p         Next / previous file\nPgUp / PgDn   Page up / down\nCtrl-u / d    Half page up / down\ng / G         First / last\n[ / ]         Previous / next hunk\ni / I         Compact indent / indent guides\nW             Ignore whitespace on / off\nt             Show / hide tests\n/             Filter file paths\nb             Choose FROM, then TO branch\nw / s / u     Working / staged / unstaged\nr             Reload comparison\nq / Ctrl-c    Quit\nEsc / ?       Close help"
                ).block(panel(" Keys ", true)), area);
            }
            Dialog::Branches(picker) => {
                let title = match &picker.from {
                    Some(from) => format!(" TO branch (FROM: {}) ", clean(from)),
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
                    Paragraph::new(format!("Search: {}", clean(&picker.query))).yellow(),
                    input,
                );
                let visible = picker.visible();
                let items = if visible.is_empty() {
                    vec![ListItem::new("No matching branches")]
                } else {
                    visible
                        .into_iter()
                        .map(|name| ListItem::new(clean(name)))
                        .collect()
                };
                let list = List::new(items)
                    .highlight_style(Style::new().bg(Color::DarkGray).bold())
                    .highlight_symbol("> ");
                frame.render_stateful_widget(list, choices, &mut picker.state);
                frame.render_widget(
                    Paragraph::new("Type: filter  Up/Down: select  Enter: choose  Esc: cancel")
                        .dim(),
                    hint,
                );
            }
        }
    }
}

fn tree_item(entry: &Entry, changes: &[Change], width: u16) -> ListItem<'static> {
    let (marker, style) = if let Some(index) = entry.change {
        let change = &changes[index];
        (
            change.status,
            match change.status {
                'A' => Style::new().green(),
                'D' => Style::new().red(),
                'R' => Style::new().cyan(),
                _ => Style::new().yellow(),
            },
        )
    } else {
        (if entry.collapsed { '▸' } else { '▾' }, Style::new().cyan())
    };
    let indent = " ".repeat((entry.depth * 2).min(usize::from(width.saturating_sub(8))));
    let prefix = format!("{indent}{marker} ");
    let available = usize::from(width).saturating_sub(prefix.width()).max(1);
    let lines: Vec<_> = wrap_name(&entry.label, available)
        .into_iter()
        .enumerate()
        .map(|(i, part)| {
            Line::from(vec![
                Span::styled(
                    if i == 0 {
                        prefix.clone()
                    } else {
                        " ".repeat(prefix.width())
                    },
                    style,
                ),
                Span::raw(part),
            ])
        })
        .collect();
    ListItem::new(lines)
}

fn wrap_name(name: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for part in name.graphemes(true) {
        if !line.is_empty() && used + part.width() > width.max(1) {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push_str(part);
        used += part.width();
    }
    lines.push(line);
    lines
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    if app.display.mode == ViewMode::Split {
        let [before, after] =
            Layout::horizontal([Constraint::Length(area.width / 2), Constraint::Min(0)])
                .areas(area);
        render_code_panel(
            frame,
            app,
            before,
            true,
            SPLIT_GUTTER,
            &format!(" Before | {} ", app.display.language),
        );
        render_code_panel(
            frame,
            app,
            after,
            false,
            SPLIT_GUTTER,
            &format!(" After | {} ", app.display.language),
        );
    } else {
        render_code_panel(
            frame,
            app,
            area,
            true,
            UNIFIED_GUTTER,
            &format!(" Unified | {} ", app.display.language),
        );
    }
}

fn render_code_panel(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    before: bool,
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
        let cell = if before { &row.left } else { &row.right };
        if let Some(cell) = cell {
            let area = Rect::new(inner.x, inner.y + offset as u16, inner.width, 1);
            render_cell(frame, cell, area, gutter, app.horizontal);
        }
    }
}

fn render_cell(frame: &mut Frame, cell: &Cell, area: Rect, gutter: u16, horizontal: u16) {
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
            .scroll((0, horizontal)),
        code,
    );
}

fn panel(title: &str, focused: bool) -> Block<'_> {
    Block::bordered().title(title).border_style(if focused {
        Style::new().cyan()
    } else {
        Style::new().dark_gray()
    })
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
    use crate::git::{Comparison, Mode, Repository, Snapshot};
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn empty_filtered_and_small_screens_render() {
        let mut app = App::from_snapshot(
            Repository {
                root: "/example".into(),
            },
            Mode::Working,
            false,
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
        let name = "BillingKeyServiceKiccV2Logic.kt";
        let change = Change {
            status: 'M',
            path: format!("subproject/application/src/main/kotlin/transaction/billingkey/{name}")
                .into(),
            old_path: None,
        };
        let entry = Entry {
            path: change.path.clone(),
            label: name.into(),
            depth: 0,
            change: Some(0),
            collapsed: false,
        };
        let mut terminal = Terminal::new(TestBackend::new(34, 10)).unwrap();
        let mut state = ratatui::widgets::ListState::default().with_selected(Some(0));
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
        let name = "결제漢字あアcafe\u{301}.kt";
        let lines = wrap_name(name, 7);
        assert_eq!(lines.concat(), name);
        assert!(lines.iter().all(|line| line.width() <= 7));
        assert!(lines.iter().any(|line| line.contains("e\u{301}")));
    }
}
