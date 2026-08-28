//! Built-in colour themes with verified WCAG contrast.
//!
//! This is the foundation slice of Milestone 6: the renderer's palette moves
//! from a single compiled-in table to a selectable set of built-in themes —
//! `dark` (exactly today's behaviour), `light`, and `high-contrast` — and
//! every theme carries a measured, asserted contrast floor.
//!
//! # What a theme owns
//!
//! A [`Theme`] owns the colours the *application* chooses, not the ones a
//! program names explicitly:
//!
//! - the default foreground (unstyled text, sidebar, status line),
//! - the default background (the render target's clear colour),
//! - the cursor colour (the inverse-video baseline on an unstyled cell),
//! - the sixteen ANSI palette entries (`SGR 30..37`, `90..97`, `40..47`,
//!   `100..107`) that programs select by name.
//!
//! The xterm 256-colour cube and grayscale ramp (`16..=255`) and direct
//! truecolor are program-addressed device colours: they resolve through the
//! same per-theme table with the shared cube appended after the theme's own
//! sixteen ANSI entries, so `SGR 31` and `SGR 38;5;1` can never disagree.
//!
//! # The contrast contract
//!
//! The checked set is every theme-owned foreground — the default foreground
//! plus all sixteen ANSI entries — against the theme's default background:
//! the colours a program draws *normal* text with on the screen it is given.
//! That is the reading path (shell output, prompts, `ls` colours), so the
//! threshold is **WCAG AA for normal text, 4.5:1**, not the 3:1 large-text
//! relaxation: a terminal's glyphs are small (5×7 bitmaps inside 10×20 px
//! cells), never "large text".
//!
//! Two scopes are deliberately **outside** the contract because no palette
//! can honour them, not because they were forgotten:
//!
//! - *program-paired colours* (`SGR 31;41` red on red, or any explicit
//!   foreground/background pair a program names itself): identical colours
//!   are 1.0:1 by definition, and any two palette entries used both ways
//!   cannot simultaneously sit far enough from the background and near
//!   enough to each other's extremes;
//! - *the shared 256-colour cube* (`16..=255`) and *truecolor*: their
//!   extremes (the cube's black corner) fail on every background including
//!   pure white and pure black; a program selecting index 16 has asked for
//!   that device colour.
//!
//! # The dark-palette fix (issue #168)
//!
//! The shipped default (`dark`) used to **fail** the 4.5:1 floor for five of
//! its sixteen ANSI entries on its own background — ANSI black at 1.06:1,
//! blue at 2.10:1, red at 3.38:1, bright blue at 4.16:1, and magenta at
//! 4.21:1 — leaving `\x1b[30m` text effectively invisible on the near-black
//! ground. Issue #168 resolved the product decision PR #167 had deliberately
//! deferred: a terminal's default that hides text is a defect, not a
//! preference, and a preview must not ship one. The five failing entries were
//! each moved the **minimum distance** that clears 4.5:1 (the smallest u8
//! value per entry whose ratio reaches the floor), preserving slot
//! semantics — red stays a pure red ramp value, magenta stays symmetric
//! R=B, bright blue keeps its R=G lavender balance, black stays achromatic,
//! and blue keeps red at zero with blue dominant:
//!
//! | slot | before | ratio | after | ratio |
//! | --- | --- | --- | --- | --- |
//! | black | `[0,0,0]` | 1.06:1 | `[121,121,121]` | 4.53:1 |
//! | red | `[205,0,0]` | 3.38:1 | `[243,0,0]` | 4.52:1 |
//! | blue | `[0,0,238]` | 2.10:1 | `[0,113,255]` | 4.52:1 |
//! | magenta | `[205,0,205]` | 4.21:1 | `[213,0,213]` | 4.50:1 |
//! | bright blue | `[92,92,255]` | 4.16:1 | `[100,100,255]` | 4.52:1 |
//!
//! The theme's measured minimum is now 4.50:1 (magenta, the tightest of the
//! five minimum moves), pinned by test. One visible consequence is
//! documented rather than hidden: ANSI black and bright black now sit close
//! together (`[121,121,121]` vs `[127,127,127]`) because any achromatic
//! entry clearing 4.5:1 on this background must be at least grey 121 —
//! bright black itself was already 127. `light` (5.07:1) and `high-contrast`
//! (7.84:1) are untouched by the fix.

