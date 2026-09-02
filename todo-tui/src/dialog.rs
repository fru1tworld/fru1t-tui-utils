use tui_input::Input;

#[derive(Clone, Copy)]
pub(crate) enum PopupKind {
    Edit { id: i64 },
    Subtask { parent_id: i64 },
    NewProject,
    RenameProject { id: i64 },
}

impl PopupKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Edit { .. } => "내용 편집 (Enter 저장 · Esc 취소)",
            Self::Subtask { .. } => "하위 목표 (Enter 저장 · Esc 취소)",
            Self::NewProject => "새 프로젝트 이름 (Enter 생성 · Esc 취소)",
            Self::RenameProject { .. } => "프로젝트 이름 변경 (Enter 저장 · Esc 취소)",
        }
    }
}

pub(crate) struct Popup {
    pub(crate) kind: PopupKind,
    pub(crate) input: Input,
}
