use ratatui::widgets::ListState;
use tui_input::Input;

use crate::action::{Action, Flow};
use crate::clipboard;
use crate::db::{ParentCompletion, Project, ProjectId, RestoreOutcome, Store, Todo, TodoId};
use crate::dialog::{Popup, PopupKind};
use crate::error::{Error, Result};
use crate::tree::{self, MAX_DEPTH};
use crate::undo::{Snapshot, UndoHistory};

const UNDO_LIMIT: usize = 5;
pub(crate) const PROJECT_LIMIT: usize = 5;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    Insert,
    Normal,
}

pub(crate) struct App {
    pub(crate) store: Store,
    pub(crate) projects: Vec<Project>,
    pub(crate) active_project_id: ProjectId,
    pub(crate) todos: Vec<Todo>,
    pub(crate) visible: Vec<usize>,
    pub(crate) list_state: ListState,
    pub(crate) mode: Mode,
    pub(crate) input: Input,
    pub(crate) popup: Option<Popup>,
    pub(crate) status: String,
    pub(crate) tab_held: bool,
    undo_history: UndoHistory,
    data_version: i64,
}

fn rotate<Id: Copy + PartialEq>(order: &[Id], id: Id, delta: isize) -> Option<Vec<Id>> {
    let idx = order
        .iter()
        .position(|&x| x == id)
        .filter(|_| order.len() >= 2)?;
    let j = (idx as isize + delta).rem_euclid(order.len() as isize) as usize;
    let mut out: Vec<Id> = order.iter().copied().filter(|&x| x != id).collect();
    out.insert(j, id);
    Some(out)
}

fn required<'a>(text: &'a str, missing: &str) -> Result<&'a str> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::Invalid(missing.into()));
    }
    Ok(text)
}

impl App {
    pub(crate) fn new(store: Store) -> Result<Self> {
        let mut app = Self {
            store,
            projects: Vec::new(),
            active_project_id: ProjectId(0),
            todos: Vec::new(),
            visible: Vec::new(),
            list_state: ListState::default(),
            mode: Mode::Insert,
            input: Input::default(),
            popup: None,
            status: String::new(),
            tab_held: false,
            undo_history: UndoHistory::new(UNDO_LIMIT),
            data_version: 0,
        };
        app.reload()?;
        Ok(app)
    }

    pub(crate) fn sync(&mut self) -> Result<()> {
        let current_data_version = self.store.data_version()?;
        if current_data_version != self.data_version {
            self.undo_history.clear();
            self.reload()?;
            self.status = "외부 변경 동기화됨 · 되돌리기 기록 초기화".into();
        }
        Ok(())
    }

    pub(crate) fn hold_tab(&mut self) {
        self.tab_held = true;
        self.status = "←→ 메모를 옆 탭으로 보내기 · 다른 키를 누르면 해제".into();
    }

    pub(crate) fn apply(&mut self, action: Action) -> Result<Flow> {
        match action {
            Action::Quit => return Ok(Flow::Quit),
            Action::EnterInsert => self.mode = Mode::Insert,
            Action::EnterNormal => self.mode = Mode::Normal,
            Action::Input(req) => {
                self.input.handle(req);
            }
            Action::CommitInsert => self.commit_insert()?,
            Action::Select(delta) => self.move_selection(delta),
            Action::Reorder(delta) => self.move_selected(delta)?,
            Action::Indent => self.indent_selected()?,
            Action::Outdent => self.outdent_selected()?,
            Action::Collapse => self.set_collapsed(true)?,
            Action::Expand => self.set_collapsed(false)?,
            Action::ToggleDone => self.toggle_done()?,
            Action::Delete => self.delete_selected()?,
            Action::Undo => self.undo()?,
            Action::Yank => self.yank_selected(),
            Action::SelectProject(idx) => self.select_project(idx)?,
            Action::MoveProject(delta) => self.move_project(delta)?,
            Action::MoveToProject(delta) => self.move_to_project(delta)?,
            Action::OpenEdit => self.open_edit(),
            Action::OpenSubtask => self.open_subtask(),
            Action::OpenNewProject => self.open_new_project(),
            Action::OpenRenameProject => self.open_rename_project(),
            Action::DeleteProject => self.delete_project()?,
            Action::PopupInput(req) => {
                if let Some(popup) = &mut self.popup {
                    popup.input.handle(req);
                }
            }
            Action::PopupCommit => self.popup_commit()?,
            Action::PopupCancel => self.popup_cancel(),
        }
        Ok(Flow::Continue)
    }

    fn find(&self, id: TodoId) -> Option<&Todo> {
        tree::find(&self.todos, id)
    }

    pub(crate) fn depth_of(&self, id: TodoId) -> usize {
        tree::depth(&self.todos, id)
    }

