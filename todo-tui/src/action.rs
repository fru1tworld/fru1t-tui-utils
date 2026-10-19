use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_input::InputRequest;
use tui_kit::keys::{is_shift_enter, text_edit_request};
use tui_kit::mode::Mode;

use crate::app::App;

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Quit,
    HoldTab,
    EnterInsert,
    EnterNormal,
    Input(InputRequest),
    CommitInsert,
    Select(isize),
    Reorder(isize),
    Indent,
    Outdent,
    Collapse,
    Expand,
    ToggleDone,
    Delete,
    Undo,
    Yank,
    SelectProject(usize),
    MoveProject(isize),
    MoveToProject(isize),
    OpenEdit,
    OpenSubtask,
    OpenNewProject,
    OpenRenameProject,
    DeleteProject,
    PopupInput(InputRequest),
    PopupCommit,
    PopupCancel,
}

pub(crate) enum Flow {
    Continue,
    Quit,
}

pub(crate) fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    use Action::*;
    use KeyCode::{Char, Down, Enter, Esc, Left, Right, Up};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    if let Some(popup) = &app.popup {
        return match key.code {
            Esc => Some(PopupCancel),
            Enter if is_shift_enter(key) && popup.kind.is_multiline() => {
                Some(PopupInput(InputRequest::InsertChar('\n')))
            }
            Enter => Some(PopupCommit),
            _ => text_edit_request(key).map(PopupInput),
        };
    }

    if key.code == KeyCode::Tab {
        return Some(HoldTab);
    }
    if app.tab_held {
        match key.code {
            Left => return Some(MoveToProject(-1)),
            Right => return Some(MoveToProject(1)),
            _ => {}
        }
    }

    let arrows_move_cursor = app.mode == Mode::Insert && !app.input.value().is_empty();

    let nav = match key.code {
        Left if ctrl && shift => Some(MoveToProject(-1)),
        Right if ctrl && shift => Some(MoveToProject(1)),
        Up if shift => Some(Reorder(-1)),
        Down if shift => Some(Reorder(1)),
        Left if shift && !arrows_move_cursor => Some(Indent),
        Right if shift && !arrows_move_cursor => Some(Outdent),
        Up => Some(Select(-1)),
        Down => Some(Select(1)),
        Left if !arrows_move_cursor => Some(Collapse),
        Right if !arrows_move_cursor => Some(Expand),
        _ => None,
    };
    if nav.is_some() {
        return nav;
    }

    match app.mode {
        Mode::Insert => match key.code {
            Esc => Some(EnterNormal),
            Enter => Some(CommitInsert),
            _ => text_edit_request(key).map(Input),
        },
        Mode::Normal => Some(match key.code {
            Char('q') => Quit,
            Char('i' | 'a') | Esc => EnterInsert,
            Char('e') => OpenEdit,
            Char('s') => OpenSubtask,
            Char('d') => Delete,
            Char('u') => Undo,
            Char('y') => Yank,
            Char('n') => OpenNewProject,
            Char('r') => OpenRenameProject,
            Char('x') => DeleteProject,
            Char('<' | ',') => MoveToProject(-1),
            Char('>' | '.') => MoveToProject(1),
            Char('{' | '[') => MoveProject(-1),
            Char('}' | ']') => MoveProject(1),
            Char(c @ '1'..='5') => SelectProject(c as usize - '1' as usize),
            Char(' ') => ToggleDone,
            Char('j') => Select(1),
            Char('k') => Select(-1),
            Char('l') => Expand,
            Char('h') => Collapse,
            _ => return None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use tui_input::Input;

    use super::*;
    use crate::db::Store;
    use crate::dialog::{Popup, PopupKind};
    use crate::domain::ProjectId;

    fn app_with_popup(kind: PopupKind) -> App {
        let mut app = App::new(Store::open(std::path::Path::new(":memory:")).unwrap()).unwrap();
        app.popup = Some(Popup {
            kind,
            input: Input::default(),
        });
        app
    }

    #[test]
    fn shift_enter_adds_a_newline_only_in_multiline_popups() {
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);

        let app = app_with_popup(PopupKind::NewProject);
        assert!(matches!(
            map_key(&app, shift_enter),
            Some(Action::PopupCommit)
        ));

        let app = app_with_popup(PopupKind::RenameProject { id: ProjectId(1) });
        assert!(matches!(
            map_key(&app, shift_enter),
            Some(Action::PopupCommit)
        ));

        let app = app_with_popup(PopupKind::AddSubtask {
            parent_id: crate::domain::TodoId(1),
        });
        assert!(matches!(
            map_key(&app, shift_enter),
            Some(Action::PopupInput(InputRequest::InsertChar('\n')))
        ));
    }
}
