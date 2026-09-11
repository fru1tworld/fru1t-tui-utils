use std::{
    collections::{BTreeSet, HashMap},
    time::{Duration, Instant},
};

use ratatui::{
    style::{Color, Style},
    text::Line,
};

use crate::{
    git::{Change, DiffLine, LineKind, Sources},
    matching::{align_lines, changed_words},
    syntax::SyntaxEngine,
    wrap::Code,
};

pub const SPLIT_GUTTER: u16 = 8;
pub const UNIFIED_GUTTER: u16 = 14;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Split,
    Unified,
}

impl ViewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Split => "Side by side",
            Self::Unified => "Unified",
        }
    }
}

pub struct Cell {
    pub gutter: String,
    pub code: Line<'static>,
    pub kind: LineKind,
}

pub struct Row {
    pub left: Option<Cell>,
    pub right: Option<Cell>,
    pub anchor: usize,
    pub other: Option<usize>,
    pub hunk: bool,
}

struct PreparedLine {
    kind: LineKind,
    old: Option<usize>,
    new: Option<usize>,
    before: Code,
    after: Code,
}

impl PreparedLine {
    fn is_file_header(&self) -> bool {
        self.kind == LineKind::Header
            && ["diff --git ", "index ", "--- ", "+++ "]
                .iter()
                .any(|prefix| self.before.text.starts_with(prefix))
    }
}

#[derive(Default)]
pub struct DiffDisplay {
    pub mode: ViewMode,
    pub full_indent: bool,
    pub hide_guides: bool,
    pub rows: Vec<Row>,
    pub language: String,
    lines: Vec<PreparedLine>,
    pairs: Vec<(Option<usize>, Option<usize>)>,
    indents: Vec<usize>,
    size: Option<(u16, ViewMode, bool, bool)>,
}

impl DiffDisplay {
    pub fn clear(&mut self) {
        self.rows.clear();
        self.lines.clear();
        self.pairs.clear();
        self.indents.clear();
        self.language.clear();
        self.size = None;
    }

    pub fn load(
        &mut self,
        patch: &[DiffLine],
        change: &Change,
        sources: &Sources,
    ) -> Option<String> {
        self.clear();
        let engine = SyntaxEngine::shared();
        self.language = engine.language(&change.path);
        let old_lines: BTreeSet<_> = patch.iter().filter_map(|line| line.old).collect();
        let new_lines: BTreeSet<_> = patch.iter().filter_map(|line| line.new).collect();
        let mut warning = None;
        let mut highlight = |path, source: Option<&str>, wanted: &BTreeSet<usize>| match source
            .map(|source| engine.highlight(path, source, wanted))
            .transpose()
        {
            Ok(lines) => lines.unwrap_or_default(),
            Err(error) => {
                warning = Some(format!(
                    "Syntax highlighting unavailable: {error}. Showing plain text."
                ));
                HashMap::new()
            }
        };
        let before = highlight(
            change.old_path.as_deref().unwrap_or(&change.path),
            sources.old.as_deref(),
            &old_lines,
        );
        let after = highlight(&change.path, sources.new.as_deref(), &new_lines);
        self.lines = patch
            .iter()
            .map(|line| {
                let is_code = matches!(
                    line.kind,
                    LineKind::Added | LineKind::Removed | LineKind::Context
                );
                let text = if is_code { &line.text[1..] } else { &line.text };
                let choose = |codes: &HashMap<usize, Code>, number: Option<usize>| {
                    // A worktree can change between reading its diff and its source. Keep the diff authoritative.
                    number
                        .and_then(|number| codes.get(&number))
                        .filter(|code| code.text == text)
                        .cloned()
                        .unwrap_or_else(|| Code::plain(text))
                };
                PreparedLine {
                    kind: line.kind,
                    old: line.old,
                    new: line.new,
                    before: choose(&before, line.old),
                    after: choose(&after, line.new),
                }
            })
            .collect();
        self.prepare();
        warning
    }

