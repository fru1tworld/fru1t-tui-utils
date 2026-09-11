use anyhow::Result;
use ratatui::{
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
    widgets::ListState,
};

use crate::{
    diff_view::{DiffDisplay, ViewMode},
    file_tree::FileTree,
    git::{Change, Mode, Repository, Snapshot, clean},
    refresh::{Preview, Request, Update},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

pub struct BranchPicker {
    pub branches: Vec<String>,
    pub from: Option<String>,
    pub query: String,
    pub state: ListState,
}

impl BranchPicker {
    pub fn visible(&self) -> Vec<&String> {
        let query = self.query.to_lowercase();
        self.branches
            .iter()
            .filter(|name| name.to_lowercase().contains(&query))
            .collect()
    }

    fn move_selection(&mut self, delta: isize) {
        let last = self.visible().len().saturating_sub(1);
        self.state.select(Some(
            self.state
                .selected()
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(last),
        ));
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
    pub visible: Vec<usize>,
    pub selected_file: Option<usize>,
    pub tree: FileTree,
    pub show_tests: bool,
    pub ignore_whitespace: bool,
    pub query: String,
    pub searching: bool,
    pub focus: Focus,
    pub preview: Preview,
    pub display: DiffDisplay,
    pub scroll: usize,
    pub horizontal: u16,
    pub page_height: usize,
    pub dialog: Option<Dialog>,
    pub error: Option<String>,
    pub refresh_error: Option<String>,
    generation: u64,
    diff_width: u16,
}

impl App {
    pub fn new(repo: Repository, mode: Mode, show_tests: bool) -> Result<Self> {
        let snapshot = repo.snapshot(&mode)?;
        Self::from_snapshot(repo, mode, show_tests, snapshot)
    }

    pub(crate) fn from_snapshot(
        repo: Repository,
        mode: Mode,
        show_tests: bool,
        snapshot: Snapshot,
    ) -> Result<Self> {
        let mut app = Self {
            repo,
            mode,
            snapshot,
            show_tests,
            ignore_whitespace: false,
            visible: Vec::new(),
            selected_file: None,
            tree: FileTree::default(),
            query: String::new(),
            searching: false,
            focus: Focus::Files,
            preview: Preview::default(),
            display: DiffDisplay::default(),
            scroll: 0,
            horizontal: 0,
            page_height: 20,
            dialog: None,
            error: None,
            refresh_error: None,
            generation: 0,
            diff_width: 80,
        };
        app.refilter(None)?;
        Ok(app)
    }

    pub fn selected(&self) -> Option<&Change> {
        self.selected_file
            .and_then(|i| self.visible.get(i))
            .map(|i| &self.snapshot.changes[*i])
    }

    pub fn hidden_count(&self) -> usize {
        if self.show_tests {
            0
        } else {
            self.snapshot
                .changes
                .iter()
                .filter(|change| change.is_test())
                .count()
        }
    }

    fn refilter(&mut self, keep: Option<Change>) -> Result<()> {
        self.select_visible(keep.as_ref());
        self.sync_tree(true);
        self.load_patch()
    }

    fn select_visible(&mut self, keep: Option<&Change>) {
        let query = self.query.to_lowercase();
        self.visible = self
            .snapshot
            .changes
            .iter()
            .enumerate()
            .filter(|(_, change)| {
                (self.show_tests || !change.is_test())
                    && change.label().to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        let index = keep
            .and_then(|change| {
                self.visible
                    .iter()
                    .position(|i| self.snapshot.changes[*i].path == change.path)
            })
            .unwrap_or_else(|| {
                self.selected_file
                    .unwrap_or(0)
                    .min(self.visible.len().saturating_sub(1))
            });
        self.selected_file = (!self.visible.is_empty()).then_some(index);
    }

    fn sync_tree(&mut self, reveal: bool) {
        let selected = self.selected().map(|change| change.path.clone());
        self.tree.rebuild(
            &self.snapshot.changes,
            &self.visible,
            !self.query.is_empty(),
            selected.as_deref(),
            reveal,
        );
    }

    fn load_patch(&mut self) -> Result<()> {
        self.generation = self.generation.wrapping_add(1);
        let change = self.selected().cloned();
        let preview = change
            .as_ref()
            .map(|change| Preview::read(&self.repo, &self.snapshot, change, self.ignore_whitespace))
            .transpose()?
            .unwrap_or_default();
        self.preview = preview;
        self.scroll = 0;
        self.horizontal = 0;
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
            ignore_whitespace: self.ignore_whitespace,
        }
    }

    pub fn apply_refresh(&mut self, request: Request, update: Result<Update>) {
        // A result must not overwrite a file, filter, or comparison selected after it started.
        if request.generation != self.generation {
            return;
        }
        self.refresh_error = update
            .and_then(|update| self.apply_update(update))
            .err()
            .map(|error| clean(&format!("Auto refresh failed: {error:#}")));
    }

    fn apply_update(&mut self, update: Update) -> Result<()> {
        let keep = self.selected().cloned();
        let files_changed = self.snapshot.changes != update.snapshot.changes;
        self.snapshot = update.snapshot;
        if files_changed {
            self.select_visible(keep.as_ref());
            self.sync_tree(false);
        }
        let selected = self.selected().cloned();
        let Some((change, preview)) = update
            .preview
            .filter(|(change, _)| selected.as_ref() == Some(change))
        else {
            if selected.is_none() && keep.is_none() {
                return Ok(());
            }
            return self.load_patch();
        };
        if keep.as_ref() == Some(&change) && preview == self.preview {
            return Ok(());
        }

        let anchor = self.display.anchor(self.scroll);
        let part = self.scroll.saturating_sub(self.display.locate(anchor));
        let previous = self.preview.patch.get(anchor);
        let next_anchor = previous
            .and_then(|line| {
                preview
                    .patch
                    .iter()
                    .position(|candidate| candidate == line)
                    .or_else(|| {
                        preview.patch.iter().position(|candidate| {
                            candidate.kind == line.kind
                                && line.old.is_some()
                                && candidate.old == line.old
                        })
                    })
                    .or_else(|| {
                        preview.patch.iter().position(|candidate| {
                            candidate.kind == line.kind
                                && line.new.is_some()
                                && candidate.new == line.new
                        })
                    })
            })
            .unwrap_or(anchor.min(preview.patch.len().saturating_sub(1)));
        self.preview = preview;
        self.rebuild_display(Some(&change));
        self.display.layout(self.diff_width);
        self.scroll = (self.display.locate(next_anchor) + part).min(self.max_scroll());
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    fn move_file(&mut self, delta: isize) -> Result<()> {
        if !self.visible.is_empty() {
            let index = self
                .selected_file
                .unwrap_or(0)
                .saturating_add_signed(delta)
                .min(self.visible.len() - 1);
            if Some(index) != self.selected_file {
                self.selected_file = Some(index);
                self.load_patch()?;
                self.sync_tree(true);
            }
        }
        Ok(())
    }

    fn move_vertical(&mut self, delta: isize) -> Result<()> {
        if self.focus == Focus::Files {
            self.tree.move_by(delta);
            self.select_tree_file()
        } else {
            self.scroll = self
                .scroll
                .saturating_add_signed(delta)
                .min(self.max_scroll());
            Ok(())
        }
    }

    fn select_tree_file(&mut self) -> Result<()> {
        if let Some(change) = self.tree.current().and_then(|entry| entry.change) {
            let index = self.visible.iter().position(|&index| index == change);
            if self.selected_file != index {
                self.selected_file = index;
                self.load_patch()?;
            }
        }
        Ok(())
    }

    fn open_tree(&mut self, toggle: bool) -> Result<()> {
        if let Some(entry) = self.tree.current() {
            if entry.change.is_some() {
                self.select_tree_file()?;
                self.focus = Focus::Diff;
            } else if entry.collapsed {
                self.tree.expand();
                self.sync_tree(false);
            } else if toggle {
                self.tree.collapse_or_parent();
                self.sync_tree(false);
            } else {
                self.tree.move_by(1);
                self.select_tree_file()?;
            }
        }
        Ok(())
    }

    pub fn max_scroll(&self) -> usize {
        self.display.rows.len().saturating_sub(self.page_height)
    }

    pub fn layout_diff(&mut self, width: u16, height: u16) {
        self.diff_width = width;
        let anchor = self.display.anchor(self.scroll);
        self.page_height = usize::from(height.saturating_sub(2)).max(1);
        if self.display.layout(width) {
            self.scroll = self.display.locate(anchor);
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn jump_hunk(&mut self, forward: bool) {
        self.display.layout(self.diff_width);
        let mut indices = self
            .display
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.hunk)
            .map(|(i, _)| i);
        let target = if forward {
            indices.find(|i| *i > self.scroll)
        } else {
            indices.rfind(|i| *i < self.scroll)
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
        self.refilter(keep)
    }

    pub fn handle(&mut self, key: KeyEvent) -> Result<bool> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(true);
        }
        self.error = None;
        if let Some(dialog) = self.dialog.take() {
            return self.handle_dialog(dialog, key);
        }
        if self.searching {
            match key.code {
                KeyCode::Esc => {
                    self.searching = false;
                    self.query.clear();
                }
                KeyCode::Enter => {
                    self.searching = false;
                    return Ok(false);
                }
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.query.push(ch)
                }
                _ => return Ok(false),
            }
            self.refilter(self.selected().cloned())?;
            return Ok(false);
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('d') => self.move_vertical((self.page_height / 2).max(1) as isize)?,
                KeyCode::Char('u') => {
                    self.move_vertical(-((self.page_height / 2).max(1) as isize))?
                }
                _ => {}
            }
            return Ok(false);
        }
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == Focus::Files {
                    Focus::Diff
                } else {
                    Focus::Files
                }
            }
            KeyCode::Enter if self.focus == Focus::Files => self.open_tree(true)?,
            KeyCode::Right | KeyCode::Char('l') if self.focus == Focus::Files => {
                self.open_tree(false)?
            }
            KeyCode::Left | KeyCode::Char('h') if self.focus == Focus::Files => {
                self.tree.collapse_or_parent();
                self.sync_tree(false);
            }
            KeyCode::Char('h') => self.focus = Focus::Files,
            KeyCode::Down | KeyCode::Char('j') => self.move_vertical(1)?,
            KeyCode::Up | KeyCode::Char('k') => self.move_vertical(-1)?,
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.move_vertical(self.page_height as isize)?
            }
            KeyCode::PageUp => self.move_vertical(-(self.page_height as isize))?,
            KeyCode::Char('n') => self.move_file(1)?,
            KeyCode::Char('p') => self.move_file(-1)?,
            KeyCode::Home | KeyCode::Char('g') => self.move_vertical(isize::MIN)?,
            KeyCode::End | KeyCode::Char('G') => self.move_vertical(isize::MAX)?,
            KeyCode::Left => self.horizontal = self.horizontal.saturating_sub(8),
            KeyCode::Right => self.horizontal = self.horizontal.saturating_add(8),
            KeyCode::Char(']') => self.jump_hunk(true),
            KeyCode::Char('[') => self.jump_hunk(false),
            KeyCode::Char('v') => {
                self.display.mode = if self.display.mode == ViewMode::Split {
                    ViewMode::Unified
                } else {
                    ViewMode::Split
                };
            }
            KeyCode::Char('i') => {
                self.display.full_indent = !self.display.full_indent;
                self.horizontal = 0;
            }
            KeyCode::Char('I') => self.display.hide_guides = !self.display.hide_guides,
            KeyCode::Char('t') => {
                self.show_tests = !self.show_tests;
                self.refilter(self.selected().cloned())?;
            }
            KeyCode::Char('/') => {
                self.focus = Focus::Files;
                self.searching = true;
                self.query.clear();
                self.refilter(self.selected().cloned())?;
            }
            KeyCode::Esc => {
                self.query.clear();
                self.refilter(self.selected().cloned())?;
            }
            KeyCode::Char('r') | KeyCode::Char('W') => {
                if key.code == KeyCode::Char('W') {
                    self.ignore_whitespace = !self.ignore_whitespace;
                }
                self.generation = self.generation.wrapping_add(1);
                let request = self.refresh_request();
                let update = request.read(&self.repo);
                self.apply_refresh(request, update);
            }
            KeyCode::Char('w') => self.switch_mode(Mode::Working)?,
            KeyCode::Char('s') => self.switch_mode(Mode::Staged)?,
            KeyCode::Char('u') => self.switch_mode(Mode::Unstaged)?,
            KeyCode::Char('b') => {
                let branches = self.repo.branches()?;
                self.dialog = Some(Dialog::Branches(BranchPicker {
                    branches,
                    from: None,
                    query: String::new(),
                    state: ListState::default().with_selected(Some(0)),
                }));
            }
            KeyCode::Char('?') => self.dialog = Some(Dialog::Help),
            _ => {}
        }
        Ok(false)
    }

    fn handle_dialog(&mut self, dialog: Dialog, key: KeyEvent) -> Result<bool> {
        let Dialog::Branches(mut picker) = dialog else {
            if !matches!(
                key.code,
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter
            ) {
                self.dialog = Some(Dialog::Help);
            }
            return Ok(false);
        };
        match key.code {
            KeyCode::Esc => return Ok(false),
            KeyCode::Down => picker.move_selection(1),
            KeyCode::Up => picker.move_selection(-1),
            KeyCode::Enter => {
                let chosen = picker
                    .visible()
                    .get(picker.state.selected().unwrap_or(0))
                    .cloned()
                    .cloned();
                if let Some(chosen) = chosen {
                    if let Some(from) = picker.from {
                        self.switch_mode(Mode::Branches(from, chosen))?;
                        return Ok(false);
                    }
                    picker.from = Some(chosen);
                    picker.query.clear();
                    picker.state.select(Some(0));
                }
            }
            KeyCode::Backspace => {
                picker.query.pop();
                picker.state.select(Some(0));
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.query.push(ch);
                picker.state.select(Some(0));
            }
            _ => {}
        }
        self.dialog = Some(Dialog::Branches(picker));
        Ok(false)
    }
}
