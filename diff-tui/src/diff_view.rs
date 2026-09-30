use std::{
    collections::BTreeSet,
    iter,
    path::Path,
    time::{Duration, Instant},
};

use ratatui::{
    style::{Color, Style},
    text::Line,
};

use crate::{
    git::{Change, DiffLine, LineKind, Sources},
    matching::{align_lines, changed_words},
    syntax::{HighlightedSource, SyntaxEngine},
    wrap::{Code, IndentGuides},
};

pub const SPLIT_GUTTER: u16 = 8;
pub const UNIFIED_GUTTER: u16 = 14;
const SPLIT_CONTINUATION: &str = "      > ";
const UNIFIED_CONTINUATION: &str = "            > ";
const ALIGNMENT_BUDGET: Duration = Duration::from_millis(200);
const FILE_HEADER_PREFIXES: [&str; 8] = [
    "diff --git ",
    "index ",
    "--- ",
    "+++ ",
    "rename from ",
    "rename to ",
    "copy from ",
    "copy to ",
];

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

    pub fn toggled(self) -> Self {
        match self {
            Self::Split => Self::Unified,
            Self::Unified => Self::Split,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Indentation {
    #[default]
    Compact,
    Full,
}

impl Indentation {
    pub fn toggled(self) -> Self {
        match self {
            Self::Compact => Self::Full,
            Self::Full => Self::Compact,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Before,
    After,
}

impl Side {
    fn pick<T>(self, before: T, after: T) -> T {
        match self {
            Self::Before => before,
            Self::After => after,
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
    pub line: usize,
    pub right_line: Option<usize>,
    pub starts_hunk: bool,
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
            && FILE_HEADER_PREFIXES
                .iter()
                .any(|prefix| self.before.text.starts_with(prefix))
    }

    fn is_missing_newline_note(&self) -> bool {
        self.before.text.starts_with("\\ No newline")
    }

    fn displayed_code(&self) -> &Code {
        if self.new.is_some() {
            &self.after
        } else {
            &self.before
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct LayoutKey {
    width: u16,
    mode: ViewMode,
    indentation: Indentation,
    guides: IndentGuides,
}

#[derive(Default)]
pub struct DiffDisplay {
    pub mode: ViewMode,
    pub indentation: Indentation,
    pub guides: IndentGuides,
    pub rows: Vec<Row>,
    pub language: String,
    lines: Vec<PreparedLine>,
    pairs: Vec<(Option<usize>, Option<usize>)>,
    hunk_indents: Vec<usize>,
    laid_out_for: Option<LayoutKey>,
}

impl DiffDisplay {
    pub fn clear(&mut self) {
        self.rows.clear();
        self.lines.clear();
        self.pairs.clear();
        self.hunk_indents.clear();
        self.language.clear();
        self.laid_out_for = None;
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
        let mut warning = None;
        let mut highlight =
            |path: &Path, source: Option<&str>, numbers: BTreeSet<usize>| match source
                .map(|source| engine.highlight(path, source, &numbers))
                .transpose()
            {
                Ok(highlighted) => highlighted.unwrap_or_default(),
                Err(error) => {
                    warning = Some(format!(
                        "Syntax highlighting unavailable: {error}. Showing plain text."
                    ));
                    HighlightedSource::default()
                }
            };
        let before = highlight(
            change.old_path.as_deref().unwrap_or(&change.path),
            sources.old.as_deref(),
            patch.iter().filter_map(|line| line.old).collect(),
        );
        let after = highlight(
            &change.path,
            sources.new.as_deref(),
            patch.iter().filter_map(|line| line.new).collect(),
        );
        self.lines = patch
            .iter()
            .enumerate()
            .map(|(index, line)| prepare_line(line, &patch[index + 1..], change, [&before, &after]))
            .collect();
        self.hunk_indents = hunk_indents(&self.lines);
        self.pair_lines();
        warning
    }

    pub fn line_at_row(&self, row: usize) -> usize {
        self.rows.get(row).map_or(0, |row| row.line)
    }

    pub fn row_of_line(&self, line: usize) -> usize {
        self.rows
            .iter()
            .position(|row| row.line == line || row.right_line == Some(line))
            .or_else(|| self.rows.iter().position(|row| row.line > line))
            .unwrap_or(0)
    }

    pub fn layout(&mut self, width: u16) -> bool {
        let key = LayoutKey {
            width,
            mode: self.mode,
            indentation: self.indentation,
            guides: self.guides,
        };
        if self.laid_out_for == Some(key) {
            return false;
        }
        self.laid_out_for = Some(key);
        self.rows = match self.mode {
            ViewMode::Split => self.split_rows(width),
            ViewMode::Unified => self.unified_rows(width),
        };
        true
    }

    fn unified_rows(&self, width: u16) -> Vec<Row> {
        let width = usize::from(width.saturating_sub(2 + UNIFIED_GUTTER)).max(1);
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| !line.is_file_header())
            .flat_map(|(index, line)| {
                let side = if line.new.is_none() {
                    Side::Before
                } else {
                    Side::After
                };
                let old = number_label(line.old);
                let new = number_label(line.new);
                self.wrapped_code(index, side, width)
                    .into_iter()
                    .enumerate()
                    .map(move |(part, code)| {
                        let gutter = if part == 0 {
                            format!("{old:>5} {new:>5} {} ", marker(line.kind))
                        } else {
                            UNIFIED_CONTINUATION.into()
                        };
                        Row {
                            left: Some(Cell {
                                gutter,
                                code,
                                kind: line.kind,
                            }),
                            right: None,
                            line: index,
                            right_line: None,
                            starts_hunk: line.kind == LineKind::Hunk && part == 0,
                        }
                    })
            })
            .collect()
    }

    fn split_rows(&self, width: u16) -> Vec<Row> {
        let left_width = usize::from((width / 2).saturating_sub(2 + SPLIT_GUTTER)).max(1);
        let right_width = usize::from((width - width / 2).saturating_sub(2 + SPLIT_GUTTER)).max(1);
        let mut rows = Vec::new();
        for &(left, right) in &self.pairs {
            let left_cells = left
                .map(|index| self.side_cells(index, Side::Before, left_width))
                .unwrap_or_default();
            let right_cells = right
                .map(|index| self.side_cells(index, Side::After, right_width))
                .unwrap_or_default();
            let height = left_cells.len().max(right_cells.len());
            let mut left_cells = left_cells.into_iter();
            let mut right_cells = right_cells.into_iter();
            let line = left.or(right).unwrap_or(0);
            for part in 0..height {
                rows.push(Row {
                    left: left_cells.next(),
                    right: right_cells.next(),
                    line,
                    right_line: right,
                    starts_hunk: self.lines[line].kind == LineKind::Hunk && part == 0,
                });
            }
        }
        rows
    }

    fn side_cells(&self, index: usize, side: Side, width: usize) -> Vec<Cell> {
        let line = &self.lines[index];
        let number = number_label(side.pick(line.old, line.new));
        self.wrapped_code(index, side, width)
            .into_iter()
            .enumerate()
            .map(|(part, code)| Cell {
                gutter: if part == 0 {
                    format!("{number:>5} {} ", marker(line.kind))
                } else {
                    SPLIT_CONTINUATION.into()
                },
                code,
                kind: line.kind,
            })
            .collect()
    }

    fn wrapped_code(&self, index: usize, side: Side, width: usize) -> Vec<Line<'static>> {
        let line = &self.lines[index];
        let code = side.pick(&line.before, &line.after);
        let trim = match self.indentation {
            Indentation::Full => 0,
            Indentation::Compact => self.hunk_indents[index],
        };
        if line.kind == LineKind::Hunk && trim > 0 {
            Code::plain(format!("{}  [indent -{trim}]", code.text)).wrap(width)
        } else if line.kind.is_code() {
            code.wrap_indented(width, trim, self.guides)
        } else {
            code.wrap(width)
        }
    }

    fn pair_lines(&mut self) {
        let deadline = Instant::now() + ALIGNMENT_BUDGET;
        let mut index = 0;
        while let Some(line) = self.lines.get(index) {
            if line.is_file_header() {
                index += 1;
            } else if line.kind.is_change() {
                index = self.pair_change_block(index, deadline);
            } else {
                self.pairs.push((Some(index), Some(index)));
                index += 1;
            }
        }
    }

    fn pair_change_block(&mut self, start: usize, deadline: Instant) -> usize {
        let mut removed = Vec::new();
        let mut added = Vec::new();
        let mut old_notes = Vec::new();
        let mut new_notes = Vec::new();
        let mut previous = LineKind::Context;
        let mut end = start;
        while let Some(line) = self.lines.get(end)
            && (line.kind.is_change() || line.is_missing_newline_note())
        {
            match line.kind {
                LineKind::Removed => {
                    removed.push(end);
                    previous = LineKind::Removed;
                }
                LineKind::Added => {
                    added.push(end);
                    previous = LineKind::Added;
                }
                _ if previous == LineKind::Removed => old_notes.push(end),
                _ => new_notes.push(end),
            }
            end += 1;
        }
        let old: Vec<_> = removed
            .iter()
            .map(|&index| self.lines[index].before.text.as_str())
            .collect();
        let new: Vec<_> = added
            .iter()
            .map(|&index| self.lines[index].after.text.as_str())
            .collect();
        for (left, right) in align_lines(&old, &new, deadline) {
            let left = left.map(|position| removed[position]);
            let right = right.map(|position| added[position]);
            if let (Some(left), Some(right)) = (left, right) {
                self.emphasize_changed_words(left, right, deadline);
            }
            self.pairs.push((left, right));
        }
        for position in 0..old_notes.len().max(new_notes.len()) {
            self.pairs.push((
                old_notes.get(position).copied(),
                new_notes.get(position).copied(),
            ));
        }
        end
    }

    fn emphasize_changed_words(&mut self, removed: usize, added: usize, deadline: Instant) {
        let (old, new) = changed_words(
            &self.lines[removed].before.text,
            &self.lines[added].after.text,
            deadline,
        );
        self.lines[removed]
            .before
            .emphasize(&old, Style::new().bg(Color::Rgb(100, 44, 54)).bold());
        self.lines[added]
            .after
            .emphasize(&new, Style::new().bg(Color::Rgb(40, 88, 59)).bold());
    }
}

fn prepare_line(
    line: &DiffLine,
    following: &[DiffLine],
    change: &Change,
    [before, after]: [&HighlightedSource; 2],
) -> PreparedLine {
    let (before_code, after_code) = match line.kind {
        LineKind::Hunk => {
            let old_function = enclosing_function(before, following, Side::Before);
            let new_function = enclosing_function(after, following, Side::After);
            (
                hunk_header(&line.text, old_function.or(new_function)),
                hunk_header(&line.text, new_function.or(old_function)),
            )
        }
        LineKind::Header => {
            let move_summary = line
                .text
                .strip_prefix("similarity index ")
                .and_then(|similarity| change.move_summary(similarity));
            let text = move_summary.as_deref().unwrap_or(&line.text);
            (Code::plain(text), Code::plain(text))
        }
        LineKind::Context | LineKind::Added | LineKind::Removed => {
            let text = &line.text[1..];
            let highlighted_or_plain = |source: &HighlightedSource, number: Option<usize>| {
                highlight_matching_diff(source, number, text)
                    .cloned()
                    .unwrap_or_else(|| Code::plain(text))
            };
            (
                highlighted_or_plain(before, line.old),
                highlighted_or_plain(after, line.new),
            )
        }
    };
    PreparedLine {
        kind: line.kind,
        old: line.old,
        new: line.new,
        before: before_code,
        after: after_code,
    }
}

fn highlight_matching_diff<'a>(
    source: &'a HighlightedSource,
    number: Option<usize>,
    diff_text: &str,
) -> Option<&'a Code> {
    source
        .lines
        .get(&number?)
        .filter(|code| code.text == diff_text)
}

fn enclosing_function<'a>(
    source: &'a HighlightedSource,
    following: &[DiffLine],
    side: Side,
) -> Option<&'a str> {
    following
        .iter()
        .take_while(|line| line.kind != LineKind::Hunk)
        .filter(|line| line.kind.is_change())
        .find_map(|line| {
            let number = side.pick(line.old, line.new)?;
            highlight_matching_diff(source, Some(number), line.text.get(1..)?)?;
            Some(source.enclosing_functions.get(&number))
        })
        .flatten()
        .map(String::as_str)
}

fn hunk_header(header: &str, function: Option<&str>) -> Code {
    match function {
        Some(function) if !header.ends_with(function) => {
            let separator = if header.ends_with("@@") { " " } else { " / " };
            Code::plain(format!("{header}{separator}{function}"))
        }
        _ => Code::plain(header),
    }
}

fn hunk_indents(lines: &[PreparedLine]) -> Vec<usize> {
    lines
        .chunk_by(|_, next| next.kind != LineKind::Hunk)
        .flat_map(|hunk| {
            let indent = hunk
                .iter()
                .filter(|line| line.kind.is_code())
                .map(PreparedLine::displayed_code)
                .filter(|code| !code.text.trim().is_empty())
                .map(Code::indent)
                .min()
                .unwrap_or(0);
            iter::repeat_n(indent, hunk.len())
        })
        .collect()
}

fn number_label(number: Option<usize>) -> String {
    number.map(|number| number.to_string()).unwrap_or_default()
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
    use crate::git::{ChangeStatus, parse_patch};

    fn modified(path: &str) -> Change {
        Change {
            path: path.into(),
            status: ChangeStatus::Modified,
            old_path: None,
        }
    }

    fn display_for(patch: &str) -> DiffDisplay {
        let mut display = DiffDisplay::default();
        display.load(
            &parse_patch(patch),
            &modified("main.rs"),
            &Sources::default(),
        );
        display
    }

    #[test]
    fn hunk_headers_keep_git_context_and_show_each_sides_function() {
        let patch = parse_patch(
            "@@ -1,4 +1,4 @@ class Client\n class Client {\n-    fun before() {\n+    fun after() {\n     }\n }\n",
        );
        let change = modified("client.kt");
        let sources = Sources {
            old: Some("class Client {\n    fun before() {\n    }\n}\n".into()),
            new: Some("class Client {\n    fun after() {\n    }\n}\n".into()),
        };
        let mut display = DiffDisplay::default();
        assert_eq!(display.load(&patch, &change, &sources), None);
        assert_eq!(
            display.lines[0].before.text,
            "@@ -1,4 +1,4 @@ class Client / fun before() {"
        );
        assert_eq!(
            display.lines[0].after.text,
            "@@ -1,4 +1,4 @@ class Client / fun after() {"
        );
        for mode in [ViewMode::Split, ViewMode::Unified] {
            display.mode = mode;
            display.layout(250);
            assert!(display.rows[0].starts_hunk);
            assert!(
                display.rows[0]
                    .left
                    .as_ref()
                    .unwrap()
                    .code
                    .to_string()
                    .contains("fun before()")
            );
        }

        let stale = Sources {
            old: sources.old.map(|source| source.replace("before", "stale")),
            new: sources.new.map(|source| source.replace("after", "stale")),
        };
        assert_eq!(display.load(&patch, &change, &stale), None);
        assert_eq!(display.lines[0].before.text, patch[0].text);
        assert_eq!(display.lines[0].after.text, patch[0].text);
    }

    #[test]
    fn pairs_replacements_and_pads_unbalanced_changes_and_wraps() {
        let mut display = display_for(
            "@@ -1,3 +1,2 @@\n-old call with many arguments and more words\n-extra\n+new call with arguments\n same\n",
        );
        assert_eq!(display.mode, ViewMode::Split);
        display.layout(60);
        let first = display.rows.iter().find(|row| row.line == 1).unwrap();
        assert!(first.left.as_ref().unwrap().gutter.contains('1'));
        assert!(first.right.as_ref().unwrap().gutter.contains('1'));
        assert!(
            display
                .rows
                .iter()
                .any(|row| row.line == 1 && row.right.is_none())
        );
        assert!(
            display
                .rows
                .iter()
                .any(|row| row.line == 2 && row.right.is_none())
        );
        let context = display.rows.last().unwrap();
        assert!(context.left.as_ref().unwrap().gutter.contains('3'));
        assert!(context.right.as_ref().unwrap().gutter.contains('2'));
    }

    #[test]
    fn unified_view_keeps_git_order_and_change_anchors() {
        let mut display = display_for("@@ -1,2 +1,2 @@\n-old\n+new\n same\n");
        display.mode = ViewMode::Unified;
        display.layout(90);
        assert_eq!(display.rows.len(), 4);
        assert_eq!(
            display.rows[1].left.as_ref().unwrap().kind,
            LineKind::Removed
        );
        assert_eq!(display.rows[2].left.as_ref().unwrap().kind, LineKind::Added);
        let line = display.line_at_row(2);
        display.mode = ViewMode::Split;
        display.layout(90);
        let row = &display.rows[display.row_of_line(line)];
        assert!(row.left.is_some() && row.right.is_some());
        assert_eq!(display.rows.iter().filter(|row| row.starts_hunk).count(), 1);
    }

    #[test]
    fn missing_final_newline_does_not_break_replacement_pairing() {
        let mut display = display_for(
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
        let mut display = display_for(
            "diff --git a/main.rs b/main.rs\nindex abc123..def456 100644\n--- a/main.rs\n+++ b/main.rs\n@@ -1,2 +1,2 @@\n--- code\n+++ code\n index is code here\n",
        );
        for mode in [ViewMode::Split, ViewMode::Unified] {
            display.mode = mode;
            display.layout(120);
            assert_eq!(display.rows[0].line, 4);
            assert!(display.rows[0].starts_hunk);
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
            assert_eq!(display.rows[display.row_of_line(5)].line, 5);
            let added_row = &display.rows[display.row_of_line(6)];
            assert!(added_row.line == 6 || added_row.right_line == Some(6));
        }
    }

    #[test]
    fn binary_and_mode_only_changes_keep_their_summary() {
        for summary in [
            "Binary files a/data.bin and b/data.bin differ\n",
            "old mode 100644\nnew mode 100755\n",
        ] {
            let mut display = display_for(&format!("diff --git a/main.rs b/main.rs\n{summary}"));
            for mode in [ViewMode::Split, ViewMode::Unified] {
                display.mode = mode;
                display.layout(120);
                assert!(!display.rows.is_empty());
                assert!(display.rows.iter().all(|row| row.line > 0));
            }
        }
    }
}
