use std::collections::VecDeque;

use ratatui::widgets::ListState;
use tui_input::Input;

use crate::action::{Action, Flow};
use crate::clipboard;
use crate::db::{Project, Store, Todo, parse_due};
use crate::error::{Error, Result};

/// 최근 몇 개의 작업까지 되돌릴 수 있는지.
const UNDO_LIMIT: usize = 5;
/// 프로젝트(탭) 최대 개수.
pub(crate) const PROJECT_LIMIT: usize = 5;
/// 트리 최대 깊이(0-based 최심 depth = MAX_DEPTH - 1).
pub(crate) const MAX_DEPTH: usize = 3;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Mode {
    Insert,
    Normal,
}

#[derive(Clone, Copy)]
pub(crate) enum PopupKind {
    Edit { id: i64 },
    Due { id: i64 },
    Subtask { parent_id: i64 },
    NewProject,
    RenameProject { id: i64 },
}

impl PopupKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PopupKind::Edit { .. } => "내용 편집 (Enter 저장 · Esc 취소)",
            PopupKind::Due { .. } => "마감 (YYYY-MM-DD, 비우면 해제)",
            PopupKind::Subtask { .. } => "하위 목표 (Enter 저장 · Esc 취소)",
            PopupKind::NewProject => "새 프로젝트 이름 (Enter 생성 · Esc 취소)",
            PopupKind::RenameProject { .. } => "프로젝트 이름 변경 (Enter 저장 · Esc 취소)",
        }
    }
}

pub(crate) struct Popup {
    pub(crate) kind: PopupKind,
    pub(crate) input: Input,
}

/// undo 스냅숏: 두 테이블의 전체 상태.
struct Snapshot {
    projects: Vec<Project>,
    todos: Vec<Todo>,
}

pub(crate) struct App {
    pub(crate) store: Store,
    pub(crate) projects: Vec<Project>,
    pub(crate) project_id: i64,
    pub(crate) todos: Vec<Todo>,
    pub(crate) visible: Vec<usize>,
    pub(crate) state: ListState,
    pub(crate) mode: Mode,
    pub(crate) input: Input,
    pub(crate) popup: Option<Popup>,
    pub(crate) status: String,
    /// Tab 키가 눌려 있는 동안 true. 화살표를 '탭으로 보내기'로 바꾼다(kitty 프로토콜 필요).
    pub(crate) tab_held: bool,
    undo_stack: VecDeque<Snapshot>,
    data_version: i64,
}

/// order 안에서 id를 delta만큼 옮긴 새 순서. 끝에서는 반대편으로 감긴다.
/// 항목이 둘 미만이거나 id가 없으면 None.
fn rotate(order: &[i64], id: i64, delta: isize) -> Option<Vec<i64>> {
    let idx = order
        .iter()
        .position(|&x| x == id)
        .filter(|_| order.len() >= 2)?;
    let j = (idx as isize + delta).rem_euclid(order.len() as isize) as usize;
    let mut out: Vec<i64> = order.iter().copied().filter(|&x| x != id).collect();
    out.insert(j, id);
    Some(out)
}

impl App {
    pub(crate) fn new(store: Store) -> Result<Self> {
        let mut app = Self {
            store,
            projects: Vec::new(),
            project_id: 0,
            todos: Vec::new(),
            visible: Vec::new(),
            state: ListState::default(),
            mode: Mode::Insert,
            input: Input::default(),
            popup: None,
            status: String::new(),
            tab_held: false,
            undo_stack: VecDeque::new(),
            data_version: 0,
        };
        app.reload()?;
        Ok(app)
    }

