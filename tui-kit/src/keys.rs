use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_input::InputRequest;

/// Maps readline-style editing keys to a text input request.
pub fn text_edit_request(key: KeyEvent) -> Option<InputRequest> {
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

/// Whether `key` is Shift+Enter, which inserts a newline in multiline inputs.
pub fn is_shift_enter(key: KeyEvent) -> bool {
    key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::SHIFT)
}
