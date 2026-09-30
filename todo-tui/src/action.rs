use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_input::InputRequest;

use crate::app::{App, Mode};

#[derive(Clone, Copy)]
pub(crate) enum Action {
    Quit,
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

fn edit_request(key: KeyEvent) -> Option<InputRequest> {
    use InputRequest::*;

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Backspace if ctrl => DeletePrevWord,
        KeyCode::Backspace => DeletePrevChar,
        KeyCode::Delete => DeleteNextChar,
        KeyCode::Left if ctrl => GoToPrevWord,
        KeyCode::Right if ctrl => GoToNextWord,
        KeyCode::Left => GoToPrevChar,
        KeyCode::Right => GoToNextChar,
        KeyCode::Home => GoToStart,
        KeyCode::End => GoToEnd,
        KeyCode::Char('w') if ctrl => DeletePrevWord,
        KeyCode::Char('u') if ctrl => DeleteLine,
        KeyCode::Char('a') if ctrl => GoToStart,
        KeyCode::Char('e') if ctrl => GoToEnd,
        KeyCode::Char('k') if ctrl => DeleteTillEnd,
        KeyCode::Char(c) if !ctrl => InsertChar(c),
        _ => return None,
    })
}

pub(crate) fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    use Action::*;
    use KeyCode::{Char, Down, Enter, Esc, Left, Right, Up};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    if app.popup.is_some() {
        return match key.code {
            Esc => Some(PopupCancel),
            Enter => Some(PopupCommit),
            _ => edit_request(key).map(PopupInput),
        };
    }

    if app.tab_held {
        return match key.code {
            Left => Some(MoveToProject(-1)),
            Right => Some(MoveToProject(1)),
            _ => None,
        };
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
            _ => edit_request(key).map(Input),
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
