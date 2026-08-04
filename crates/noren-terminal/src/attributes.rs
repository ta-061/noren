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
/// background according to the field in which the value is used. [`Ansi`]
/// selects one of the 16 ANSI palette colors; [`Indexed`] selects an entry of
/// the xterm 256-color palette (the 16 ANSI colors, a 6×6×6 color cube, and a
/// 24-step grayscale ramp); [`Rgb`] selects a direct 24-bit color. All three
/// extended forms are modelled but left to the renderer to resolve against a
/// palette or theme — the terminal state only records the selection.
///
/// [`Ansi`]: Self::Ansi
/// [`Indexed`]: Self::Indexed
/// [`Rgb`]: Self::Rgb
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Color {
    #[default]
    Default,
    Ansi(AnsiColor),
    /// An xterm 256-color palette index (`0..=255`).
    Indexed(u8),
    /// A direct 24-bit color (red, green, blue), each channel `0..=255`.
    Rgb(u8, u8, u8),
}

impl Color {
    /// Select an ANSI palette color.
    #[must_use]
    pub const fn ansi(color: AnsiColor) -> Self {
        Self::Ansi(color)
    }

    /// Select an xterm 256-color palette index.
    #[must_use]
    pub const fn indexed(index: u8) -> Self {
        Self::Indexed(index)
    }

    /// Select a direct 24-bit color.
    #[must_use]
    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::Rgb(red, green, blue)
    }

    /// Whether this selection uses the renderer's contextual default color.
    #[must_use]
    pub const fn is_default(self) -> bool {
        matches!(self, Self::Default)
    }

    /// The selected ANSI color, or `None` for any other selection.
    #[must_use]
    pub const fn ansi_color(self) -> Option<AnsiColor> {
        match self {
            Self::Ansi(color) => Some(color),
            _ => None,
        }
    }

    /// The selected 256-color palette index, or `None` for other selections.
    #[must_use]
    pub const fn indexed_value(self) -> Option<u8> {
        match self {
            Self::Indexed(index) => Some(index),
            _ => None,
        }
    }

    /// The selected direct color channels, or `None` for other selections.
    #[must_use]
    pub const fn rgb_channels(self) -> Option<(u8, u8, u8)> {
        match self {
            Self::Rgb(red, green, blue) => Some((red, green, blue)),
            _ => None,
        }
    }
}

const BOLD: u8 = 1 << 0;
const UNDERLINE: u8 = 1 << 1;
const REVERSE: u8 = 1 << 2;

/// Renderer-independent visual attributes for a terminal cell.
///
/// The default uses contextual foreground, background, and underline colors
/// with all style flags disabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellAttributes {
    foreground: Color,
    background: Color,
    underline_color: Color,
    flags: u8,
}

impl CellAttributes {
    /// Baseline cell attributes used by [`Default`].
    pub const DEFAULT: Self = Self {
        foreground: Color::Default,
        background: Color::Default,
        underline_color: Color::Default,
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

    /// Underline color selection.
    #[must_use]
    pub const fn underline_color(self) -> Color {
        self.underline_color
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

    /// Return these attributes with a different underline color selection.
    #[must_use]
    pub const fn with_underline_color(mut self, underline_color: Color) -> Self {
        self.underline_color = underline_color;
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