use std::fmt;

/// Names the theme's default slot in [`Theme::min_contrast_on_default_background`].
pub(crate) const DEFAULT_FOREGROUND_SLOT: &str = "default foreground";

/// Slot labels for the sixteen ANSI entries, in palette-index order.
const ANSI_SLOT_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright black",
    "bright red",
    "bright green",
    "bright yellow",
    "bright blue",
    "bright magenta",
    "bright cyan",
    "bright white",
];

/// One of the built-in themes, as named by the `[theme]` configuration.
///
/// [`ThemeName::default`] is `Dark`: with no `[theme]` section the app
/// renders exactly as it did before themes existed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeName {
    /// The default palette: xterm ANSI values on the near-black background,
    /// with the five AA-failing entries minimally brightened (issue #168).
    #[default]
    Dark,
    /// A light background with darkened ANSI entries.
    Light,
    /// Pure white on pure black with pastel ANSI entries; every slot meets
    /// WCAG AAA (7:1) for normal text.
    HighContrast,
}

impl ThemeName {
    /// Every accepted `[theme]` name, in documentation order.
    pub const NAMES: [&'static str; 3] = ["dark", "light", "high-contrast"];

    /// Resolve one configuration value to a theme name.
    ///
    /// Matching is exact (case-sensitive): a theme name is a closed
    /// vocabulary, and accepting `Dark` or `DARK` would be a second spelling
    /// of the same setting rather than a default. Unmatched values are the
    /// caller's typed error, never a fallback.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "high-contrast" => Some(Self::HighContrast),
            _ => None,
        }
    }

    /// The canonical configuration name of this theme.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::HighContrast => "high-contrast",
        }
    }

    /// The concrete palette this name selects.
    #[must_use]
    pub const fn palette(self) -> Theme {
        match self {
            Self::Dark => DARK,
            Self::Light => LIGHT,
            Self::HighContrast => HIGH_CONTRAST,
        }
    }
}

impl fmt::Display for ThemeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The sixteen ANSI colours of the dark theme: the xterm defaults the app
/// shipped since issue #107, with the five entries that failed WCAG AA on
/// the dark background brightened the minimum distance to clear 4.5:1
/// (issue #168; see the module docs for the before/after measurements).
///
/// The foreground/background floats and the shared cube tail below are still
/// byte-identical to the pre-theme renderer; only these five entries moved,
/// deliberately. The values are pinned by test.
const DARK_ANSI: [[u8; 3]; 16] = [
    [121, 121, 121], // 0 black (was [0,0,0], 1.06:1 — issue #168)
    [243, 0, 0],     // 1 red (was [205,0,0], 3.38:1 — issue #168)
    [0, 205, 0],     // 2 green
    [205, 205, 0],   // 3 yellow
    [0, 113, 255],   // 4 blue (was [0,0,238], 2.10:1 — issue #168)
    [213, 0, 213],   // 5 magenta (was [205,0,205], 4.21:1 — issue #168)
    [0, 205, 205],   // 6 cyan
    [229, 229, 229], // 7 white
    [127, 127, 127], // 8 bright black
    [255, 0, 0],     // 9 bright red
    [0, 255, 0],     // 10 bright green
    [255, 255, 0],   // 11 bright yellow
    [100, 100, 255], // 12 bright blue (was [92,92,255], 4.16:1 — #168)
    [255, 0, 255],   // 13 bright magenta
    [0, 255, 255],   // 14 bright cyan
    [255, 255, 255], // 15 bright white
];

