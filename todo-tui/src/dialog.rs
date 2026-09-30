use tui_input::Input;

use crate::db::{ProjectId, TodoId};

#[derive(Clone, Copy)]
pub(crate) enum PopupKind {
    EditTodo { id: TodoId },
    AddSubtask { parent_id: TodoId },
    NewProject,
    RenameProject { id: ProjectId },
}

impl PopupKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::EditTodo { .. } => "내용 편집 (Enter 저장 · Esc 취소)",
            Self::AddSubtask { .. } => "하위 목표 (Enter 저장 · Esc 취소)",
            Self::NewProject => "새 프로젝트 이름 (Enter 생성 · Esc 취소)",
            Self::RenameProject { .. } => "프로젝트 이름 변경 (Enter 저장 · Esc 취소)",
        }
    }
}

pub(crate) struct Popup {
    pub(crate) kind: PopupKind,
    pub(crate) input: Input,
}
