use ratatui::style::Color;
use two_face::theme::EmbeddedThemeName;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    Dark,
    Light,
}

pub struct Palette {
    pub added: Color,
    pub removed: Color,
    pub added_word: Color,
    pub removed_word: Color,
    pub selection: Color,
    pub brackets: [Color; 6],
    pub syntax: EmbeddedThemeName,
}

const DARK: Palette = Palette {
    added: Color::Rgb(24, 48, 34),
    removed: Color::Rgb(54, 29, 35),
    added_word: Color::Rgb(40, 88, 59),
    removed_word: Color::Rgb(100, 44, 54),
    selection: Color::DarkGray,
    brackets: [
        Color::Rgb(235, 203, 139),
        Color::Rgb(180, 142, 173),
        Color::Rgb(136, 192, 208),
        Color::Rgb(208, 135, 112),
        Color::Rgb(129, 161, 193),
        Color::Rgb(163, 190, 140),
    ],
    syntax: EmbeddedThemeName::Nord,
};

const LIGHT: Palette = Palette {
    added: Color::Rgb(230, 255, 236),
    removed: Color::Rgb(255, 235, 233),
    added_word: Color::Rgb(171, 242, 188),
    removed_word: Color::Rgb(255, 193, 192),
    selection: Color::Rgb(208, 215, 222),
    brackets: [
        Color::Rgb(154, 103, 0),
        Color::Rgb(130, 80, 223),
        Color::Rgb(9, 105, 218),
        Color::Rgb(188, 76, 0),
        Color::Rgb(17, 99, 41),
        Color::Rgb(207, 34, 46),
    ],
    syntax: EmbeddedThemeName::Github,
};

impl Appearance {
    pub const ALL: [Self; 2] = [Self::Dark, Self::Light];

    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.label() == label)
    }

    pub fn palette(self) -> &'static Palette {
        match self {
            Self::Dark => &DARK,
            Self::Light => &LIGHT,
        }
    }
}