    pub fn anchor(&self, row: usize) -> usize {
        self.rows.get(row).map_or(0, |row| row.anchor)
    }

    pub fn locate(&self, anchor: usize) -> usize {
        self.rows
            .iter()
            .position(|row| row.anchor == anchor || row.other == Some(anchor))
            .or_else(|| self.rows.iter().position(|row| row.anchor > anchor))
            .unwrap_or(0)
    }

    pub fn layout(&mut self, width: u16) -> bool {
        let size = (width, self.mode, self.full_indent, self.hide_guides);
        if self.size == Some(size) {
            return false;
        }
        self.size = Some(size);
        self.rows = match self.mode {
            ViewMode::Split => self.split(width),
            ViewMode::Unified => self.unified(width),
        };
        true
    }

    fn unified(&self, width: u16) -> Vec<Row> {
        let width = usize::from(width.saturating_sub(2 + UNIFIED_GUTTER)).max(1);
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.is_file_header())
            .flat_map(|(index, line)| {
                self.wrapped(index, line.new.is_none(), width)
                    .into_iter()
                    .enumerate()
                    .map(move |(part, code)| {
                        let old = line.old.map(|n| n.to_string()).unwrap_or_default();
                        let new = line.new.map(|n| n.to_string()).unwrap_or_default();
                        let gutter = if part == 0 {
                            format!("{old:>5} {new:>5} {} ", marker(line.kind))
                        } else {
                            "            > ".into()
                        };
                        Row {
                            left: Some(Cell {
                                gutter,
                                code,
                                kind: line.kind,
                            }),
                            right: None,
                            anchor: index,
                            other: None,
                            hunk: line.kind == LineKind::Hunk && part == 0,
                        }
                    })
            })
            .collect()
    }

    fn split(&self, width: u16) -> Vec<Row> {
        let left_width = usize::from((width / 2).saturating_sub(2 + SPLIT_GUTTER)).max(1);
        let right_width = usize::from((width - width / 2).saturating_sub(2 + SPLIT_GUTTER)).max(1);
        let mut rows = Vec::new();
        for &(left, right) in &self.pairs {
            self.append_pair(&mut rows, left, right, left_width, right_width);
        }
        rows
    }

    fn prepare(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(200);
        self.indents.resize(self.lines.len(), 0);
        let mut start = 0;
        for end in 1..=self.lines.len() {
            if end == self.lines.len() || self.lines[end].kind == LineKind::Hunk {
                let indent = self.lines[start..end]
                    .iter()
                    .filter_map(|line| {
                        let code = if line.new.is_some() {
                            &line.after
                        } else {
                            &line.before
                        };
                        ((line.old.is_some() || line.new.is_some()) && !code.text.trim().is_empty())
                            .then(|| code.indent())
                    })
                    .min()
                    .unwrap_or(0);
                self.indents[start..end].fill(indent);
                start = end;
            }
        }
        let mut index = 0;
        while index < self.lines.len() {
            if self.lines[index].is_file_header() {
                index += 1;
                continue;
            }
            if matches!(self.lines[index].kind, LineKind::Removed | LineKind::Added) {
                let mut removed = Vec::new();
                let mut added = Vec::new();
                let mut old_notes = Vec::new();
                let mut new_notes = Vec::new();
                let mut previous = LineKind::Context;
                while index < self.lines.len()
                    && (matches!(self.lines[index].kind, LineKind::Removed | LineKind::Added)
                        || self.lines[index].before.text.starts_with("\\ No newline"))
                {
                    if self.lines[index].kind == LineKind::Removed {
                        removed.push(index);
                        previous = LineKind::Removed;
                    } else if self.lines[index].kind == LineKind::Added {
                        added.push(index);
                        previous = LineKind::Added;
                    } else if previous == LineKind::Removed {
                        old_notes.push(index);
                    } else {
                        new_notes.push(index);
                    }
                    index += 1;
                }
                let old: Vec<_> = removed
                    .iter()
                    .map(|&i| self.lines[i].before.text.as_str())
                    .collect();
                let new: Vec<_> = added
                    .iter()
                    .map(|&i| self.lines[i].after.text.as_str())
                    .collect();
                let pairs = align_lines(&old, &new, deadline);
                for (left, right) in pairs {
                    let left = left.map(|i| removed[i]);
                    let right = right.map(|i| added[i]);
                    if let (Some(left), Some(right)) = (left, right) {
                        let (old, new) = changed_words(
                            &self.lines[left].before.text,
                            &self.lines[right].after.text,
                            deadline,
                        );
                        self.lines[left]
                            .before
                            .emphasize(&old, Style::new().bg(Color::Rgb(100, 44, 54)).bold());
                        self.lines[right]
                            .after
                            .emphasize(&new, Style::new().bg(Color::Rgb(40, 88, 59)).bold());
                    }
                    self.pairs.push((left, right));
                }
                for pair in 0..old_notes.len().max(new_notes.len()) {
                    self.pairs
                        .push((old_notes.get(pair).copied(), new_notes.get(pair).copied()));
                }
            } else {
                self.pairs.push((Some(index), Some(index)));
                index += 1;
            }
        }
    }

    fn wrapped(&self, index: usize, before: bool, width: usize) -> Vec<Line<'static>> {
        let line = &self.lines[index];
        let code = if before { &line.before } else { &line.after };
        let trim = if self.full_indent {
            0
        } else {
            self.indents[index]
        };
        if line.kind == LineKind::Hunk && trim > 0 {
            Code::plain(format!("{}  [indent -{trim}]", code.text)).wrap(width)
        } else if line.old.is_some() || line.new.is_some() {
            code.review(width, trim, !self.hide_guides)
        } else {
            code.wrap(width)
        }
    }

    fn append_pair(
        &self,
        rows: &mut Vec<Row>,
        left: Option<usize>,
        right: Option<usize>,
        left_width: usize,
        right_width: usize,
    ) {
        let left_cells = left
            .map(|index| self.side_cells(index, true, left_width))
            .unwrap_or_default();
        let right_cells = right
            .map(|index| self.side_cells(index, false, right_width))
            .unwrap_or_default();
        let count = left_cells.len().max(right_cells.len());
        let mut left_cells = left_cells.into_iter();
        let mut right_cells = right_cells.into_iter();
        let anchor = left.or(right).unwrap_or(0);
        for part in 0..count {
            rows.push(Row {
                left: left_cells.next(),
                right: right_cells.next(),
                anchor,
                other: right,
                hunk: self.lines[anchor].kind == LineKind::Hunk && part == 0,
            });
        }
    }

    fn side_cells(&self, index: usize, before: bool, width: usize) -> Vec<Cell> {
        let line = &self.lines[index];
        let number = if before { line.old } else { line.new };
        self.wrapped(index, before, width)
            .into_iter()
            .enumerate()
            .map(|(part, code)| {
                let number = number.map(|n| n.to_string()).unwrap_or_default();
                let gutter = if part == 0 {
                    format!("{number:>5} {} ", marker(line.kind))
                } else {
                    "      > ".into()
                };
                Cell {
                    gutter,
                    code,
                    kind: line.kind,
                }
            })
            .collect()
    }
}

