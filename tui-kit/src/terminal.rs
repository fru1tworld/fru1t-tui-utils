use std::io;

use ratatui::{
    DefaultTerminal,
    crossterm::{
        cursor::SetCursorStyle,
        event::{
            KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::supports_keyboard_enhancement,
    },
};

/// Runs `body` in the alternate screen with a bar cursor and, when the terminal
/// supports it, disambiguated escape codes so Shift+Enter and Esc are distinct.
pub fn run<T>(body: impl FnOnce(&mut DefaultTerminal) -> T) -> io::Result<T> {
    let mut terminal = ratatui::try_init()?;
    let _session = Session::start();
    Ok(body(&mut terminal))
}

struct Session {
    keyboard_enhanced: bool,
}

impl Session {
    fn start() -> Self {
        let keyboard_enhanced = supports_keyboard_enhancement().unwrap_or(false);
        if keyboard_enhanced {
            let _ = execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        let _ = execute!(io::stdout(), SetCursorStyle::BlinkingBar);
        Self { keyboard_enhanced }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), SetCursorStyle::DefaultUserShape);
        if self.keyboard_enhanced {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
        ratatui::restore();
    }
}
