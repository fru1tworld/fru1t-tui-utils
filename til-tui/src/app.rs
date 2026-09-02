use std::collections::{BTreeMap, HashSet, VecDeque};

use chrono::{Duration, Local, NaiveDate};
use ratatui::widgets::ListState;
use tui_input::Input;

use crate::action::{Action, Flow};
use crate::clipboard;
use crate::db::SqliteTilRepository;
use crate::domain::{TilEntry, validate_content};
use crate::error::{Error, Result};
use crate::output;
use crate::week::CalendarWeek;

const UNDO_LIMIT: usize = 5;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    Insert,
    Normal,
}

pub(crate) struct EditDialog {
    pub(crate) entry_id: i64,
    pub(crate) input: Input,
}

pub(crate) struct DateGroup {
    pub(crate) date: NaiveDate,
    pub(crate) entries: Vec<TilEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VisibleRow {
    Date {
        group_index: usize,
    },
    Entry {
        group_index: usize,
        entry_index: usize,
    },
}

#[derive(Clone)]
enum UndoOperation {
    DeleteCreated {
        entry_id: i64,
    },
    RestoreDeleted {
        entry: TilEntry,
    },
    RestoreContent {
        entry_id: i64,
        content: String,
    },
    RestoreRecordedAt {
        entry_id: i64,
        recorded_at: i64,
        date_to_show: NaiveDate,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowKey {
    Date(NaiveDate),
    Entry(i64),
}

pub(crate) struct App {
    repository: SqliteTilRepository,
    pub(crate) selected_date: NaiveDate,
    pub(crate) date_groups: Vec<DateGroup>,
    pub(crate) visible_rows: Vec<VisibleRow>,
    pub(crate) selection: ListState,
    pub(crate) mode: Mode,
    pub(crate) input: Input,
    pub(crate) edit_dialog: Option<EditDialog>,
    pub(crate) status: String,
    collapsed_dates: HashSet<NaiveDate>,
    undo_history: VecDeque<UndoOperation>,
    data_version: i64,
}

impl App {
    pub(crate) fn new(repository: SqliteTilRepository) -> Result<Self> {
        let today = Local::now().date_naive();
        let mut app = Self {
            repository,
            selected_date: today,
            date_groups: Vec::new(),
            visible_rows: Vec::new(),
            selection: ListState::default(),
            mode: Mode::Insert,
            input: Input::default(),
            edit_dialog: None,
            status: String::new(),
            collapsed_dates: HashSet::new(),
            undo_history: VecDeque::new(),
            data_version: 0,
        };
        app.refresh(Some(RowKey::Date(today)))?;
        Ok(app)
    }

    pub(crate) fn sync_external_changes(&mut self) -> Result<()> {
        let current_version = self.repository.data_version()?;
        if current_version != self.data_version {
            self.refresh(self.selected_row_key())?;
        }
        Ok(())
    }

    pub(crate) fn apply(&mut self, action: Action) -> Result<Flow> {
        match action {
            Action::Quit => return Ok(Flow::Quit),
            Action::Output => return Ok(Flow::Output(self.formatted_output())),
            Action::EnterInsert => self.mode = Mode::Insert,
            Action::EnterNormal => self.mode = Mode::Normal,
            Action::Input(request) => {
                self.input.handle(request);
            }
            Action::CommitInsert => {
                if let Some(output) = self.commit_input()? {
                    return Ok(Flow::Output(output));
                }
            }
            Action::Select(offset) => self.move_selection(offset),
            Action::ToggleCollapse => self.toggle_selected_date(),
            Action::BrowseWeek(offset) => self.browse_week(offset)?,
            Action::MoveEntryByDays(offset) => self.move_selected_entry_by_days(offset)?,
            Action::BrowseToday => self.browse_today()?,
            Action::OpenEdit => self.open_edit_dialog(),
            Action::Delete => self.delete_selected_entry()?,
            Action::Undo => self.undo()?,
            Action::Yank => self.copy_day_to_clipboard(),
            Action::PopupInput(request) => {
                if let Some(dialog) = &mut self.edit_dialog {
                    dialog.input.handle(request);
                }
            }
            Action::PopupCommit => self.commit_edit_dialog()?,
            Action::PopupCancel => {
                self.edit_dialog = None;
                self.status = "취소됨".into();
            }
        }
        Ok(Flow::Continue)
    }

    pub(crate) fn is_collapsed(&self, date: NaiveDate) -> bool {
        self.collapsed_dates.contains(&date)
    }

    pub(crate) fn selected_date_entries(&self) -> &[TilEntry] {
        self.date_groups
            .iter()
            .find(|group| group.date == self.selected_date)
            .map_or(&[], |group| group.entries.as_slice())
    }

    pub(crate) fn calendar_week(&self) -> CalendarWeek {
        CalendarWeek::containing(self.selected_date)
    }

    pub(crate) fn week_entry_count(&self) -> usize {
        self.date_groups
            .iter()
            .map(|group| group.entries.len())
            .sum()
    }

    fn selected_row(&self) -> Option<VisibleRow> {
        self.selection
            .selected()
            .and_then(|index| self.visible_rows.get(index))
            .copied()
    }

    fn selected_row_key(&self) -> Option<RowKey> {
        match self.selected_row()? {
            VisibleRow::Date { group_index } => self
                .date_groups
                .get(group_index)
                .map(|group| RowKey::Date(group.date)),
            VisibleRow::Entry {
                group_index,
                entry_index,
            } => self
                .date_groups
                .get(group_index)
                .and_then(|group| group.entries.get(entry_index))
                .map(|entry| RowKey::Entry(entry.id)),
        }
    }

    fn row_key(&self, row: VisibleRow) -> Option<RowKey> {
        match row {
            VisibleRow::Date { group_index } => self
                .date_groups
                .get(group_index)
                .map(|group| RowKey::Date(group.date)),
            VisibleRow::Entry {
                group_index,
                entry_index,
            } => self
                .date_groups
                .get(group_index)
                .and_then(|group| group.entries.get(entry_index))
                .map(|entry| RowKey::Entry(entry.id)),
        }
    }

    fn selected_entry_id(&self) -> Option<i64> {
        match self.selected_row_key()? {
            RowKey::Entry(entry_id) => Some(entry_id),
            RowKey::Date(_) => None,
        }
    }

    fn selected_entry(&self) -> Option<&TilEntry> {
        let VisibleRow::Entry {
            group_index,
            entry_index,
        } = self.selected_row()?
        else {
            return None;
        };
        self.date_groups
            .get(group_index)
            .and_then(|group| group.entries.get(entry_index))
    }

    fn refresh(&mut self, preferred_row: Option<RowKey>) -> Result<()> {
        let visible_week = self.calendar_week();
        let mut entries_by_date = BTreeMap::<NaiveDate, Vec<TilEntry>>::new();
        for entry in self
            .repository
            .entries_between(visible_week.start(), visible_week.end())?
        {
            let date = entry.recorded_date().ok_or_else(|| {
                Error::InvalidInput(format!("#{} 기록의 날짜를 변환할 수 없습니다", entry.id))
            })?;
            entries_by_date.entry(date).or_default().push(entry);
        }

        self.date_groups = self
            .calendar_week()
            .dates()
            .map(|date| DateGroup {
                date,
                entries: entries_by_date.remove(&date).unwrap_or_default(),
            })
            .collect();
        let available_dates = self
            .date_groups
            .iter()
            .map(|group| group.date)
            .collect::<HashSet<_>>();
        self.collapsed_dates
            .retain(|date| available_dates.contains(date));
        self.rebuild_visible_rows();
        self.data_version = self.repository.data_version()?;

        let preferred_row = preferred_row.unwrap_or(RowKey::Date(self.selected_date));
        let selected_index = self
            .visible_rows
            .iter()
            .position(|row| self.row_key(*row) == Some(preferred_row))
            .or_else(|| {
                self.visible_rows
                    .iter()
                    .position(|row| self.row_key(*row) == Some(RowKey::Date(self.selected_date)))
            })
            .or((!self.visible_rows.is_empty()).then_some(0));
        self.selection.select(selected_index);
        self.update_selected_date();
        Ok(())
    }

    fn rebuild_visible_rows(&mut self) {
        self.visible_rows.clear();
        for (group_index, group) in self.date_groups.iter().enumerate() {
            self.visible_rows.push(VisibleRow::Date { group_index });
            if self.collapsed_dates.contains(&group.date) {
                continue;
            }
            self.visible_rows
                .extend(group.entries.iter().enumerate().map(|(entry_index, _)| {
                    VisibleRow::Entry {
                        group_index,
                        entry_index,
                    }
                }));
        }
    }

    fn update_selected_date(&mut self) {
        let group_index = match self.selected_row() {
            Some(VisibleRow::Date { group_index } | VisibleRow::Entry { group_index, .. }) => {
                group_index
            }
            None => return,
        };
        if let Some(group) = self.date_groups.get(group_index) {
            self.selected_date = group.date;
        }
    }

    fn remember(&mut self, operation: UndoOperation) {
        if self.undo_history.len() >= UNDO_LIMIT {
            self.undo_history.pop_front();
        }
        self.undo_history.push_back(operation);
    }

    fn undo(&mut self) -> Result<()> {
        let Some(operation) = self.undo_history.back().cloned() else {
            self.status = "되돌릴 작업이 없어요".into();
            return Ok(());
        };

        let preferred_row = match operation {
            UndoOperation::DeleteCreated { entry_id } => {
                self.repository.delete(entry_id)?;
                RowKey::Date(self.selected_date)
            }
            UndoOperation::RestoreDeleted { ref entry } => {
                self.repository.restore(entry)?;
                RowKey::Entry(entry.id)
            }
            UndoOperation::RestoreContent {
                entry_id,
                ref content,
            } => {
                self.repository.update_content(entry_id, content)?;
                RowKey::Entry(entry_id)
            }
            UndoOperation::RestoreRecordedAt {
                entry_id,
                recorded_at,
                date_to_show,
            } => {
                self.repository.set_recorded_at(entry_id, recorded_at)?;
                self.selected_date = date_to_show;
                RowKey::Entry(entry_id)
            }
        };

        self.undo_history.pop_back();
        self.refresh(Some(preferred_row))?;
        self.status = format!("되돌림 (남은 {}개)", self.undo_history.len());
        Ok(())
    }

    fn move_selection(&mut self, offset: isize) {
        if self.visible_rows.is_empty() {
            return;
        }
        let row_count = self.visible_rows.len() as isize;
        let current_index = self.selection.selected().unwrap_or(0) as isize;
        let next_index = (current_index + offset).rem_euclid(row_count) as usize;
        self.selection.select(Some(next_index));
        self.update_selected_date();
    }

    fn set_selected_date_collapsed(&mut self, collapsed: bool) {
        let Some(row) = self.selected_row() else {
            return;
        };
        let group_index = match row {
            VisibleRow::Date { group_index } => group_index,
            VisibleRow::Entry { group_index, .. } if collapsed => {
                let date = self.date_groups[group_index].date;
                if let Some(index) = self
                    .visible_rows
                    .iter()
                    .position(|row| self.row_key(*row) == Some(RowKey::Date(date)))
                {
                    self.selection.select(Some(index));
                    self.update_selected_date();
                }
                return;
            }
            VisibleRow::Entry { .. } => return,
        };
        let date = self.date_groups[group_index].date;
        if collapsed {
            self.collapsed_dates.insert(date);
        } else {
            self.collapsed_dates.remove(&date);
        }
        self.rebuild_visible_rows();
        if let Some(index) = self
            .visible_rows
            .iter()
            .position(|row| self.row_key(*row) == Some(RowKey::Date(date)))
        {
            self.selection.select(Some(index));
        }
        self.status = if collapsed {
            format!("{date} 접음")
        } else {
            format!("{date} 펼침")
        };
    }

    fn toggle_selected_date(&mut self) {
        let Some(VisibleRow::Date { group_index }) = self.selected_row() else {
            return;
        };
        let date = self.date_groups[group_index].date;
        self.set_selected_date_collapsed(!self.collapsed_dates.contains(&date));
    }

    fn browse_week(&mut self, offset: i64) -> Result<()> {
        let week = self.calendar_week();
        let day_offset = self.selected_date - week.start();
        self.selected_date = week.shifted(offset).start() + day_offset;
        self.refresh(Some(RowKey::Date(self.selected_date)))?;
        self.status = self.calendar_week().label();
        Ok(())
    }

    fn browse_today(&mut self) -> Result<()> {
        let today = Local::now().date_naive();
        self.selected_date = today;
        self.collapsed_dates.remove(&today);
        self.refresh(Some(RowKey::Date(today)))?;
        self.status = "오늘".into();
        Ok(())
    }

    fn move_selected_entry_by_days(&mut self, offset: i64) -> Result<()> {
        let Some(entry_id) = self.selected_entry_id() else {
            self.status = "옮길 기록을 선택하세요".into();
            return Ok(());
        };
        let source_date = self.selected_date;
        let target_date = source_date + Duration::days(offset);
        let recorded_at = self.repository.move_to_date(entry_id, target_date)?;
        self.remember(UndoOperation::RestoreRecordedAt {
            entry_id,
            recorded_at,
            date_to_show: source_date,
        });

        self.selected_date = target_date;
        self.collapsed_dates.remove(&target_date);
        self.refresh(Some(RowKey::Entry(entry_id)))?;
        self.status = format!("{target_date}로 옮김 (u 되돌리기)");
        Ok(())
    }

    fn commit_input(&mut self) -> Result<Option<String>> {
        let input = self.input.value().trim().to_owned();
        self.input.reset();
        if input.is_empty() {
            return Ok(None);
        }
        if input == "out" {
            return Ok(Some(self.formatted_output()));
        }

        let content = validate_content(&input)?;
        let entry = self.repository.create_on(self.selected_date, content)?;
        self.remember(UndoOperation::DeleteCreated { entry_id: entry.id });
        self.collapsed_dates.remove(&self.selected_date);
        self.refresh(Some(RowKey::Entry(entry.id)))?;
        self.status = format!("{}에 추가됨", self.selected_date);
        Ok(None)
    }

    fn open_edit_dialog(&mut self) {
        let Some(entry) = self.selected_entry() else {
            self.status = "편집할 기록을 선택하세요".into();
            return;
        };
        self.edit_dialog = Some(EditDialog {
            entry_id: entry.id,
            input: Input::new(entry.content.clone()),
        });
        self.status = "Enter 저장 · Shift+Enter 개행 · Esc 취소".into();
    }

    fn commit_edit_dialog(&mut self) -> Result<()> {
        let Some(dialog) = self.edit_dialog.take() else {
            return Ok(());
        };
        let content = match validate_content(dialog.input.value()) {
            Ok(content) => content.to_owned(),
            Err(Error::InvalidInput(message)) => {
                self.status = message;
                self.edit_dialog = Some(dialog);
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let original_content = self
            .date_groups
            .iter()
            .flat_map(|group| &group.entries)
            .find(|entry| entry.id == dialog.entry_id)
            .map(|entry| entry.content.clone())
            .ok_or(Error::EntryNotFound(dialog.entry_id))?;

        self.repository.update_content(dialog.entry_id, &content)?;
        self.remember(UndoOperation::RestoreContent {
            entry_id: dialog.entry_id,
            content: original_content,
        });
        self.refresh(Some(RowKey::Entry(dialog.entry_id)))?;
        self.status = "수정됨 (u 되돌리기)".into();
        Ok(())
    }

    fn delete_selected_entry(&mut self) -> Result<()> {
        let Some(entry_id) = self.selected_entry_id() else {
            self.status = "삭제할 기록을 선택하세요".into();
            return Ok(());
        };
        let date = self.selected_date;
        let deleted_entry = self.repository.delete(entry_id)?;
        self.remember(UndoOperation::RestoreDeleted {
            entry: deleted_entry,
        });
        self.refresh(Some(RowKey::Date(date)))?;
        self.status = "삭제됨 (u 되돌리기)".into();
        Ok(())
    }

    fn copy_day_to_clipboard(&mut self) {
        let output = self.formatted_output();
        let entry_count = self.selected_date_entries().len();
        self.status = match clipboard::copy(&output) {
            Ok(()) => format!("{} 기록 {}개 복사됨", self.selected_date, entry_count),
            Err(message) => message,
        };
    }

    fn formatted_output(&self) -> String {
        output::format_day_as_markdown(self.selected_date, self.selected_date_entries())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn select_date(app: &mut App, date: NaiveDate) {
        app.selected_date = date;
        app.refresh(Some(RowKey::Date(date))).unwrap();
    }

    #[test]
    fn dates_are_first_level_rows_and_entries_are_second_level_rows() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        app.repository.create_on(date, "기록").unwrap();
        select_date(&mut app, date);

        let date_index = app
            .visible_rows
            .iter()
            .position(|row| app.row_key(*row) == Some(RowKey::Date(date)))
            .unwrap();
        assert!(matches!(
            app.visible_rows[date_index],
            VisibleRow::Date { .. }
        ));
        assert!(matches!(
            app.visible_rows[date_index + 1],
            VisibleRow::Entry { .. }
        ));
    }

    #[test]
    fn current_view_contains_monday_through_sunday_in_order() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        select_date(&mut app, NaiveDate::from_ymd_opt(2026, 9, 2).unwrap());

        assert_eq!(app.date_groups.len(), 7);
        assert_eq!(
            app.date_groups.first().unwrap().date,
            NaiveDate::from_ymd_opt(2026, 8, 31).unwrap()
        );
        assert_eq!(
            app.date_groups.last().unwrap().date,
            NaiveDate::from_ymd_opt(2026, 9, 6).unwrap()
        );
    }

    #[test]
    fn browsing_weeks_preserves_the_selected_weekday() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        select_date(&mut app, NaiveDate::from_ymd_opt(2026, 9, 2).unwrap());

        app.apply(Action::BrowseWeek(-1)).unwrap();

        assert_eq!(
            app.selected_date,
            NaiveDate::from_ymd_opt(2026, 8, 26).unwrap()
        );
        assert_eq!(app.calendar_week().label(), "2026년 8월 4주차");
        assert_eq!(
            app.selected_row_key(),
            Some(RowKey::Date(app.selected_date))
        );
    }

    #[test]
    fn collapsing_a_date_hides_only_its_entries() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        app.repository.create_on(date, "기록 1").unwrap();
        app.repository.create_on(date, "기록 2").unwrap();
        select_date(&mut app, date);
        let expanded_count = app.visible_rows.len();

        app.apply(Action::ToggleCollapse).unwrap();

        assert!(app.is_collapsed(date));
        assert_eq!(app.visible_rows.len(), expanded_count - 2);
        assert_eq!(app.selected_row_key(), Some(RowKey::Date(date)));
    }

    #[test]
    fn external_changes_sync_without_losing_the_collapsed_date_selection() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("til-tui-sync-{}-{unique}.db", std::process::id()));
        let repository = SqliteTilRepository::open(&path).unwrap();
        let external_repository = SqliteTilRepository::open(&path).unwrap();
        let mut app = App::new(repository).unwrap();
        let today = Local::now().date_naive();
        select_date(&mut app, today);
        app.apply(Action::ToggleCollapse).unwrap();