/// The dark theme's default foreground, as the exact floats the fragment
/// shader historically returned as a constant.
///
/// Keeping the raw floats (rather than re-deriving from the 8-bit triple
/// 204/235/209) preserves the shipped shade bit-for-bit; see the twin
/// constant in the renderer for the original rationale.
const DARK_FOREGROUND: [f32; 3] = [0.80, 0.92, 0.82];

/// The dark theme's default background, as the exact clear-colour floats.
const DARK_BACKGROUND: [f32; 3] = [0.035, 0.045, 0.04];

/// The light theme's sixteen ANSI entries: darkened, screen-oriented values
/// chosen so every entry keeps at least 5:1 against the light background
/// (AA 4.5:1 with margin). Slot semantics are preserved, not inverted
/// wholesale: the "white" slots (7, 15) carry dark greys because a light
/// theme's white-on-white would be the unreadable extreme the contract
/// exists to prevent.
const LIGHT_ANSI: [[u8; 3]; 16] = [
    [59, 59, 59],
    [176, 48, 48],
    [0, 118, 40],
    [124, 100, 0],
    [0, 66, 148],
    [148, 32, 148],
    [0, 110, 132],
    [95, 102, 104],
    [102, 102, 102],
    [190, 54, 54],
    [0, 122, 44],
    [126, 95, 0],
    [64, 86, 190],
    [164, 44, 164],
    [0, 112, 130],
    [72, 79, 84],
];

/// The light theme's background as shader floats (u8 channels over 255).
const LIGHT_BACKGROUND: [f32; 3] = [246.0 / 255.0, 246.0 / 255.0, 240.0 / 255.0];

/// The light theme's foreground as shader floats.
const LIGHT_FOREGROUND: [f32; 3] = [31.0 / 255.0, 35.0 / 255.0, 40.0 / 255.0];

/// The high-contrast theme's sixteen ANSI entries: pastel values on pure
/// black. Every slot — including the grey slots that drive this theme's
/// minimum — keeps at least 7.5:1, clearing WCAG AAA (7:1) for normal text
/// with margin.
const HIGH_CONTRAST_ANSI: [[u8; 3]; 16] = [
    [160, 160, 160],
    [255, 176, 176],
    [176, 255, 176],
    [255, 255, 176],
    [176, 176, 255],
    [255, 176, 255],
    [176, 255, 255],
    [224, 224, 224],
    [158, 158, 158],
    [255, 192, 192],
    [192, 255, 192],
    [255, 255, 192],
    [192, 192, 255],
    [255, 192, 255],
    [192, 255, 255],
    [255, 255, 255],
];

/// The high-contrast theme's background: pure black.
const HIGH_CONTRAST_BACKGROUND: [f32; 3] = [0.0, 0.0, 0.0];

/// The high-contrast theme's foreground: pure white (21:1 on its ground).
const HIGH_CONTRAST_FOREGROUND: [f32; 3] = [1.0, 1.0, 1.0];

/// One channel of the xterm 6×6×6 colour cube: level zero is zero, and the
/// remaining five levels are `55 + 40 * level`.
const fn cube_channel(level: u8) -> u8 {
    if level == 0 { 0 } else { level * 40 + 55 }
}

/// Derive a full 256-colour table once, at compile time: the given sixteen
/// ANSI entries followed by the shared xterm cube and grayscale ramp.
const fn build_palette(ansi: [[u8; 3]; 16]) -> [[u8; 3]; 256] {
    let mut palette = [[0_u8; 3]; 256];
    let mut index = 0_usize;
    while index < 256 {
        palette[index] = if index < 16 {
            ansi[index]
        } else if index < 232 {
            let cube = (index - 16) as u32;
            [
                cube_channel((cube / 36) as u8),
                cube_channel(((cube / 6) % 6) as u8),
                cube_channel((cube % 6) as u8),
            ]
        } else {
            let gray = (8 + (index - 232) * 10) as u8;
            [gray, gray, gray]
        };
        index += 1;
    }
    palette
}

