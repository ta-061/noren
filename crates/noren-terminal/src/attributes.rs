/// One of the 16 colors in the ANSI terminal palette.
///
/// The discriminants are stable palette indexes: standard colors occupy
/// `0..=7`, and their bright counterparts occupy `8..=15`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AnsiColor {
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    BrightBlack = 8,
    BrightRed = 9,
    BrightGreen = 10,
    BrightYellow = 11,
    BrightBlue = 12,
    BrightMagenta = 13,
    BrightCyan = 14,
    BrightWhite = 15,
}

impl AnsiColor {
    /// All ANSI colors in palette order.
    pub const ALL: [Self; 16] = [
        Self::Black,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Blue,
        Self::Magenta,
        Self::Cyan,
        Self::White,
        Self::BrightBlack,
        Self::BrightRed,
        Self::BrightGreen,
        Self::BrightYellow,
        Self::BrightBlue,
        Self::BrightMagenta,
        Self::BrightCyan,
        Self::BrightWhite,
    ];

    /// Zero-based index into an ANSI 16-color palette.
    #[must_use]
    pub const fn palette_index(self) -> u8 {
        self as u8
    }
}

/// Renderer-independent color selection for a terminal cell.
///
/// [`Default`](Self::Default) means the renderer's default foreground or
/// background according to the field in which the value is used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Ansi(AnsiColor),
}

impl Color {
    /// Select an ANSI palette color.
    #[must_use]
    pub const fn ansi(color: AnsiColor) -> Self {
        Self::Ansi(color)
    }

    /// Whether this selection uses the renderer's contextual default color.
    #[must_use]
    pub const fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }

    /// The selected ANSI color, or `None` for the contextual default.
    #[must_use]
    pub const fn ansi_color(self) -> Option<AnsiColor> {
        match self {
            Self::Default => None,
            Self::Ansi(color) => Some(color),
        }
    }
}

const BOLD: u8 = 1 << 0;
const UNDERLINE: u8 = 1 << 1;
const REVERSE: u8 = 1 << 2;

/// Renderer-independent visual attributes for a terminal cell.
///
/// The default uses contextual foreground and background colors with all
/// style flags disabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellAttributes {
    foreground: Color,
    background: Color,
    flags: u8,
}

impl CellAttributes {
    /// Baseline cell attributes used by [`Default`].
    pub const DEFAULT: Self = Self {
        foreground: Color::Default,
        background: Color::Default,
        flags: 0,
    };

    /// Construct baseline cell attributes in a const context.
    #[must_use]
    pub const fn new() -> Self {
        Self::DEFAULT
    }

    /// Foreground color selection.
    #[must_use]
    pub const fn foreground(self) -> Color {
        self.foreground
    }

    /// Background color selection.
    #[must_use]
    pub const fn background(self) -> Color {
        self.background
    }

    /// Whether bold intensity is enabled.
    #[must_use]
    pub const fn is_bold(self) -> bool {
        self.flags & BOLD != 0
    }

    /// Whether underlining is enabled.
    #[must_use]
    pub const fn is_underlined(self) -> bool {
        self.flags & UNDERLINE != 0
    }

    /// Whether foreground and background should be rendered in reverse.
    #[must_use]
    pub const fn is_reversed(self) -> bool {
        self.flags & REVERSE != 0
    }

    /// Return these attributes with a different foreground selection.
    #[must_use]
    pub const fn with_foreground(mut self, foreground: Color) -> Self {
        self.foreground = foreground;
        self
    }

    /// Return these attributes with a different background selection.
    #[must_use]
    pub const fn with_background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }

    /// Return these attributes with bold intensity enabled or disabled.
    #[must_use]
    pub const fn with_bold(mut self, enabled: bool) -> Self {
        self.flags = update_flag(self.flags, BOLD, enabled);
        self
    }

    /// Return these attributes with underlining enabled or disabled.
    #[must_use]
    pub const fn with_underline(mut self, enabled: bool) -> Self {
        self.flags = update_flag(self.flags, UNDERLINE, enabled);
        self
    }

    /// Return these attributes with reverse rendering enabled or disabled.
    #[must_use]
    pub const fn with_reverse(mut self, enabled: bool) -> Self {
        self.flags = update_flag(self.flags, REVERSE, enabled);
        self
    }
}

const fn update_flag(flags: u8, flag: u8, enabled: bool) -> u8 {
    if enabled { flags | flag } else { flags & !flag }
}