    pub(crate) fn sync(&mut self) -> Result<()> {
        let v = self.store.data_version()?;
        if v != self.data_version {
            self.reload()?;
        }
        Ok(())
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
            Action::Collapse(collapsed) => self.set_collapse(collapsed)?,
            Action::ToggleDone => self.toggle_done()?,
            Action::Delete => self.delete_selected()?,
            Action::Undo => self.undo()?,
            Action::Yank => self.yank_selected(),
            Action::ProjectSelect(idx) => self.select_project(idx)?,
            Action::MoveProject(delta) => self.move_project(delta)?,
            Action::MoveToProject(delta) => self.move_to_project(delta)?,
            Action::OpenEdit => self.open_edit(),
            Action::OpenDue => self.open_due(),
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

    fn find(&self, id: i64) -> Option<&Todo> {
        self.todos.iter().find(|t| t.id == id)
    }

    /// 직계 자식들(표시 순서).
    fn children(&self, id: i64) -> impl Iterator<Item = &Todo> {
        self.todos.iter().filter(move |c| c.parent_id == Some(id))
    }

    /// 자신을 제외한 조상들(가까운 순).
    fn ancestors(&self, id: i64) -> impl Iterator<Item = &Todo> {
        std::iter::successors(self.find(id), |t| {
            t.parent_id.and_then(|pid| self.find(pid))
        })
        .skip(1)
    }

    /// 하위 전체 id(깊이 우선 순).
    fn descendant_ids(&self, id: i64) -> Vec<i64> {
        self.children(id)
            .flat_map(|c| std::iter::once(c.id).chain(self.descendant_ids(c.id)))
            .collect()
    }

    pub(crate) fn children_done(&self, parent_id: i64) -> (usize, usize) {
        self.children(parent_id).fold((0, 0), |(done, total), c| {
            (done + usize::from(c.done), total + 1)
        })
    }

    /// 항목의 깊이(최상위 = 0).
    pub(crate) fn depth_of(&self, id: i64) -> usize {
        self.ancestors(id).count()
    }

    /// 항목을 뿌리로 한 서브트리의 높이(자식 없음 = 1).
    fn subtree_height(&self, id: i64) -> usize {
        1 + self
            .children(id)
            .map(|c| self.subtree_height(c.id))
            .max()
            .unwrap_or(0)
    }

    /// 현재 상태를 undo 스택에 쌓는다(가장 오래된 것부터 밀어냄).
    fn push_undo(&mut self) -> Result<()> {
        let snap = Snapshot {
            projects: self.store.list_projects()?,
            todos: self.store.list_all()?,
        };
        if self.undo_stack.len() >= UNDO_LIMIT {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(snap);
        Ok(())
    }

    fn undo(&mut self) -> Result<()> {
        let Some(snap) = self.undo_stack.pop_back() else {
            self.status = "되돌릴 작업이 없어요".into();
            return Ok(());
        };
        self.store.replace_all(&snap.projects, &snap.todos)?;
        self.reload()?;
        self.status = format!("되돌림 (남은 되돌리기 {}개)", self.undo_stack.len());
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.projects = self.store.list_projects()?;
        if self.projects.is_empty() {
            self.store.add_project("기본")?;
            self.projects = self.store.list_projects()?;
        }
        if !self.projects.iter().any(|p| p.id == self.project_id) {
            self.project_id = self.projects[0].id;
        }
        let prev = self.selected_id();
        self.todos = self.store.list(self.project_id)?;
        self.data_version = self.store.data_version()?;
        self.rebuild_visible();
        self.select_id_or_keep(prev);
        Ok(())
    }

    fn rebuild_visible(&mut self) {
        // 조상 중 하나라도 접혀 있으면 숨긴다.
        self.visible = self
            .todos
            .iter()
            .enumerate()
            .filter(|(_, t)| !self.ancestors(t.id).any(|a| a.collapsed))
            .map(|(i, _)| i)
            .collect();
    }

    /// id가 아직 보이면 그 항목을, 사라졌으면 이전 커서 자리를(범위에 맞게 잘라) 유지한다.
    fn select_id_or_keep(&mut self, id: Option<i64>) {
        let idx = id
            .and_then(|id| self.visible.iter().position(|&i| self.todos[i].id == id))
            .or_else(|| {
                (!self.visible.is_empty()).then(|| {
                    self.state
                        .selected()
                        .unwrap_or(0)
                        .min(self.visible.len() - 1)
                })
            });
        self.state.select(idx);
    }

    fn selected(&self) -> Option<&Todo> {
        let v = self.state.selected()?;
        let &i = self.visible.get(v)?;
        self.todos.get(i)
    }

    fn selected_id(&self) -> Option<i64> {
        self.selected().map(|t| t.id)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let len = self.visible.len() as isize;
        let cur = self.state.selected().unwrap_or(0) as isize;
        self.state
            .select(Some((cur + delta).rem_euclid(len) as usize));
    }

    /// 같은 부모·같은 완료 상태의 형제 안에서 순서를 옮긴다. 끝에서는 반대편으로 감긴다.
    fn move_selected(&mut self, delta: isize) -> Result<()> {
        let Some(cur) = self.selected() else {
            return Ok(());
        };
        let (id, parent_id, done) = (cur.id, cur.parent_id, cur.done);

        let group: Vec<i64> = self
            .todos
            .iter()
            .filter(|t| t.parent_id == parent_id && t.done == done)
            .map(|t| t.id)
            .collect();
        let Some(order) = rotate(&group, id, delta) else {
            return Ok(());
        };

        self.push_undo()?;
        self.store.set_positions(&order)?;
        self.reload()?;
        self.status = "순서 이동됨".into();
        Ok(())
    }

    fn toggle_done(&mut self) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let (id, done) = (t.id, t.done);
        let new = !done;

        // 체크/해제는 하위 전체에 전파되고, 해제는 완료된 조상들을 다시 연다.
        let reopened = (!new).then(|| self.ancestors(id).map(|a| (a.id, false)));
        let updates: Vec<(i64, bool)> = std::iter::once((id, new))
            .chain(self.descendant_ids(id).into_iter().map(|d| (d, new)))
            .chain(reopened.into_iter().flatten())
            .collect();

        self.push_undo()?;
        self.store.set_done_many(&updates)?;
        self.reload()?;
        Ok(())
    }

    fn set_collapse(&mut self, collapsed: bool) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let id = t.id;
        if self.children(id).next().is_none() || t.collapsed == collapsed {
            return Ok(());
        }
        // 접기/펼치기는 보기 상태라 undo 대상에서 뺀다.
        self.store.set_collapsed(id, collapsed)?;
        self.reload()
    }

