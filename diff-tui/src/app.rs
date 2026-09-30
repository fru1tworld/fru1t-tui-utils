use anyhow::Result;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    widgets::ListState,
};

use crate::{
    diff_view::DiffDisplay,
    file_tree::{Entry, EntryKind, FileTree, FolderState, TreeCursor},
    git::{Change, ChangeIndex, DiffLine, Mode, Repository, Snapshot, Whitespace, escape_controls},
    refresh::{Preview, Request, Update},
};

const HORIZONTAL_SCROLL_STEP: u16 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

impl Focus {
    fn toggled(self) -> Self {
        match self {
            Self::Files => Self::Diff,
            Self::Diff => Self::Files,
        }
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum TestFiles {
    #[default]
    Hidden,
    Shown,
}

impl TestFiles {
    fn toggled(self) -> Self {
        match self {
            Self::Hidden => Self::Shown,
            Self::Shown => Self::Hidden,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyOutcome {
    Continue,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExpandedDirectory {
    Collapse,
    Descend,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HunkDirection {
    Next,
    Previous,
}

pub struct BranchPicker {
    pub branches: Vec<String>,
    pub from: Option<String>,
    pub query: String,
    pub state: ListState,
}

impl BranchPicker {
    fn new(branches: Vec<String>) -> Self {
        Self {
            branches,
            from: None,
            query: String::new(),
            state: ListState::default().with_selected(Some(0)),
        }
    }

    pub fn matching(&self) -> impl Iterator<Item = &str> {
        let query = self.query.to_lowercase();
        self.branches
            .iter()
            .filter(move |name| name.to_lowercase().contains(&query))
            .map(String::as_str)
    }

    fn selected_branch(&self) -> Option<String> {
        self.matching()
            .nth(self.state.selected().unwrap_or(0))
            .map(str::to_owned)
    }

    fn move_selection(&mut self, delta: isize) {
        let last = self.matching().count().saturating_sub(1);
        self.state.select(Some(
            self.state
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(last),
        ));
    }

    fn reset_selection(&mut self) {
        self.state.select(Some(0));
    }
}

pub enum Dialog {
    Help,
    Branches(BranchPicker),
}

pub struct App {
    pub repo: Repository,
    pub mode: Mode,
    pub snapshot: Snapshot,
    pub visible: Vec<ChangeIndex>,
    pub selected_file: Option<usize>,
    pub tree: FileTree,
    pub test_files: TestFiles,
    pub whitespace: Whitespace,
    pub query: String,
    pub searching: bool,
    pub focus: Focus,
    pub preview: Preview,
    pub display: DiffDisplay,
    pub scroll: usize,
    pub horizontal_scroll: u16,
    pub page_height: usize,
    pub dialog: Option<Dialog>,
    pub error: Option<String>,
    pub refresh_error: Option<String>,
    generation: u64,
    diff_width: u16,
}

impl App {
    pub fn new(repo: Repository, mode: Mode, test_files: TestFiles) -> Result<Self> {
        let snapshot = repo.snapshot(&mode)?;
        Self::from_snapshot(repo, mode, test_files, snapshot)
    }

    pub(crate) fn from_snapshot(
        repo: Repository,
        mode: Mode,
        test_files: TestFiles,
        snapshot: Snapshot,
    ) -> Result<Self> {
        let mut app = Self {
            repo,
            mode,
            snapshot,
            test_files,
            whitespace: Whitespace::default(),
            visible: Vec::new(),
            selected_file: None,
            tree: FileTree::default(),
            query: String::new(),
            searching: false,
            focus: Focus::Files,
            preview: Preview::default(),
            display: DiffDisplay::default(),
            scroll: 0,
            horizontal_scroll: 0,
            page_height: 20,
            dialog: None,
            error: None,
            refresh_error: None,
            generation: 0,
            diff_width: 80,
        };
        app.refilter_keeping(None)?;
        Ok(app)
    }

    pub fn selected(&self) -> Option<&Change> {
        let index = *self.visible.get(self.selected_file?)?;
        Some(self.snapshot.change(index))
    }

    pub fn hidden_test_count(&self) -> usize {
        match self.test_files {
            TestFiles::Shown => 0,
            TestFiles::Hidden => self
                .snapshot
                .changes
                .iter()
                .filter(|change| change.is_test())
                .count(),
        }
    }

    fn refilter(&mut self) -> Result<()> {
        let keep = self.selected().cloned();
        self.refilter_keeping(keep.as_ref())
    }

    fn refilter_keeping(&mut self, keep: Option<&Change>) -> Result<()> {
        self.select_visible(keep);
        self.sync_tree(TreeCursor::RevealSelected);
        self.load_preview()
    }

    fn select_visible(&mut self, keep: Option<&Change>) {
        let query = self.query.to_lowercase();
        self.visible = self
            .snapshot
            .changes
            .iter()
            .enumerate()
            .filter(|(_, change)| {
                (self.test_files == TestFiles::Shown || !change.is_test())
                    && change.label().to_lowercase().contains(&query)
            })
            .map(|(index, _)| ChangeIndex(index))
            .collect();
        let kept_position = keep.and_then(|kept| {
            self.visible
                .iter()
                .position(|&index| self.snapshot.change(index).path == kept.path)
        });
        let position = kept_position.unwrap_or_else(|| {
            self.selected_file
                .unwrap_or(0)
                .min(self.visible.len().saturating_sub(1))
        });
        self.selected_file = (!self.visible.is_empty()).then_some(position);
    }

    fn sync_tree(&mut self, cursor: TreeCursor) {
        let selected = self.selected().map(|change| change.path.clone());
        let folders = if self.query.is_empty() {
            FolderState::Remembered
        } else {
            FolderState::AllExpanded
        };
        self.tree.rebuild(
            &self.snapshot.changes,
            &self.visible,
            folders,
            selected.as_deref(),
            cursor,
        );
    }

    fn invalidate_pending_refresh(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn load_preview(&mut self) -> Result<()> {
        self.invalidate_pending_refresh();
        let change = self.selected().cloned();
        self.preview = match &change {
            Some(change) => Preview::read(&self.repo, &self.snapshot, change, self.whitespace)?,
            None => Preview::default(),
        };
        self.scroll = 0;
        self.horizontal_scroll = 0;
        self.rebuild_display(change.as_ref());
        Ok(())
    }

    fn rebuild_display(&mut self, change: Option<&Change>) {
        if let Some(change) = change {
            self.error = self
                .display
                .load(&self.preview.patch, change, &self.preview.sources)
                .or_else(|| self.preview.warning.clone());
        } else {
            self.display.clear();
        }
    }

    pub fn refresh_request(&self) -> Request {
        Request {
            generation: self.generation,
            mode: self.mode.clone(),
            selected: self.selected().cloned(),
            whitespace: self.whitespace,
        }
    }

    pub fn apply_refresh(&mut self, request: Request, update: Result<Update>) {
        let superseded = request.generation != self.generation;
        if superseded {
            return;
        }
        self.refresh_error = update
            .and_then(|update| self.apply_update(update))
            .err()
            .map(|error| escape_controls(&format!("Auto refresh failed: {error:#}")));
    }

    fn apply_update(&mut self, update: Update) -> Result<()> {
        let previous = self.selected().cloned();
        let files_changed = self.snapshot.changes != update.snapshot.changes;
        self.snapshot = update.snapshot;
        if files_changed {
            self.select_visible(previous.as_ref());
            self.sync_tree(TreeCursor::KeepCurrent);
        }
        let selected = self.selected().cloned();
        let Some((change, preview)) = update
            .preview
            .filter(|(change, _)| selected.as_ref() == Some(change))
        else {
            if selected.is_none() && previous.is_none() {
                return Ok(());
            }
            return self.load_preview();
        };
        if previous.as_ref() == Some(&change) && preview == self.preview {
            return Ok(());
        }
        self.replace_preview_keeping_position(&change, preview);
        Ok(())
    }

    fn replace_preview_keeping_position(&mut self, change: &Change, preview: Preview) {
        let line = self.display.line_at_row(self.scroll);
        let rows_into_line = self.scroll.saturating_sub(self.display.row_of_line(line));
        let next_line = self
            .preview
            .patch
            .get(line)
            .and_then(|previous| find_same_line(&preview.patch, previous))
            .unwrap_or(line.min(preview.patch.len().saturating_sub(1)));
        self.preview = preview;
        self.rebuild_display(Some(change));
        self.display.layout(self.diff_width);
        self.scroll = (self.display.row_of_line(next_line) + rows_into_line).min(self.max_scroll());
        self.invalidate_pending_refresh();
    }

    fn move_file(&mut self, delta: isize) -> Result<()> {
        let Some(last) = self.visible.len().checked_sub(1) else {
            return Ok(());
        };
        let position = self
            .selected_file
            .unwrap_or(0)
            .saturating_add_signed(delta)
            .min(last);
        if Some(position) != self.selected_file {
            self.selected_file = Some(position);
            self.load_preview()?;
            self.sync_tree(TreeCursor::RevealSelected);
        }
        Ok(())
    }

    fn move_vertical(&mut self, delta: isize) -> Result<()> {
        match self.focus {
            Focus::Files => {
                self.tree.move_by(delta);
                self.select_tree_file()
            }
            Focus::Diff => {
                self.scroll = self
                    .scroll
                    .saturating_add_signed(delta)
                    .min(self.max_scroll());
                Ok(())
            }
        }
    }

    fn select_tree_file(&mut self) -> Result<()> {
        let Some(change) = self.tree.current().and_then(Entry::change) else {
            return Ok(());
        };
        let position = self.visible.iter().position(|&index| index == change);
        if self.selected_file != position {
            self.selected_file = position;
            self.load_preview()?;
        }
        Ok(())
    }

    fn open_tree_entry(&mut self, on_expanded: ExpandedDirectory) -> Result<()> {
        let Some(kind) = self.tree.current().map(|entry| entry.kind) else {
            return Ok(());
        };
        match kind {
            EntryKind::File(_) => {
                self.select_tree_file()?;
                self.focus = Focus::Diff;
            }
            EntryKind::Directory { collapsed: true } => {
                self.tree.expand();
                self.sync_tree(TreeCursor::KeepCurrent);
            }
            EntryKind::Directory { collapsed: false } => match on_expanded {
                ExpandedDirectory::Collapse => self.collapse_tree_entry(),
                ExpandedDirectory::Descend => {
                    self.tree.move_by(1);
                    self.select_tree_file()?;
                }
            },
        }
        Ok(())
    }

    fn collapse_tree_entry(&mut self) {
        self.tree.collapse_or_parent();
        self.sync_tree(TreeCursor::KeepCurrent);
    }

    pub fn max_scroll(&self) -> usize {
        self.display.rows.len().saturating_sub(self.page_height)
    }

    pub fn layout_diff(&mut self, width: u16, height: u16) {
        self.diff_width = width;
        let line = self.display.line_at_row(self.scroll);
        self.page_height = usize::from(height.saturating_sub(2)).max(1);
        if self.display.layout(width) {
            self.scroll = self.display.row_of_line(line);
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn jump_hunk(&mut self, direction: HunkDirection) {
        self.display.layout(self.diff_width);
        let mut hunk_rows = self
            .display
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.starts_hunk)
            .map(|(index, _)| index);
        let target = match direction {
            HunkDirection::Next => hunk_rows.find(|&row| row > self.scroll),
            HunkDirection::Previous => hunk_rows.rfind(|&row| row < self.scroll),
        };
        if let Some(target) = target {
            self.scroll = target.min(self.max_scroll());
        }
        self.focus = Focus::Diff;
    }

    fn switch_mode(&mut self, mode: Mode) -> Result<()> {
        let snapshot = self.repo.snapshot(&mode)?;
        let keep = self.selected().cloned();
        self.snapshot = snapshot;
        self.mode = mode;
        self.refresh_error = None;
        self.refilter_keeping(keep.as_ref())
    }

    fn reload(&mut self) {
        self.invalidate_pending_refresh();
        let request = self.refresh_request();
        let update = request.read(&self.repo);
        self.apply_refresh(request, update);
    }

    fn half_page(&self) -> isize {
        (self.page_height / 2).max(1) as isize
    }

    pub fn handle(&mut self, key: KeyEvent) -> Result<KeyOutcome> {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('c') {
            return Ok(KeyOutcome::Quit);
        }
        self.error = None;
        if let Some(dialog) = self.dialog.take() {
            self.handle_dialog(dialog, key)?;
        } else if self.searching {
            self.handle_search_key(key)?;
        } else if control {
            self.handle_control_key(key.code)?;
        } else {
            return self.handle_key(key.code);
        }
        Ok(KeyOutcome::Continue)
    }

    fn handle_search_key(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.searching = false;
                self.query.clear();
            }
            KeyCode::Enter => {
                self.searching = false;
                return Ok(());
            }
            KeyCode::Backspace => {
                self.query.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.query.push(ch);
            }
            _ => return Ok(()),
        }
        self.refilter()
    }

    fn handle_control_key(&mut self, code: KeyCode) -> Result<()> {
        match code {
            KeyCode::Char('d') => self.move_vertical(self.half_page()),
            KeyCode::Char('u') => self.move_vertical(-self.half_page()),
            _ => Ok(()),
        }
    }

    fn handle_key(&mut self, code: KeyCode) -> Result<KeyOutcome> {
        let files_focused = self.focus == Focus::Files;
        match code {
            KeyCode::Char('q') => return Ok(KeyOutcome::Quit),
            KeyCode::Tab | KeyCode::BackTab => self.focus = self.focus.toggled(),
            KeyCode::Enter if files_focused => {
                self.open_tree_entry(ExpandedDirectory::Collapse)?;
            }
            KeyCode::Right | KeyCode::Char('l') if files_focused => {
                self.open_tree_entry(ExpandedDirectory::Descend)?;
            }
            KeyCode::Left | KeyCode::Char('h') if files_focused => self.collapse_tree_entry(),
            KeyCode::Char('h') => self.focus = Focus::Files,
            KeyCode::Down | KeyCode::Char('j') => self.move_vertical(1)?,
            KeyCode::Up | KeyCode::Char('k') => self.move_vertical(-1)?,
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.move_vertical(self.page_height as isize)?;
            }
            KeyCode::PageUp => self.move_vertical(-(self.page_height as isize))?,
            KeyCode::Char('n') => self.move_file(1)?,
            KeyCode::Char('p') => self.move_file(-1)?,
            KeyCode::Home | KeyCode::Char('g') => self.move_vertical(isize::MIN)?,
            KeyCode::End | KeyCode::Char('G') => self.move_vertical(isize::MAX)?,
            KeyCode::Left => {
                self.horizontal_scroll = self
                    .horizontal_scroll
                    .saturating_sub(HORIZONTAL_SCROLL_STEP);
            }
            KeyCode::Right => {
                self.horizontal_scroll = self
                    .horizontal_scroll
                    .saturating_add(HORIZONTAL_SCROLL_STEP);
            }
            KeyCode::Char(']') => self.jump_hunk(HunkDirection::Next),
            KeyCode::Char('[') => self.jump_hunk(HunkDirection::Previous),
            KeyCode::Char('v') => self.display.mode = self.display.mode.toggled(),
            KeyCode::Char('i') => {
                self.display.indentation = self.display.indentation.toggled();
                self.horizontal_scroll = 0;
            }
            KeyCode::Char('I') => self.display.guides = self.display.guides.toggled(),
            KeyCode::Char('t') => {
                self.test_files = self.test_files.toggled();
                self.refilter()?;
            }
            KeyCode::Char('/') => {
                self.focus = Focus::Files;
                self.searching = true;
                self.query.clear();
                self.refilter()?;
            }
            KeyCode::Esc => {
                self.query.clear();
                self.refilter()?;
            }
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('W') => {
                self.whitespace = self.whitespace.toggled();
                self.reload();
            }
            KeyCode::Char('w') => self.switch_mode(Mode::Working)?,
            KeyCode::Char('s') => self.switch_mode(Mode::Staged)?,
            KeyCode::Char('u') => self.switch_mode(Mode::Unstaged)?,
            KeyCode::Char('b') => {
                self.dialog = Some(Dialog::Branches(BranchPicker::new(self.repo.branches()?)));
            }
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            _ => {}
        }
        Ok(KeyOutcome::Continue)
    }

    fn handle_dialog(&mut self, dialog: Dialog, key: KeyEvent) -> Result<()> {
        match dialog {
            Dialog::Help => {
                let closes_help = matches!(
                    key.code,
                    KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
                );
                if !closes_help {
                    self.dialog = Some(Dialog::Help);
                }
                Ok(())
            }
            Dialog::Branches(picker) => self.handle_branch_picker(picker, key),
        }
    }

    fn handle_branch_picker(&mut self, mut picker: BranchPicker, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => return Ok(()),
            KeyCode::Down => picker.move_selection(1),
            KeyCode::Up => picker.move_selection(-1),
            KeyCode::Enter => {
                if let Some(chosen) = picker.selected_branch() {
                    match picker.from.take() {
                        Some(from) => return self.switch_mode(Mode::Branches { from, to: chosen }),
                        None => {
                            picker.from = Some(chosen);
                            picker.query.clear();
                            picker.reset_selection();
                        }
                    }
                }
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.reset_selection();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.query.push(ch);
                picker.reset_selection();
            }
            _ => {}
        }
        self.dialog = Some(Dialog::Branches(picker));
        Ok(())
    }
}

fn find_same_line(patch: &[DiffLine], line: &DiffLine) -> Option<usize> {
    patch
        .iter()
        .position(|candidate| candidate == line)
        .or_else(|| {
            let old = line.old?;
            patch
                .iter()
                .position(|candidate| candidate.kind == line.kind && candidate.old == Some(old))
        })
        .or_else(|| {
            let new = line.new?;
            patch
                .iter()
                .position(|candidate| candidate.kind == line.kind && candidate.new == Some(new))
        })
}
