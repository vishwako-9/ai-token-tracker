use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub input: Color,
    pub accent: Color,
    pub muted: Color,
    pub hover_bg: Color,
    pub list_selection_bg: Color,
    pub cost: Color,
    pub positive: Color,
    pub negative: Color,
    pub header: Color,
    pub text: Color,
    pub overlay_bg: Color,
}

impl Theme {
    /// Default theme — exact byte-for-byte match of the original const values.
    pub const fn default() -> Self {
        Self {
            selection_bg: Color::Cyan,
            selection_fg: Color::Black,
            input: Color::Cyan,
            accent: Color::Cyan,
            muted: Color::DarkGray,
            hover_bg: Color::DarkGray,
            list_selection_bg: Color::DarkGray,
            cost: Color::Yellow,
            positive: Color::Green,
            negative: Color::Red,
            header: Color::Gray,
            text: Color::White,
            overlay_bg: Color::Black,
        }
    }

    /// Alternate "nord-like" cooler palette for contrast testing.
    pub const fn nord() -> Self {
        Self {
            selection_bg: Color::Rgb(0x88, 0xc0, 0xd0), // nord blue
            selection_fg: Color::Black,
            input: Color::Rgb(0x88, 0xc0, 0xd0),
            accent: Color::Rgb(0x88, 0xc0, 0xd0),
            muted: Color::Rgb(0x4c, 0x56, 0x6a), // nord dark gray
            hover_bg: Color::Rgb(0x4c, 0x56, 0x6a),
            list_selection_bg: Color::Rgb(0x4c, 0x56, 0x6a),
            cost: Color::Rgb(0xeb, 0xcb, 0x8b), // nord yellow
            positive: Color::Rgb(0xa3, 0xbe, 0x8c), // nord green
            negative: Color::Rgb(0xbf, 0x61, 0x6a), // nord red
            header: Color::Rgb(0xd8, 0xde, 0xe9), // nord white
            text: Color::White,
            overlay_bg: Color::Rgb(0x2e, 0x34, 0x40), // nord dark
        }
    }

    /// Alternate "dracula-like" warmer palette for contrast testing.
    pub const fn dracula() -> Self {
        Self {
            selection_bg: Color::Rgb(0xbd, 0x93, 0xf9), // dracula purple
            selection_fg: Color::Black,
            input: Color::Rgb(0xbd, 0x93, 0xf9),
            accent: Color::Rgb(0xbd, 0x93, 0xf9),
            muted: Color::Rgb(0x62, 0x72, 0xa4), // dracula comment
            hover_bg: Color::Rgb(0x62, 0x72, 0xa4),
            list_selection_bg: Color::Rgb(0x62, 0x72, 0xa4),
            cost: Color::Rgb(0xf1, 0xfa, 0x8c), // dracula yellow
            positive: Color::Rgb(0x50, 0xfa, 0x7b), // dracula green
            negative: Color::Rgb(0xff, 0x55, 0x55), // dracula red
            header: Color::Rgb(0xf8, 0xf8, 0xf2), // dracula foreground
            text: Color::White,
            overlay_bg: Color::Rgb(0x28, 0x2a, 0x36), // dracula background
        }
    }

    pub fn by_name(name: &str) -> Option<Self> {
        match name.to_lowercase().as_str() {
            "default" => Some(Self::default()),
            "nord" => Some(Self::nord()),
            "dracula" => Some(Self::dracula()),
            _ => None,
        }
    }

    pub fn available_names() -> &'static [&'static str] {
        &["default", "nord", "dracula"]
    }
}