fn marker(kind: LineKind) -> char {
    match kind {
        LineKind::Added => '+',
        LineKind::Removed => '-',
        _ => ' ',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::parse_patch;

    fn fixture(patch: &str) -> DiffDisplay {
        let mut display = DiffDisplay::default();
        display.load(
            &parse_patch(patch),
            &Change {
                path: "main.rs".into(),
                status: 'M',
                old_path: None,
            },
            &Sources {
                old: None,
                new: None,
            },
        );
        display
    }

    #[test]
    fn pairs_replacements_and_pads_unbalanced_changes_and_wraps() {
        let mut display = fixture(
            "@@ -1,3 +1,2 @@\n-old call with many arguments and more words\n-extra\n+new call with arguments\n same\n",
        );
        assert_eq!(display.mode, ViewMode::Split);
        display.layout(60);
        let first = display.rows.iter().find(|row| row.anchor == 1).unwrap();
        assert!(first.left.as_ref().unwrap().gutter.contains('1'));
        assert!(first.right.as_ref().unwrap().gutter.contains('1'));
        assert!(
            display
                .rows
                .iter()
                .any(|row| row.anchor == 1 && row.right.is_none())
        );
        assert!(
            display
                .rows
                .iter()
                .any(|row| row.anchor == 2 && row.right.is_none())
        );
        let context = display.rows.last().unwrap();
        assert!(context.left.as_ref().unwrap().gutter.contains('3'));
        assert!(context.right.as_ref().unwrap().gutter.contains('2'));
    }

    #[test]
    fn unified_view_keeps_git_order_and_change_anchors() {
        let mut display = fixture("@@ -1,2 +1,2 @@\n-old\n+new\n same\n");
        display.mode = ViewMode::Unified;
        display.layout(90);
        assert_eq!(display.rows.len(), 4);
        assert_eq!(
            display.rows[1].left.as_ref().unwrap().kind,
            LineKind::Removed
        );
        assert_eq!(display.rows[2].left.as_ref().unwrap().kind, LineKind::Added);
        let anchor = display.anchor(2);
        display.mode = ViewMode::Split;
        display.layout(90);
        let row = &display.rows[display.locate(anchor)];
        assert!(row.left.is_some() && row.right.is_some());
        assert_eq!(display.rows.iter().filter(|row| row.hunk).count(), 1);
    }

    #[test]
    fn missing_final_newline_does_not_break_replacement_pairing() {
        let mut display = fixture(
            "@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
        );
        display.layout(120);
        let row = &display.rows[1];
        assert_eq!(row.left.as_ref().unwrap().kind, LineKind::Removed);
        assert_eq!(row.right.as_ref().unwrap().kind, LineKind::Added);
        assert!(display.rows[2].left.is_some());
        assert!(display.rows[2].right.is_some());
    }

    #[test]
    fn hides_file_headers_in_both_views_without_hiding_code_or_changing_anchors() {
        let mut display = fixture(
            "diff --git a/main.rs b/main.rs\nindex abc123..def456 100644\n--- a/main.rs\n+++ b/main.rs\n@@ -1,2 +1,2 @@\n--- code\n+++ code\n index is code here\n",
        );
        for mode in [ViewMode::Split, ViewMode::Unified] {
            display.mode = mode;
            display.layout(120);
            assert_eq!(display.rows[0].anchor, 4);
            assert!(display.rows[0].hunk);
            let text = display
                .rows
                .iter()
                .flat_map(|row| [&row.left, &row.right])
                .flatten()
                .map(|cell| cell.code.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            for header in [
                "diff --git",
                "index abc123",
                "--- a/main.rs",
                "+++ b/main.rs",
            ] {
                assert!(!text.contains(header), "{text}");
            }
            for code in ["-- code", "++ code", "index is code here"] {
                assert!(text.contains(code), "{text}");
            }
            assert_eq!(display.rows[display.locate(5)].anchor, 5);
            assert!(
                display.rows[display.locate(6)].anchor == 6
                    || display.rows[display.locate(6)].other == Some(6)
            );
        }
    }

    #[test]
    fn binary_and_mode_only_changes_keep_their_summary() {
        for summary in [
            "Binary files a/data.bin and b/data.bin differ\n",
            "old mode 100644\nnew mode 100755\n",
        ] {
            let mut display = fixture(&format!("diff --git a/main.rs b/main.rs\n{summary}"));
            for mode in [ViewMode::Split, ViewMode::Unified] {
                display.mode = mode;
                display.layout(120);
                assert!(!display.rows.is_empty());
                assert!(display.rows.iter().all(|row| row.anchor > 0));
            }
        }
    }
}