        let external_entry = external_repository
            .create_on(today, "외부에서 추가한 기록")
            .unwrap();
        app.sync_external_changes().unwrap();

        assert!(app.is_collapsed(today));
        assert_eq!(app.selected_row_key(), Some(RowKey::Date(today)));
        assert!(
            app.selected_date_entries()
                .iter()
                .any(|entry| entry.id == external_entry.id)
        );

        drop(app);
        drop(external_repository);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn undo_history_keeps_only_the_five_most_recent_changes() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        select_date(&mut app, date);
        for index in 0..6 {
            app.input = Input::new(format!("기록 {index}"));
            app.apply(Action::CommitInsert).unwrap();
        }

        for _ in 0..6 {
            app.apply(Action::Undo).unwrap();
        }

        let entries = app.repository.entries_on(date).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].content, "기록 0");
        assert_eq!(app.status, "되돌릴 작업이 없어요");
    }

    #[test]
    fn undoing_a_tui_create_does_not_remove_an_external_create() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        select_date(&mut app, date);
        app.input = Input::new("TUI 기록".into());
        app.apply(Action::CommitInsert).unwrap();

        let external_entry = app.repository.create_on(date, "CLI 기록").unwrap();
        app.apply(Action::Undo).unwrap();

        let entries = app.repository.entries_on(date).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, external_entry.id);
        assert_eq!(entries[0].content, "CLI 기록");
    }

    #[test]
    fn out_input_returns_the_selected_day_output_without_saving_out() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        select_date(&mut app, date);
        app.input = Input::new("out".into());

        let Flow::Output(output) = app.apply(Action::CommitInsert).unwrap() else {
            panic!("out 입력은 출력과 함께 앱을 끝내야 한다");
        };

        assert_eq!(output, "- 2026-08-31\n  - (기록 없음)\n");
        assert!(
            !app.selected_date_entries()
                .iter()
                .any(|entry| entry.content == "out")
        );
    }

    #[test]
    fn multiline_input_is_saved_as_one_entry() {
        let mut app = App::new(SqliteTilRepository::in_memory()).unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        select_date(&mut app, date);
        app.input = Input::new("Lens\n- Get-Set\n- Set-Get".into());

        app.apply(Action::CommitInsert).unwrap();

        assert_eq!(app.selected_date_entries().len(), 1);
        assert_eq!(
            app.selected_date_entries()[0].content,
            "Lens\n- Get-Set\n- Set-Get"
        );
    }
}