    fn sibling_ids(&self, parent_id: Option<TodoId>) -> Vec<TodoId> {
        self.todos
            .iter()
            .filter(|t| t.parent_id == parent_id)
            .map(|t| t.id)
            .collect()
    }

    fn push_undo(&mut self) -> Result<()> {
        const SNAPSHOT_RETRY_LIMIT: usize = 3;

        for _ in 0..SNAPSHOT_RETRY_LIMIT {
            let data_version_before = self.store.data_version()?;
            let projects = self.store.list_projects()?;
            let todos = self.store.list_all_todos()?;
            let data_version_after = self.store.data_version()?;
            if data_version_before == data_version_after {
                self.undo_history.remember(Snapshot {
                    projects,
                    todos,
                    data_version: data_version_after,
                });
                return Ok(());
            }
        }

        Err(Error::Invalid(
            "외부 변경이 계속되어 되돌리기 지점을 만들 수 없어요".into(),
        ))
    }

    fn undo(&mut self) -> Result<()> {
        let Some(snapshot) = self.undo_history.latest() else {
            self.status = "되돌릴 작업이 없어요".into();
            return Ok(());
        };
        let outcome = self.store.restore_if_unchanged(
            &snapshot.projects,
            &snapshot.todos,
            snapshot.data_version,
        )?;
        match outcome {
            RestoreOutcome::Restored => {
                self.undo_history.discard_latest();
                self.reload()?;
                self.status = format!("되돌림 (남은 되돌리기 {}개)", self.undo_history.len());
            }
            RestoreOutcome::ChangedExternally => {
                self.undo_history.clear();
                self.reload()?;
                self.status = "외부 변경이 있어 되돌리기를 취소했어요".into();
            }
        }
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.projects = self.store.list_projects()?;
        if self.projects.is_empty() {
            self.store.add_project("기본")?;
            self.projects = self.store.list_projects()?;
        }
        if !self.projects.iter().any(|p| p.id == self.active_project_id) {
            self.active_project_id = self.projects[0].id;
        }
        let prev = self.selected_id();
        self.todos = self.store.list_todos(self.active_project_id)?;
        self.data_version = self.store.data_version()?;
        self.rebuild_visible();
        self.select_id_or_keep(prev);
        Ok(())
    }

    fn rebuild_visible(&mut self) {
        self.visible = self
            .todos
            .iter()
            .enumerate()
            .filter(|(_, t)| !tree::ancestors(&self.todos, t.id).any(|a| a.collapsed))
            .map(|(i, _)| i)
            .collect();
    }

    fn select_id_or_keep(&mut self, id: Option<TodoId>) {
        let idx = id
            .and_then(|id| self.visible.iter().position(|&i| self.todos[i].id == id))
            .or_else(|| {
                (!self.visible.is_empty()).then(|| {
                    self.list_state
                        .selected()
                        .unwrap_or(0)
                        .min(self.visible.len() - 1)
                })
            });
        self.list_state.select(idx);
    }

    fn selected(&self) -> Option<&Todo> {
        let row = self.list_state.selected()?;
        let &i = self.visible.get(row)?;
        self.todos.get(i)
    }

    fn selected_id(&self) -> Option<TodoId> {
        self.selected().map(|t| t.id)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let len = self.visible.len() as isize;
        let cur = self.list_state.selected().unwrap_or(0) as isize;
        self.list_state
            .select(Some((cur + delta).rem_euclid(len) as usize));
    }

    fn move_selected(&mut self, delta: isize) -> Result<()> {
        let Some(cur) = self.selected() else {
            return Ok(());
        };
        let (id, parent_id, done) = (cur.id, cur.parent_id, cur.done);

        let group: Vec<TodoId> = self
            .todos
            .iter()
            .filter(|t| t.parent_id == parent_id && t.done == done)
            .map(|t| t.id)
            .collect();
        let Some(order) = rotate(&group, id, delta) else {
            return Ok(());
        };

        self.push_undo()?;
        self.store.set_todo_positions(&order)?;
        self.reload()?;
        self.status = "순서 이동됨".into();
        Ok(())
    }

    fn toggle_done(&mut self) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let updates = tree::completion_updates(&self.todos, t.id, !t.done);

