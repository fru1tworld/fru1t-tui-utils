use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_input::InputRequest;

use crate::app::{App, Mode};

pub(crate) enum Flow {
    Continue,
    Quit,
    Output(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Quit,
    Output,
    EnterInsert,
    EnterNormal,
    Input(InputRequest),
    CommitInsert,
    Select(isize),
    ToggleCollapse,
    BrowseWeek(i64),
    MoveEntryByDays(i64),
    BrowseToday,
    OpenEdit,
    Delete,
    Undo,
    Yank,
    PopupInput(InputRequest),
    PopupCommit,
    PopupCancel,
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
        KeyCode::Char(character) if !ctrl => InsertChar(character),
        _ => return None,
    })
}

pub(crate) fn map_key(app: &App, key: KeyEvent) -> Option<Action> {
    use Action::*;
    use KeyCode::{Char, Down, Enter, Esc, Left, Right, Up};

    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if app.edit_dialog.is_some() {
        return match key.code {
            Esc => Some(PopupCancel),
            Enter if shift => Some(PopupInput(InputRequest::InsertChar('\n'))),
            Enter => Some(PopupCommit),
            _ => edit_request(key).map(PopupInput),
        };
    }

    let editing = app.mode == Mode::Insert && !app.input.value().is_empty();
    match app.mode {
        Mode::Insert => match key.code {
            Esc => Some(EnterNormal),
            Enter if shift => Some(Input(InputRequest::InsertChar('\n'))),
            Enter => Some(CommitInsert),
            Up => Some(Select(-1)),
            Down => Some(Select(1)),
            Left if !editing => Some(BrowseWeek(-1)),
            Right if !editing => Some(BrowseWeek(1)),
            _ => edit_request(key).map(Input),
        },
        Mode::Normal => Some(match key.code {
            Char('q') => Quit,
            Char('o') => Output,
            Char('i' | 'a') | Esc => EnterInsert,
            Char('e') => OpenEdit,
            Char('d') => Delete,
            Char('u') => Undo,
            Char('y') => Yank,
            Char('t') => BrowseToday,
            Enter | Char(' ') => ToggleCollapse,
            Up | Char('k') => Select(-1),
            Down | Char('j') => Select(1),
            Left if shift => MoveEntryByDays(-1),
            Right if shift => MoveEntryByDays(1),
            Left | Char('h') => BrowseWeek(-1),
            Right | Char('l') => BrowseWeek(1),
            _ => return None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::SqliteTilRepository;

    fn app() -> App {
        App::new(SqliteTilRepository::in_memory()).unwrap()
    }

    #[test]
    fn shift_enter_inserts_a_newline_instead_of_committing() {
        let app = app();
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);

        assert_eq!(
            map_key(&app, key),
            Some(Action::Input(InputRequest::InsertChar('\n')))
        );
    }

    #[test]
    fn enter_commits_the_input() {
        let app = app();
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);

        assert_eq!(map_key(&app, key), Some(Action::CommitInsert));
    }

    #[test]
    fn arrows_browse_weeks_when_insert_input_is_empty() {
        let app = app();

        assert_eq!(
            map_key(&app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            Some(Action::BrowseWeek(-1))
        );
        assert_eq!(
            map_key(&app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            Some(Action::BrowseWeek(1))
        );
    }

    #[test]
    fn arrows_browse_weeks_in_normal_mode() {
        let mut app = app();
        app.mode = Mode::Normal;

        assert_eq!(
            map_key(&app, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            Some(Action::BrowseWeek(-1))
        );
        assert_eq!(
            map_key(&app, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            Some(Action::BrowseWeek(1))
        );
    }
}