/// The per-theme 256-colour tables, ANSI head plus the shared xterm tail.
const DARK_PALETTE: [[u8; 3]; 256] = build_palette(DARK_ANSI);
const LIGHT_PALETTE: [[u8; 3]; 256] = build_palette(LIGHT_ANSI);
const HIGH_CONTRAST_PALETTE: [[u8; 3]; 256] = build_palette(HIGH_CONTRAST_ANSI);

/// A concrete, selected palette: the colours drawing resolves through.
///
/// `Copy` and const-constructible so the renderer holds it by value and the
/// default (`dark`) is a compile-time constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    ansi: [[u8; 3]; 16],
    palette256: &'static [[u8; 3]; 256],
    foreground: [f32; 3],
    background: [f32; 3],
    cursor: [f32; 3],
}

/// The built-in `dark` theme: the pre-theme renderer's colours, except the
/// five ANSI entries minimally brightened to clear WCAG AA (issue #168).
///
/// The cursor colour is the theme's default foreground: the caret is drawn
/// as the exact inverse of ordinary text (block in the foreground colour,
/// glyph inside it in the background colour), so it inherits the measured
/// default-pair contrast instead of introducing an unchecked colour.
pub const DARK: Theme = Theme {
    ansi: DARK_ANSI,
    palette256: &DARK_PALETTE,
    foreground: DARK_FOREGROUND,
    background: DARK_BACKGROUND,
    cursor: DARK_FOREGROUND,
};

/// The built-in `light` theme.
pub const LIGHT: Theme = Theme {
    ansi: LIGHT_ANSI,
    palette256: &LIGHT_PALETTE,
    foreground: LIGHT_FOREGROUND,
    background: LIGHT_BACKGROUND,
    cursor: LIGHT_FOREGROUND,
};

/// The built-in `high-contrast` theme.
pub const HIGH_CONTRAST: Theme = Theme {
    ansi: HIGH_CONTRAST_ANSI,
    palette256: &HIGH_CONTRAST_PALETTE,
    foreground: HIGH_CONTRAST_FOREGROUND,
    background: HIGH_CONTRAST_BACKGROUND,
    cursor: HIGH_CONTRAST_FOREGROUND,
};

impl Default for Theme {
    fn default() -> Self {
        DARK
    }
}

impl Theme {
    /// The theme's default foreground as shader floats: unstyled text, the
    /// sidebar, and the status line draw in this colour.
    #[must_use]
    pub const fn foreground(self) -> [f32; 3] {
        self.foreground
    }

    /// The theme's default background as shader floats: the render target's
    /// clear colour and the ground explicit backgrounds cover.
    #[must_use]
    pub const fn background(self) -> [f32; 3] {
        self.background
    }

    /// The foreground as the 8-bit triple actually stored by the RGBA8
    /// render target — `(channel * 255).round()` — which is the value
    /// contrast must be measured on: measuring the float would hide the
    /// quantization every drawn pixel undergoes.
    #[must_use]
    pub fn foreground_u8(self) -> [u8; 3] {
        quantize(self.foreground)
    }

    /// The background as the 8-bit triple actually stored by the RGBA8
    /// render target; see [`Theme::foreground_u8`].
    #[must_use]
    pub fn background_u8(self) -> [u8; 3] {
        quantize(self.background)
    }