        self.push_undo()?;
        self.store.set_done_many(&updates)?;
        self.reload()?;
        Ok(())
    }

    fn set_collapsed(&mut self, collapsed: bool) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let id = t.id;
        if tree::children(&self.todos, id).next().is_none() || t.collapsed == collapsed {
            return Ok(());
        }
        self.store.set_collapsed(id, collapsed)?;
        self.reload()
    }

    fn indent_selected(&mut self) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let (id, done) = (t.id, t.done);

        let siblings = self.sibling_ids(t.parent_id);
        let Some(idx) = siblings.iter().position(|&x| x == id) else {
            return Ok(());
        };
        let Some(&new_parent) = idx.checked_sub(1).and_then(|above| siblings.get(above)) else {
            self.status = "위에 넣을 형제 항목이 없어요".into();
            return Ok(());
        };
        let deepest_depth_after_indent = self.depth_of(id) + tree::subtree_height(&self.todos, id);
        if deepest_depth_after_indent >= MAX_DEPTH {
            self.status = format!("{MAX_DEPTH}단계까지만 넣을 수 있어요");
            return Ok(());
        }
        let parent_completion = if done {
            ParentCompletion::Keep
        } else {
            ParentCompletion::Reopen
        };

        self.push_undo()?;
        self.store.indent(id, new_parent, parent_completion)?;
        self.reload()?;
        self.select_id_or_keep(Some(id));
        self.status = "하위로 넣음".into();
        Ok(())
    }

    fn outdent_selected(&mut self) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let Some(parent_id) = t.parent_id else {
            self.status = "이미 최상위 항목이에요".into();
            return Ok(());
        };
        let id = t.id;
        let grandparent_id = self.find(parent_id).and_then(|p| p.parent_id);

        let mut order = self.sibling_ids(grandparent_id);
        let Some(parent_idx) = order.iter().position(|&x| x == parent_id) else {
            return Ok(());
        };
        order.insert(parent_idx + 1, id);

        self.push_undo()?;
        self.store.outdent(id, grandparent_id, &order)?;
        self.reload()?;
        self.select_id_or_keep(Some(id));
        self.status = "한 단계 위로 뺌".into();
        Ok(())
    }

    fn delete_selected(&mut self) -> Result<()> {
        if let Some(id) = self.selected_id() {
            self.push_undo()?;
            self.store.delete_todo(id)?;
            self.reload()?;
            self.status = "삭제됨 (u 되돌리기)".into();
        }
        Ok(())
    }

    fn yank_selected(&mut self) {
        let Some(id) = self.selected_id() else {
            self.status = "복사할 항목이 없어요".into();
            return;
        };
        let text = self.yank_text(id);
        let line_count = text.lines().count();
        self.status = match clipboard::copy(&text) {
            Ok(()) => format!("복사됨 ({line_count}줄)"),
            Err(msg) => msg,
        };
    }

    fn yank_text(&self, id: TodoId) -> String {
        let base_depth = self.depth_of(id);
        std::iter::once(id)
            .chain(tree::descendant_ids(&self.todos, id))
            .filter_map(|i| self.find(i))
            .map(|t| self.yank_line(t, base_depth))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn yank_line(&self, todo: &Todo, base_depth: usize) -> String {
        let indent = "  ".repeat(self.depth_of(todo.id).saturating_sub(base_depth));
        let check = if todo.done { "x" } else { " " };
        let text = todo.text.replace(['\n', '\r'], " ");
        format!("{indent}- [{check}] {text}")
    }

    fn commit_insert(&mut self) -> Result<()> {
        let text = self.input.value().trim().to_string();
        if !text.is_empty() {
            self.push_undo()?;
            let id = self.store.add_todo(&text, None, self.active_project_id)?;
            self.reload()?;
            self.select_id_or_keep(Some(id));
            self.status = "추가됨".into();
        }
        self.input.reset();
        Ok(())
    }

    fn active_project_index(&self) -> usize {
        self.projects
            .iter()
            .position(|p| p.id == self.active_project_id)
            .unwrap_or(0)
    }

    fn active_project(&self) -> &Project {
        &self.projects[self.active_project_index()]
    }

    fn neighbor_project(&self, delta: isize) -> Option<&Project> {
        let len = self.projects.len();
        (len >= 2)
            .then(|| {
                (self.active_project_index() as isize + delta).rem_euclid(len as isize) as usize
            })
            .and_then(|idx| self.projects.get(idx))
    }

    fn select_project(&mut self, idx: usize) -> Result<()> {
        let Some(p) = self.projects.get(idx) else {
            self.status = format!("{}번 프로젝트가 없어요", idx + 1);
            return Ok(());
        };
        self.active_project_id = p.id;
        let name = p.name.clone();
        self.list_state.select(None);
        self.reload()?;
        self.status = format!("프로젝트: {name}");
        Ok(())
    }

    fn move_project(&mut self, delta: isize) -> Result<()> {
        let ids: Vec<ProjectId> = self.projects.iter().map(|p| p.id).collect();
        let Some(order) = rotate(&ids, self.active_project_id, delta) else {
            self.status = "프로젝트가 하나뿐이에요".into();
            return Ok(());
        };

        self.push_undo()?;
        self.store.set_project_positions(&order)?;
        self.reload()?;
        self.status = "탭 순서 이동됨".into();
        Ok(())
    }

    fn move_to_project(&mut self, delta: isize) -> Result<()> {
        let Some(target) = self.neighbor_project(delta) else {
            self.status = "보낼 다른 프로젝트가 없어요".into();
            return Ok(());
        };
        let (target_id, target_name) = (target.id, target.name.clone());
        let Some(id) = self.selected_id() else {
            return Ok(());
        };
        let subtree: Vec<TodoId> = std::iter::once(id)
            .chain(tree::descendant_ids(&self.todos, id))
            .collect();

        self.push_undo()?;
        self.store.move_to_project(id, &subtree, target_id)?;
        self.reload()?;
        self.status = format!("'{target_name}' 프로젝트로 보냄 (u 되돌리기)");
        Ok(())
    }

    fn delete_project(&mut self) -> Result<()> {
        if self.projects.len() <= 1 {
            self.status = "마지막 프로젝트는 삭제할 수 없어요".into();
            return Ok(());
        }
        let name = self.active_project().name.clone();
        self.push_undo()?;
        self.store.delete_project(self.active_project_id)?;
        self.reload()?;
        self.status = format!("프로젝트 '{name}' 삭제됨 (u 되돌리기)");
        Ok(())
    }

    fn open_popup(&mut self, kind: PopupKind, initial_text: String) {
        self.status = "Enter 저장  Esc 취소".into();
        self.popup = Some(Popup {
            kind,
            input: Input::new(initial_text),
        });
    }

    fn open_edit(&mut self) {
        if let Some(t) = self.selected() {
            self.open_popup(PopupKind::EditTodo { id: t.id }, t.text.clone());
        }
    }

    fn open_subtask(&mut self) {
        let Some(t) = self.selected() else {
            return;
        };
        let parent_id = if tree::accepts_child(&self.todos, t.id) {
            t.id
        } else {
            t.parent_id.unwrap_or(t.id)
        };
        self.open_popup(PopupKind::AddSubtask { parent_id }, String::new());
    }

    fn open_new_project(&mut self) {
        if self.projects.len() >= PROJECT_LIMIT {
            self.status = format!("프로젝트는 최대 {PROJECT_LIMIT}개까지예요");
            return;
        }
        self.open_popup(PopupKind::NewProject, String::new());
    }

    fn open_rename_project(&mut self) {
        let name = self.active_project().name.clone();
        let id = self.active_project_id;
        self.open_popup(PopupKind::RenameProject { id }, name);
    }

    fn popup_cancel(&mut self) {
        self.status = "취소됨".into();
        self.popup = None;
    }

    fn popup_commit(&mut self) -> Result<()> {
        let Some(popup) = self.popup.take() else {
            return Ok(());
        };
        let text = popup.input.value();
        let committed = match popup.kind {
            PopupKind::EditTodo { id } => self.commit_edit(id, text),
            PopupKind::AddSubtask { parent_id } => self.commit_subtask(parent_id, text),
            PopupKind::NewProject => self.commit_new_project(text),
            PopupKind::RenameProject { id } => self.commit_rename_project(id, text),
        };
        match committed {
            Ok(()) => Ok(()),
            Err(Error::Invalid(msg)) => {
                self.status = msg;
                self.popup = Some(popup);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn commit_subtask(&mut self, parent_id: TodoId, text: &str) -> Result<()> {
        let text = required(text, "내용을 입력하세요")?;
        self.push_undo()?;
        let id = self.store.add_subtask(text, parent_id)?;
        self.reload()?;
        self.select_id_or_keep(Some(id));
        self.status = "하위 목표 추가됨".into();
        Ok(())
    }

    fn commit_edit(&mut self, id: TodoId, text: &str) -> Result<()> {
        let text = required(text, "내용을 입력하세요")?;
        self.push_undo()?;
        self.store.update_todo_text(id, text)?;
        self.reload()?;
        self.status = "수정됨".into();
        Ok(())
    }

    fn commit_new_project(&mut self, name: &str) -> Result<()> {
        let name = required(name, "이름을 입력하세요")?;
        if self.projects.len() >= PROJECT_LIMIT {
            return Err(Error::Invalid(format!(
                "프로젝트는 최대 {PROJECT_LIMIT}개까지예요"
            )));
        }
        self.push_undo()?;
        self.active_project_id = self.store.add_project(name)?;
        self.list_state.select(None);
        self.reload()?;
        self.status = format!("프로젝트 '{name}' 추가됨");
        Ok(())
    }

    fn commit_rename_project(&mut self, id: ProjectId, name: &str) -> Result<()> {
        let name = required(name, "이름을 입력하세요")?;
        self.push_undo()?;
        self.store.rename_project(id, name)?;
        self.reload()?;
        self.status = "프로젝트 이름 변경됨".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_todos(n: usize) -> App {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        let pid = store.list_projects().unwrap()[0].id;
        for i in 0..n {
            store.add_todo(&format!("todo {i}"), None, pid).unwrap();
        }
        App::new(store).unwrap()
    }

    fn app_with_subtasks() -> App {
        let mut app = app_with_todos(2);
        let p0 = app.todos[0].id;
        let pid = app.active_project_id;
        app.store.add_todo("child A", Some(p0), pid).unwrap();
        app.store.add_todo("child B", Some(p0), pid).unwrap();
        app.reload().unwrap();
        app
    }

    fn texts(app: &App) -> Vec<String> {
        app.todos.iter().map(|t| t.text.clone()).collect()
    }

    #[test]
    fn insert_adds_todo() {
        let mut app = app_with_todos(0);
        app.input = Input::new("장보기".to_string());
        app.commit_insert().unwrap();
        let todos = app.store.list_todos(app.active_project_id).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].text, "장보기");
        assert!(app.input.value().is_empty());
    }

    #[test]
    fn edit_updates_selected_text() {
        let mut app = app_with_todos(2);
        app.move_selection(1);
        let id = app.selected_id().unwrap();
        app.commit_edit(id, "수정됨").unwrap();
        let todos = app.store.list_todos(app.active_project_id).unwrap();
        assert_eq!(todos[1].text, "수정됨");
    }

    #[test]
    fn move_selected_reorders_and_keeps_selection() {
        let mut app = app_with_todos(3);
        app.list_state.select(Some(0));
        app.move_selected(1).unwrap();
        assert_eq!(texts(&app), ["todo 1", "todo 0", "todo 2"]);
        assert_eq!(app.selected().unwrap().text, "todo 0");
        assert_eq!(app.list_state.selected(), Some(1));
    }

    #[test]
    fn move_selected_wraps_at_edges() {
        let mut app = app_with_todos(3);
        app.list_state.select(Some(0));
        app.move_selected(-1).unwrap();
        assert_eq!(texts(&app), ["todo 1", "todo 2", "todo 0"]);
        assert_eq!(app.selected().unwrap().text, "todo 0");

        app.move_selected(1).unwrap();
        assert_eq!(texts(&app), ["todo 0", "todo 1", "todo 2"]);
        assert_eq!(app.selected().unwrap().text, "todo 0");
    }

    #[test]
    fn toggle_done_sinks_and_restores() {
        let mut app = app_with_todos(3);
        app.list_state.select(Some(0));
        app.toggle_done().unwrap();
        assert_eq!(texts(&app), ["todo 1", "todo 2", "todo 0"]);
        assert_eq!(app.selected().unwrap().text, "todo 0");

        app.toggle_done().unwrap();
        assert_eq!(texts(&app), ["todo 0", "todo 1", "todo 2"]);
        assert_eq!(app.selected().unwrap().text, "todo 0");
    }

    #[test]
    fn reorder_stays_within_done_group() {
        let mut app = app_with_todos(3);
        app.list_state.select(Some(2));
        app.toggle_done().unwrap();
        assert_eq!(app.selected().unwrap().text, "todo 2");

        app.move_selected(-1).unwrap();
        assert_eq!(texts(&app), ["todo 0", "todo 1", "todo 2"]);
    }

    #[test]
    fn toggle_and_delete() {
        let mut app = app_with_todos(1);
        app.toggle_done().unwrap();
        assert!(app.selected().unwrap().done);
        app.delete_selected().unwrap();
        assert!(app.todos.is_empty());
    }

    #[test]
    fn subtask_moves_with_parent() {
        let mut app = app_with_subtasks();
        let p0 = app.todos[0].id;
        app.select_id_or_keep(Some(p0));
        app.move_selected(1).unwrap();
        assert_eq!(texts(&app), ["todo 1", "todo 0", "child A", "child B"]);
        assert_eq!(app.selected().unwrap().text, "todo 0");
    }

    #[test]
    fn subtask_reorders_only_among_siblings() {
        let mut app = app_with_subtasks();
        let a = app.todos.iter().find(|t| t.text == "child A").unwrap().id;
        app.select_id_or_keep(Some(a));
        app.move_selected(1).unwrap();
        assert_eq!(texts(&app), ["todo 0", "child B", "child A", "todo 1"]);
        app.move_selected(1).unwrap();
        assert_eq!(texts(&app), ["todo 0", "child A", "child B", "todo 1"]);
    }

    #[test]
    fn parent_stays_open_when_all_children_done() {
        let mut app = app_with_subtasks();
        let p0 = app.todos[0].id;
        let a = app.todos.iter().find(|t| t.text == "child A").unwrap().id;
        let b = app.todos.iter().find(|t| t.text == "child B").unwrap().id;

        app.select_id_or_keep(Some(a));
        app.toggle_done().unwrap();
        assert!(!app.find(p0).unwrap().done);

        app.select_id_or_keep(Some(b));
        app.toggle_done().unwrap();
        assert!(!app.find(p0).unwrap().done);
    }

    #[test]
    fn unchecking_child_reopens_done_parent() {
        let mut app = app_with_subtasks();
        let p0 = app.todos[0].id;
        let a = app.todos.iter().find(|t| t.text == "child A").unwrap().id;

        app.select_id_or_keep(Some(p0));
        app.toggle_done().unwrap();
        assert!(app.find(p0).unwrap().done);

        app.select_id_or_keep(Some(a));
        app.toggle_done().unwrap();
        assert!(!app.find(p0).unwrap().done);
    }

    #[test]
    fn unchecking_grandchild_reopens_ancestor_chain() {
        let mut app = app_with_todos(1);
        let top = app.todos[0].id;
        let pid = app.active_project_id;
        let mid = app.store.add_todo("mid", Some(top), pid).unwrap();
        let leaf = app.store.add_todo("leaf", Some(mid), pid).unwrap();
        app.reload().unwrap();

        app.select_id_or_keep(Some(top));
        app.toggle_done().unwrap();
        assert!(app.todos.iter().all(|t| t.done));

        app.select_id_or_keep(Some(leaf));
        app.toggle_done().unwrap();
        assert!(!app.find(top).unwrap().done);
        assert!(!app.find(mid).unwrap().done);
        assert!(!app.find(leaf).unwrap().done);
    }

    #[test]
    fn toggling_parent_cascades_to_children() {
        let mut app = app_with_subtasks();
        let p0 = app.todos[0].id;
        app.select_id_or_keep(Some(p0));
        app.toggle_done().unwrap();
        assert!(
            app.todos
                .iter()
                .filter(|t| t.parent_id == Some(p0))
                .all(|t| t.done)
        );
        assert!(app.find(p0).unwrap().done);
    }

    #[test]
    fn collapse_hides_children_from_visible() {
        let mut app = app_with_subtasks();
        let p0 = app.todos[0].id;
        assert_eq!(app.visible.len(), 4);
        app.select_id_or_keep(Some(p0));
        app.set_collapsed(true).unwrap();
        assert_eq!(app.visible.len(), 2);
        app.set_collapsed(false).unwrap();
        assert_eq!(app.visible.len(), 4);
    }

    #[test]
    fn collapsed_ancestor_hides_grandchildren() {
        let mut app = app_with_todos(1);
        let top = app.todos[0].id;
        let pid = app.active_project_id;
        let mid = app.store.add_todo("mid", Some(top), pid).unwrap();
        app.store.add_todo("leaf", Some(mid), pid).unwrap();
        app.reload().unwrap();
        assert_eq!(app.visible.len(), 3);

        app.select_id_or_keep(Some(top));
        app.set_collapsed(true).unwrap();
        assert_eq!(app.visible.len(), 1);
    }

    #[test]
    fn indent_nests_under_item_above() {
        let mut app = app_with_todos(2);
        let t1 = app.todos[1].id;
        app.select_id_or_keep(Some(t1));
        app.indent_selected().unwrap();
        assert_eq!(app.find(t1).unwrap().parent_id, Some(app.todos[0].id));
        assert_eq!(texts(&app), ["todo 0", "todo 1"]);
        assert_eq!(app.selected().unwrap().id, t1);
    }

    #[test]
    fn indent_to_third_level_allowed() {
        let mut app = app_with_subtasks();
        let b = app.todos.iter().find(|t| t.text == "child B").unwrap().id;
        app.select_id_or_keep(Some(b));
        app.indent_selected().unwrap();
        let a = app.todos.iter().find(|t| t.text == "child A").unwrap().id;
        assert_eq!(app.find(b).unwrap().parent_id, Some(a));
        assert_eq!(app.depth_of(b), 2);
    }

    #[test]
    fn indent_refused_beyond_third_level() {
        let mut app = app_with_subtasks();
        let b = app.todos.iter().find(|t| t.text == "child B").unwrap().id;
        app.select_id_or_keep(Some(b));
        app.indent_selected().unwrap();
        assert_eq!(app.depth_of(b), 2);

        let a = app.todos.iter().find(|t| t.text == "child A").unwrap().id;
        let pid = app.active_project_id;
        let c = app.store.add_todo("child C", Some(a), pid).unwrap();
        app.reload().unwrap();
        app.select_id_or_keep(Some(c));
        app.indent_selected().unwrap();
        assert_eq!(app.depth_of(c), 2);
        assert_eq!(app.find(c).unwrap().parent_id, Some(a));
    }

    #[test]
    fn indent_refused_at_top_of_siblings() {
        let mut app = app_with_todos(2);
        let t0 = app.todos[0].id;
        app.select_id_or_keep(Some(t0));
        app.indent_selected().unwrap();
        assert!(app.find(t0).unwrap().parent_id.is_none());
    }

    #[test]
    fn indent_refused_when_subtree_would_exceed_depth() {
        let mut app = app_with_todos(2);
        let pid = app.active_project_id;
        let t1 = app.todos[1].id;
        let mid = app.store.add_todo("mid", Some(t1), pid).unwrap();
        app.store.add_todo("leaf", Some(mid), pid).unwrap();
        app.reload().unwrap();

        app.select_id_or_keep(Some(t1));
        app.indent_selected().unwrap();
        assert!(app.find(t1).unwrap().parent_id.is_none());
    }

    #[test]
    fn outdent_promotes_child_after_parent_block() {
        let mut app = app_with_subtasks();
        let a = app.todos.iter().find(|t| t.text == "child A").unwrap().id;
        app.select_id_or_keep(Some(a));
        app.outdent_selected().unwrap();
        assert!(app.find(a).unwrap().parent_id.is_none());
        assert_eq!(texts(&app), ["todo 0", "child B", "child A", "todo 1"]);
        assert_eq!(app.selected().unwrap().id, a);
    }

    #[test]
    fn outdent_grandchild_moves_up_one_level() {
        let mut app = app_with_todos(1);
        let top = app.todos[0].id;
        let pid = app.active_project_id;
        let mid = app.store.add_todo("mid", Some(top), pid).unwrap();
        let leaf = app.store.add_todo("leaf", Some(mid), pid).unwrap();
        app.reload().unwrap();

        app.select_id_or_keep(Some(leaf));
        app.outdent_selected().unwrap();
        assert_eq!(app.find(leaf).unwrap().parent_id, Some(top));
        assert_eq!(texts(&app), ["todo 0", "mid", "leaf"]);
    }

    #[test]
    fn indent_refused_when_done_item_sank_below() {
        let mut app = app_with_todos(2);
        let t0 = app.todos[0].id;
        app.select_id_or_keep(Some(t0));
        app.toggle_done().unwrap();
        assert!(app.find(t0).unwrap().done);
        assert_eq!(app.todos[1].id, t0);

        let t1 = app.todos.iter().find(|t| t.text == "todo 1").unwrap().id;
        app.select_id_or_keep(Some(t1));
        app.indent_selected().unwrap();
        assert!(app.find(t1).unwrap().parent_id.is_none());
    }

    #[test]
    fn adding_subtask_reopens_completed_parent() {
        let mut app = app_with_todos(1);
        let p = app.todos[0].id;
        app.select_id_or_keep(Some(p));
        app.toggle_done().unwrap();
        assert!(app.find(p).unwrap().done);
        app.commit_subtask(p, "새 하위").unwrap();
        assert!(!app.find(p).unwrap().done);
    }

    #[test]
    fn yank_text_includes_subtree_as_checklist() {
        let mut app = app_with_subtasks();
        let parent = app.todos[0].id;
        let child_a = app.todos[1].id;
        app.store.set_done_many(&[(child_a, true)]).unwrap();
        app.store.update_todo_text(parent, "todo 0").unwrap();
        app.reload().unwrap();

        assert_eq!(
            app.yank_text(parent),
            "- [ ] todo 0\n  - [ ] child B\n  - [x] child A"
        );
    }

    #[test]
    fn yank_text_of_child_starts_at_zero_indent() {
        let app = app_with_subtasks();
        let child_a = app.todos[1].id;
        assert_eq!(app.yank_text(child_a), "- [ ] child A");
    }

    #[test]
    fn yank_line_flattens_newlines_in_text() {
        let mut app = app_with_todos(1);
        let id = app.todos[0].id;
        app.store.update_todo_text(id, "첫 줄\n둘째 줄").unwrap();
        app.reload().unwrap();
        assert_eq!(app.yank_text(id), "- [ ] 첫 줄 둘째 줄");
    }

    #[test]
    fn delete_keeps_cursor_position() {
        let mut app = app_with_todos(4);
        app.list_state.select(Some(2));
        app.delete_selected().unwrap();
        assert_eq!(app.list_state.selected(), Some(2));
        assert_eq!(app.selected().unwrap().text, "todo 3");

        app.delete_selected().unwrap();
        assert_eq!(app.list_state.selected(), Some(1));
        assert_eq!(app.selected().unwrap().text, "todo 1");
    }

    #[test]
    fn undo_restores_deleted_todo() {
        let mut app = app_with_subtasks();
        let p0 = app.todos[0].id;
        app.select_id_or_keep(Some(p0));
        app.delete_selected().unwrap();
        assert_eq!(texts(&app), ["todo 1"]);

        app.undo().unwrap();
        assert_eq!(texts(&app), ["todo 0", "child A", "child B", "todo 1"]);
    }

    #[test]
    fn undo_keeps_only_last_five() {
        let mut app = app_with_todos(0);
        for i in 0..7 {
            app.input = Input::new(format!("t{i}"));
            app.commit_insert().unwrap();
        }
        for _ in 0..5 {
            app.undo().unwrap();
        }
        assert_eq!(texts(&app), ["t0", "t1"]);

        app.undo().unwrap();
        assert_eq!(texts(&app), ["t0", "t1"]);
        assert_eq!(app.status, "되돌릴 작업이 없어요");
    }

    #[test]
    fn undo_after_toggle_restores_done_state() {
        let mut app = app_with_todos(1);
        app.toggle_done().unwrap();
        assert!(app.selected().unwrap().done);
        app.undo().unwrap();
        assert!(!app.selected().unwrap().done);
    }

    #[test]
    fn projects_switch_create_delete() {
        let mut app = app_with_todos(1);
        let p1 = app.active_project_id;
        app.commit_new_project("업무").unwrap();
        let p2 = app.active_project_id;
        assert_ne!(p1, p2);
        assert!(app.todos.is_empty());

        app.input = Input::new("회사 일".to_string());
        app.commit_insert().unwrap();
        assert_eq!(texts(&app), ["회사 일"]);

        app.select_project(0).unwrap();
        assert_eq!(app.active_project_id, p1);
        assert_eq!(texts(&app), ["todo 0"]);

        app.select_project(1).unwrap();
        assert_eq!(app.active_project_id, p2);

        app.delete_project().unwrap();
        assert_eq!(app.active_project_id, p1);
        assert_eq!(app.projects.len(), 1);

        app.delete_project().unwrap();
        assert_eq!(app.projects.len(), 1);
    }

    #[test]
    fn project_limit_is_five() {
        let mut app = app_with_todos(0);
        for i in 0..4 {
            app.commit_new_project(&format!("p{i}")).unwrap();
        }
        assert_eq!(app.projects.len(), 5);
        assert!(matches!(
            app.commit_new_project("p5"),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn move_to_project_carries_subtree() {
        let mut app = app_with_subtasks();
        app.commit_new_project("업무").unwrap();
        app.select_project(0).unwrap();
        let p0 = app.todos[0].id;

        app.select_id_or_keep(Some(p0));
        app.move_to_project(1).unwrap();
        assert_eq!(texts(&app), ["todo 1"]);

        app.select_project(1).unwrap();
        assert_eq!(texts(&app), ["todo 0", "child A", "child B"]);

        app.undo().unwrap();
        assert!(app.todos.is_empty());
        app.select_project(0).unwrap();
        assert_eq!(texts(&app), ["todo 0", "child A", "child B", "todo 1"]);
    }

    #[test]
    fn move_project_reorders_tabs_and_wraps() {
        let mut app = app_with_todos(0);
        app.commit_new_project("업무").unwrap();
        app.commit_new_project("사이드").unwrap();
        let names =
            |app: &App| -> Vec<String> { app.projects.iter().map(|p| p.name.clone()).collect() };
        assert_eq!(names(&app), ["기본", "업무", "사이드"]);
        assert_eq!(app.active_project_index(), 2);

        app.move_project(-1).unwrap();
        assert_eq!(names(&app), ["기본", "사이드", "업무"]);
        assert_eq!(app.active_project_index(), 1);

        app.move_project(-1).unwrap();
        assert_eq!(names(&app), ["사이드", "기본", "업무"]);
        app.move_project(-1).unwrap();
        assert_eq!(names(&app), ["기본", "업무", "사이드"]);

        app.undo().unwrap();
        assert_eq!(names(&app), ["사이드", "기본", "업무"]);
    }

    #[test]
    fn undo_restores_deleted_project() {
        let mut app = app_with_todos(1);
        app.commit_new_project("업무").unwrap();
        app.input = Input::new("회사 일".to_string());
        app.commit_insert().unwrap();

        app.delete_project().unwrap();
        assert_eq!(app.projects.len(), 1);

        app.undo().unwrap();
        assert_eq!(app.projects.len(), 2);
        let names: Vec<_> = app.projects.iter().map(|p| p.name.clone()).collect();
        assert!(names.contains(&"업무".to_string()));
    }

    #[test]
    fn undo_is_cancelled_when_an_external_writer_changed_the_database() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database_path =
            std::env::temp_dir().join(format!("todo-tui-undo-{}-{unique}.db", std::process::id()));
        let store = Store::open(&database_path).unwrap();
        let external_store = Store::open(&database_path).unwrap();
        let project_id = store.list_projects().unwrap()[0].id;
        let mut app = App::new(store).unwrap();

        app.input = Input::new("TUI 항목".into());
        app.commit_insert().unwrap();
        external_store
            .add_todo("외부 항목", None, project_id)
            .unwrap();
        app.undo().unwrap();

        let todo_texts = app
            .store
            .list_todos(project_id)
            .unwrap()
            .into_iter()
            .map(|todo| todo.text)
            .collect::<Vec<_>>();
        assert_eq!(todo_texts, ["TUI 항목", "외부 항목"]);
        assert_eq!(app.status, "외부 변경이 있어 되돌리기를 취소했어요");

        drop(app);
        drop(external_store);
        for path in [
            database_path.clone(),
            database_path.with_extension("db-wal"),
            database_path.with_extension("db-shm"),
        ] {
            let _ = std::fs::remove_file(path);
        }
    }
}
