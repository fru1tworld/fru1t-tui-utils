use ratatui::{style::Stylize, text::Span};

/// Vim-style editing mode shared by the note-taking apps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Insert,
    Normal,
}

impl Mode {
    pub fn badge(self) -> Span<'static> {
        match self {
            Self::Insert => " INSERT ".black().on_green().bold(),
            Self::Normal => " NORMAL ".black().on_blue().bold(),
        }
    }
}