    /// The cursor colour as shader floats: the inverse-video baseline for a
    /// cursor on a cell with the default foreground.
    ///
    /// Every built-in theme sets this to its default foreground, so the
    /// caret inverts the reading pair (cursor block in the foreground colour,
    /// glyph inside it in the background colour) and inherits the measured
    /// default-pair WCAG contrast — the order-independent ratio is identical
    /// in both directions. On an SGR foreground/background pair the renderer
    /// instead starts from that cell's resolved foreground; if either the
    /// inverse foreground or a configured override misses 4.5:1 against the
    /// actual cell background, it falls back to the better of black/white.
    /// The theme value therefore remains the unstyled default without making
    /// a fixed-colour promise on backgrounds the theme did not choose.
    #[must_use]
    pub const fn cursor(self) -> [f32; 3] {
        self.cursor
    }

    /// The unstyled cursor baseline as the 8-bit triple actually stored by
    /// the RGBA8 render target; see [`Theme::foreground_u8`].
    #[must_use]
    pub fn cursor_u8(self) -> [u8; 3] {
        quantize(self.cursor)
    }

    /// The theme's sixteen ANSI palette entries, in palette-index order.
    #[must_use]
    pub const fn ansi(self) -> [[u8; 3]; 16] {
        self.ansi
    }

    /// The full 256-colour table drawing resolves `Ansi` and `Indexed`
    /// selections through: the theme's sixteen entries followed by the
    /// shared xterm cube (`16..=231`) and grayscale ramp (`232..=255`), so
    /// the two spellings of one colour cannot disagree.
    #[must_use]
    pub const fn indexed_palette(self) -> &'static [[u8; 3]; 256] {
        self.palette256
    }

    /// The worst WCAG contrast ratio any theme-owned foreground achieves on
    /// the theme's default background, with the slot that produced it.
    ///
    /// This is the single number the contrast contract is asserted on: the
    /// checked foregrounds are the default foreground and all sixteen ANSI
    /// entries (the module docs state why program-paired colours and the
    /// shared cube are out of scope). Tests pin the per-theme minima, so
    /// degrading any pair below its floor fails.
    #[must_use]
    pub fn min_contrast_on_default_background(self) -> (f64, &'static str) {
        let ground = self.background_u8();
        let mut worst = (
            contrast_ratio(self.foreground_u8(), ground),
            DEFAULT_FOREGROUND_SLOT,
        );
        for (slot, color) in self.ansi().into_iter().enumerate() {
            let ratio = contrast_ratio(color, ground);
            if ratio < worst.0 {
                worst = (ratio, ANSI_SLOT_NAMES[slot]);
            }
        }
        worst
    }
}

/// Quantize shader floats to the 8-bit channels the RGBA8 target stores.
const fn quantize([red, green, blue]: [f32; 3]) -> [u8; 3] {
    // `round` is not const-stable on floats in all toolchains this crate
    // pins; the half-up bias here matches `(x * 255.0).round()` for every
    // channel value these themes define (verified by the theme tests, which
    // recompute the quantization with the runtime `round`).
    [
        (red * 255.0 + 0.5) as u8,
        (green * 255.0 + 0.5) as u8,
        (blue * 255.0 + 0.5) as u8,
    ]
}

/// Linearize one 8-bit sRGB channel per WCAG 2.x.
fn linear_channel(channel: u8) -> f64 {
    let value = f64::from(channel) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.x relative luminance of an 8-bit sRGB triple.
#[must_use]
pub fn relative_luminance([red, green, blue]: [u8; 3]) -> f64 {
    0.2126 * linear_channel(red) + 0.7152 * linear_channel(green) + 0.0722 * linear_channel(blue)
}

/// WCAG 2.x contrast ratio between two colours, in `1.0..=21.0`.
///
/// Order-independent: the lighter colour is always the numerator, exactly as
/// the WCAG definition specifies.
#[must_use]
pub fn contrast_ratio(first: [u8; 3], second: [u8; 3]) -> f64 {
    let (first, second) = (relative_luminance(first), relative_luminance(second));
    let (lighter, darker) = if first >= second {
        (first, second)
    } else {
        (second, first)
    };
    (lighter + 0.05) / (darker + 0.05)
}