    fn indent_selected(&mut self) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let (id, done, parent_id) = (t.id, t.done, t.parent_id);

        let siblings: Vec<i64> = self
            .todos
            .iter()
            .filter(|x| x.parent_id == parent_id)
            .map(|x| x.id)
            .collect();
        let idx = siblings.iter().position(|&x| x == id).unwrap();
        if idx == 0 {
            self.status = "위에 넣을 형제 항목이 없어요".into();
            return Ok(());
        }
        // 넣은 뒤 가장 깊은 노드가 depth(0-based) MAX_DEPTH-1을 넘으면 안 된다.
        if self.depth_of(id) + self.subtree_height(id) > MAX_DEPTH - 1 {
            self.status = format!("{MAX_DEPTH}단계까지만 넣을 수 있어요");
            return Ok(());
        }
        let new_parent = siblings[idx - 1];

        self.push_undo()?;
        self.store.indent(id, new_parent, !done)?;
        self.reload()?;
        self.select_id_or_keep(Some(id));
        self.status = "하위로 넣음".into();
        Ok(())
    }

    fn outdent_selected(&mut self) -> Result<()> {
        let Some(t) = self.selected() else {
            return Ok(());
        };
        let Some(pid) = t.parent_id else {
            self.status = "이미 최상위 항목이에요".into();
            return Ok(());
        };
        let id = t.id;
        let grandparent = self.find(pid).and_then(|p| p.parent_id);

        // 한 단계 위 형제들 사이, 기존 부모 바로 다음 자리에 끼워 넣는다.
        let mut order: Vec<i64> = self
            .todos
            .iter()
            .filter(|x| x.parent_id == grandparent)
            .map(|x| x.id)
            .collect();
        let at = order.iter().position(|&x| x == pid).unwrap();
        order.insert(at + 1, id);

        self.push_undo()?;
        self.store.outdent(id, grandparent, &order)?;
        self.reload()?;
        self.select_id_or_keep(Some(id));
        self.status = "한 단계 위로 뺌".into();
        Ok(())
    }

    fn delete_selected(&mut self) -> Result<()> {
        if let Some(id) = self.selected_id() {
            self.push_undo()?;
            self.store.delete(id)?;
            self.reload()?;
            self.status = "삭제됨 (u 되돌리기)".into();
        }
        Ok(())
    }

    /// 선택 항목과 하위 목표들을 마크다운 체크리스트로 시스템 클립보드에 복사한다.
    fn yank_selected(&mut self) {
        let Some(id) = self.selected_id() else {
            self.status = "복사할 항목이 없어요".into();
            return;
        };
        let text = self.yank_text(id);
        let count = 1 + self.descendant_ids(id).len();
        self.status = match clipboard::copy(&text) {
            Ok(()) => format!("복사됨 ({count}줄)"),
            Err(msg) => msg,
        };
    }

    /// 항목과 하위 목표들을 마크다운 체크리스트 여러 줄로 만든다.
    fn yank_text(&self, id: i64) -> String {
        let base = self.depth_of(id);
        std::iter::once(id)
            .chain(self.descendant_ids(id))
            .filter_map(|i| self.find(i))
            .map(|t| self.yank_line(t, base))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 클립보드용 한 줄: `- [ ] 내용 (마감: YYYY-MM-DD)`, 깊이만큼 들여쓴다.
    /// 내용에 줄바꿈이 들어 있어도 체크리스트가 깨지지 않게 공백으로 눕힌다.
    fn yank_line(&self, todo: &Todo, base_depth: usize) -> String {
        let indent = "  ".repeat(self.depth_of(todo.id).saturating_sub(base_depth));
        let check = if todo.done { "x" } else { " " };
        let text = todo.text.replace(['\n', '\r'], " ");
        let due = todo
            .due_string()
            .map(|d| format!(" (마감: {d})"))
            .unwrap_or_default();
        format!("{indent}- [{check}] {text}{due}")
    }

    fn commit_insert(&mut self) -> Result<()> {
        let text = self.input.value().trim().to_string();
        if !text.is_empty() {
            self.push_undo()?;
            let id = self.store.add(&text, None, None, self.project_id)?;
            self.reload()?;
            self.select_id_or_keep(Some(id));
            self.status = "추가됨".into();
        }
        self.input.reset();
        Ok(())
    }

    fn project_index(&self) -> usize {
        self.projects
            .iter()
            .position(|p| p.id == self.project_id)
            .unwrap_or(0)
    }

    /// 현재 탭에서 delta만큼 떨어진 프로젝트(순환). 탭이 하나뿐이면 None.
    fn neighbor_project(&self, delta: isize) -> Option<&Project> {
        let len = self.projects.len();
        (len >= 2)
            .then(|| (self.project_index() as isize + delta).rem_euclid(len as isize) as usize)
            .and_then(|idx| self.projects.get(idx))
    }

    fn select_project(&mut self, idx: usize) -> Result<()> {
        let Some(p) = self.projects.get(idx) else {
            self.status = format!("{}번 프로젝트가 없어요", idx + 1);
            return Ok(());
        };
        self.project_id = p.id;
        let name = p.name.clone();
        self.state.select(None);
        self.reload()?;
        self.status = format!("프로젝트: {name}");
        Ok(())
    }

    /// 현재 탭 자체의 순서를 옮긴다. 끝에서는 반대편으로 감긴다.
    fn move_project(&mut self, delta: isize) -> Result<()> {
        let ids: Vec<i64> = self.projects.iter().map(|p| p.id).collect();
        let Some(order) = rotate(&ids, self.project_id, delta) else {
            self.status = "프로젝트가 하나뿐이에요".into();
            return Ok(());
        };

        self.push_undo()?;
        self.store.set_project_positions(&order)?;
        self.reload()?;
        self.status = "탭 순서 이동됨".into();
        Ok(())
    }

    /// 선택한 항목(하위 포함)을 옆 프로젝트의 최상위로 보낸다.
    fn move_to_project(&mut self, delta: isize) -> Result<()> {
        let Some(target) = self.neighbor_project(delta).map(|p| (p.id, p.name.clone())) else {
            self.status = "보낼 다른 프로젝트가 없어요".into();
            return Ok(());
        };
        let Some(id) = self.selected_id() else {
            return Ok(());
        };
        let subtree: Vec<i64> = std::iter::once(id).chain(self.descendant_ids(id)).collect();

        self.push_undo()?;
        self.store.move_to_project(id, &subtree, target.0)?;
        self.reload()?;
        self.status = format!("'{}' 프로젝트로 보냄 (u 되돌리기)", target.1);
        Ok(())
    }

    fn delete_project(&mut self) -> Result<()> {
        if self.projects.len() <= 1 {
            self.status = "마지막 프로젝트는 삭제할 수 없어요".into();
            return Ok(());
        }
        let name = self.projects[self.project_index()].name.clone();
        self.push_undo()?;
        self.store.delete_project(self.project_id)?;
        self.reload()?;
        self.status = format!("프로젝트 '{name}' 삭제됨 (u 되돌리기)");
        Ok(())
    }

    fn open_popup(&mut self, kind: PopupKind, input: String) {
        self.status = "Enter 저장  Esc 취소".into();
        self.popup = Some(Popup {
            kind,
            input: Input::new(input),
        });
    }

    fn open_edit(&mut self) {
        if let Some(t) = self.selected() {
            self.open_popup(PopupKind::Edit { id: t.id }, t.text.clone());
        }
    }

    fn open_due(&mut self) {
        if let Some(t) = self.selected() {
            let input = t.due_string().unwrap_or_default();
            self.open_popup(PopupKind::Due { id: t.id }, input);
        }
    }

    fn open_subtask(&mut self) {
        let Some(t) = self.selected() else {
            return;
        };
        // 최대 깊이 미만이면 선택 항목 밑으로, 이미 최심이면 형제로 추가한다.
        let parent_id = if self.depth_of(t.id) < MAX_DEPTH - 1 {
            t.id
        } else {
            t.parent_id.unwrap_or(t.id)
        };
        self.open_popup(PopupKind::Subtask { parent_id }, String::new());
    }

    fn open_new_project(&mut self) {
        if self.projects.len() >= PROJECT_LIMIT {
            self.status = format!("프로젝트는 최대 {PROJECT_LIMIT}개까지예요");
            return;
        }
        self.open_popup(PopupKind::NewProject, String::new());
    }

    fn open_rename_project(&mut self) {
        let name = self.projects[self.project_index()].name.clone();
        self.open_popup(
            PopupKind::RenameProject {
                id: self.project_id,
            },
            name,
        );
    }

    fn popup_cancel(&mut self) {
        self.status = "취소됨".into();
        self.popup = None;
    }

    fn popup_commit(&mut self) -> Result<()> {
        let Some(popup) = self.popup.take() else {
            return Ok(());
        };
        let committed = match popup.kind {
            PopupKind::Edit { id } => self.commit_edit(id, popup.input.value()),
            PopupKind::Due { id } => self.commit_due(id, popup.input.value()),
            PopupKind::Subtask { parent_id } => self.commit_subtask(parent_id, popup.input.value()),
            PopupKind::NewProject => self.commit_new_project(popup.input.value()),
            PopupKind::RenameProject { id } => self.commit_rename_project(id, popup.input.value()),
        };
        match committed {
            Ok(()) => Ok(()),
            // 검증 실패는 상태 표시줄에 알리고 팝업을 유지한다.
            Err(Error::Invalid(msg)) => {
                self.status = msg;
                self.popup = Some(popup);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn commit_subtask(&mut self, parent_id: i64, text: &str) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("내용을 입력하세요".into()));
        }
        self.push_undo()?;
        let id = self.store.add_subtask(text, parent_id)?;
        self.reload()?;
        self.select_id_or_keep(Some(id));
        self.status = "하위 목표 추가됨".into();
        Ok(())
    }

    fn commit_edit(&mut self, id: i64, text: &str) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return Err(Error::Invalid("내용을 입력하세요".into()));
        }
        let due = self.find(id).and_then(|t| t.due_at);
        self.push_undo()?;
        self.store.update(id, text, due)?;
        self.reload()?;
        self.status = "수정됨".into();
        Ok(())
    }

    fn commit_due(&mut self, id: i64, input: &str) -> Result<()> {
        let value = parse_due(input)?;
        self.push_undo()?;
        self.store.set_due(id, value)?;
        self.reload()?;
        self.status = if value.is_some() {
            "마감 설정됨"
        } else {
            "마감 해제됨"
        }
        .to_string();
        Ok(())
    }

    fn commit_new_project(&mut self, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("이름을 입력하세요".into()));
        }
        if self.projects.len() >= PROJECT_LIMIT {
            return Err(Error::Invalid(format!(
                "프로젝트는 최대 {PROJECT_LIMIT}개까지예요"
            )));
        }
        self.push_undo()?;
        let id = self.store.add_project(name)?;
        self.project_id = id;
        self.state.select(None);
        self.reload()?;
        self.status = format!("프로젝트 '{name}' 추가됨");
        Ok(())
    }

    fn commit_rename_project(&mut self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("이름을 입력하세요".into()));
        }
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
        let store = Store::open(std::path::PathBuf::from(":memory:")).unwrap();
        let pid = store.list_projects().unwrap()[0].id;
        for i in 0..n {
            store.add(&format!("todo {i}"), None, None, pid).unwrap();
        }
        App::new(store).unwrap()
    }

    fn app_with_subtasks() -> App {
        let mut app = app_with_todos(2);
        let p0 = app.todos[0].id;
        let pid = app.project_id;
        app.store.add("child A", None, Some(p0), pid).unwrap();
        app.store.add("child B", None, Some(p0), pid).unwrap();
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
        let todos = app.store.list(app.project_id).unwrap();
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
        let todos = app.store.list(app.project_id).unwrap();
        assert_eq!(todos[1].text, "수정됨");
    }

    #[test]
    fn move_selected_reorders_and_keeps_selection() {
        let mut app = app_with_todos(3);
        app.state.select(Some(0));
        app.move_selected(1).unwrap();
        assert_eq!(texts(&app), ["todo 1", "todo 0", "todo 2"]);
        assert_eq!(app.selected().unwrap().text, "todo 0");
        assert_eq!(app.state.selected(), Some(1));
    }

    #[test]
    fn move_selected_wraps_at_edges() {
        let mut app = app_with_todos(3);
        app.state.select(Some(0));
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
        app.state.select(Some(0));
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
        app.state.select(Some(2));
        app.toggle_done().unwrap();
        assert_eq!(app.selected().unwrap().text, "todo 2");

        // 완료 항목이 하나뿐이라 이동할 곳이 없다(미완료 경계를 넘지 않음).
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
        // 끝에서 한 번 더 내리면 형제 안에서 맨 앞으로 감긴다.
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
        let pid = app.project_id;
        let mid = app.store.add("mid", None, Some(top), pid).unwrap();
        let leaf = app.store.add("leaf", None, Some(mid), pid).unwrap();
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
        app.set_collapse(true).unwrap();
        assert_eq!(app.visible.len(), 2);
        app.set_collapse(false).unwrap();
        assert_eq!(app.visible.len(), 4);
    }

    #[test]
    fn collapsed_ancestor_hides_grandchildren() {
        let mut app = app_with_todos(1);
        let top = app.todos[0].id;
        let pid = app.project_id;
        let mid = app.store.add("mid", None, Some(top), pid).unwrap();
        app.store.add("leaf", None, Some(mid), pid).unwrap();
        app.reload().unwrap();
        assert_eq!(app.visible.len(), 3);

        app.select_id_or_keep(Some(top));
        app.set_collapse(true).unwrap();
        // mid와 leaf 모두 숨는다(leaf의 직계 부모는 안 접혔어도 조상이 접힘).
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

        // depth 2 항목의 형제를 만들어 한 번 더 넣으면 depth 3이 되므로 거부된다.
        let a = app.todos.iter().find(|t| t.text == "child A").unwrap().id;
        let pid = app.project_id;
        let c = app.store.add("child C", None, Some(a), pid).unwrap();
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
        // 3단 트리 전체를 다른 항목 밑에 넣으면 4단이 되므로 거부된다.
        let mut app = app_with_todos(2);
        let pid = app.project_id;
        let t1 = app.todos[1].id;
        let mid = app.store.add("mid", None, Some(t1), pid).unwrap();
        app.store.add("leaf", None, Some(mid), pid).unwrap();
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
        let pid = app.project_id;
        let mid = app.store.add("mid", None, Some(top), pid).unwrap();
        let leaf = app.store.add("leaf", None, Some(mid), pid).unwrap();
        app.reload().unwrap();

        app.select_id_or_keep(Some(leaf));
        app.outdent_selected().unwrap();
        // 최상위가 아니라 한 단계 위(top의 자식)로 올라온다.
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
        // 완료 항목은 형제 아래로 가라앉으므로 순서도 화면과 같다.
        app.store.set_done_many(&[(child_a, true)]).unwrap();
        app.store
            .update(parent, "todo 0", parse_due("2026-07-01").unwrap())
            .unwrap();
        app.reload().unwrap();

        assert_eq!(
            app.yank_text(parent),
            "- [ ] todo 0 (마감: 2026-07-01)\n  - [ ] child B\n  - [x] child A"
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
        app.store.update(id, "첫 줄\n둘째 줄", None).unwrap();
        app.reload().unwrap();
        assert_eq!(app.yank_text(id), "- [ ] 첫 줄 둘째 줄");
    }

    #[test]
    fn delete_keeps_cursor_position() {
        let mut app = app_with_todos(4);
        app.state.select(Some(2));
        app.delete_selected().unwrap();
        // 커서가 맨 위로 튀지 않고 같은 자리(다음 항목)에 남는다.
        assert_eq!(app.state.selected(), Some(2));
        assert_eq!(app.selected().unwrap().text, "todo 3");

        // 마지막 항목을 지우면 범위에 맞게 한 칸 위로 온다.
        app.delete_selected().unwrap();
        assert_eq!(app.state.selected(), Some(1));
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
        // 5개까지만 되돌아가므로 처음 2개는 남는다.
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
        let p1 = app.project_id;
        app.commit_new_project("업무").unwrap();
        let p2 = app.project_id;
        assert_ne!(p1, p2);
        assert!(app.todos.is_empty());

        app.input = Input::new("회사 일".to_string());
        app.commit_insert().unwrap();
        assert_eq!(texts(&app), ["회사 일"]);

        app.select_project(0).unwrap();
        assert_eq!(app.project_id, p1);
        assert_eq!(texts(&app), ["todo 0"]);

        app.select_project(1).unwrap();
        assert_eq!(app.project_id, p2);

        app.delete_project().unwrap();
        assert_eq!(app.project_id, p1);
        assert_eq!(app.projects.len(), 1);

        // 마지막 프로젝트는 지울 수 없다.
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
        app.select_project(0).unwrap(); // 원래 프로젝트로 복귀
        let p0 = app.todos[0].id;

        app.select_id_or_keep(Some(p0));
        app.move_to_project(1).unwrap();
        assert_eq!(texts(&app), ["todo 1"]);

        app.select_project(1).unwrap();
        assert_eq!(texts(&app), ["todo 0", "child A", "child B"]);

        // undo로 원상 복구: 업무 프로젝트는 비고, 원래 프로젝트에 전부 돌아온다.
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
        assert_eq!(app.project_index(), 2); // 현재 탭 = 사이드

        app.move_project(-1).unwrap();
        assert_eq!(names(&app), ["기본", "사이드", "업무"]);
        assert_eq!(app.project_index(), 1);

        // 맨 앞에서 한 번 더 올리면 맨 뒤로 감긴다.
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
}
