//! Theme palette and WCAG contrast contract (Milestone 6, foundation slice).
//!
//! These tests are the point of the slice, not decoration: every built-in
//! theme's readability is *measured* — the WCAG 2.x contrast ratio between
//! each theme-owned foreground and the theme's default background — and
//! asserted against a documented floor.
//!
//! # Thresholds and why
//!
//! - **AA, 4.5:1, for the `dark` and `light` themes.** WCAG reserves 3:1
//!   for large text; a terminal never draws large text — the PoC's glyphs
//!   are 5×7 bitmaps in 10×20 px cells — so the normal-text bound is the
//!   honest one.
//! - **AAA, 7:1, for `high-contrast`.** A theme named high-contrast that
//!   merely matched AA would be a third ordinary palette; it must exceed
//!   the others' measured minima, and AAA is the level that says so.
//!
//! # The checked set (and what is deliberately outside it)
//!
//! The checked foregrounds are the theme's default foreground plus its
//! sixteen ANSI entries, each against the theme's default background: the
//! colours programs use for ordinary text on the screen they are given.
//! Program-paired colours (`SGR 31;41`) and the shared 256-colour cube are
//! outside any palette's control — identical colours are 1.0:1 by
//! definition, and the cube's black corner fails on every possible
//! background. See `src/theme.rs` for the full statement.
//!
//! # The dark-palette finding
//!
//! The existing default (`dark`) fails 4.5:1 for five ANSI slots on its own
//! background; its measured minimum, 1.06:1 (ANSI black), is pinned below.
//! The colours are frozen anyway: this slice's contract is that no
//! `[theme]` section renders byte-identically to the pre-theme renderer,
//! and the frame oracle proves that at the pixel level. Changing the dark
//! values to pass AA is a separate, deliberate decision for a follow-up.

use noren_app::theme::{Theme, ThemeName, contrast_ratio, relative_luminance};

/// WCAG's own anchor points, so the math below is trusted before it is used.
#[test]
fn contrast_ratio_matches_the_wcag_reference_values() {
    assert!(
        (contrast_ratio([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 0.01,
        "pure black on pure white must be 21:1"
    );
    // #767676 on #FFFFFF is the classic "smallest grey passing AA" example.
    assert!(
        (contrast_ratio([118, 118, 118], [255, 255, 255]) - 4.54).abs() < 0.01,
        "#767676 on #FFFFFF must be ~4.54:1"
    );
    assert!(
        (contrast_ratio([153, 153, 153], [255, 255, 255]) - 2.85).abs() < 0.01,
        "#999999 on #FFFFFF must be ~2.85:1"
    );
    // Order independence: the ratio of a pair does not depend on argument
    // order, because the lighter colour is always the numerator.
    assert_eq!(
        contrast_ratio([255, 255, 255], [118, 118, 118]),
        contrast_ratio([118, 118, 118], [255, 255, 255])
    );
    assert_eq!(relative_luminance([255, 255, 255]), 1.0);
    assert_eq!(relative_luminance([0, 0, 0]), 0.0);
}

/// The pair every unstyled character draws in — the reading path — clears
/// AA for every theme, including the frozen dark default.
#[test]
fn every_theme_default_pair_meets_aa() {
    for name in [ThemeName::Dark, ThemeName::Light, ThemeName::HighContrast] {
        let theme = name.palette();
        let ratio = contrast_ratio(theme.foreground_u8(), theme.background_u8());
        assert!(
            ratio >= 4.5,
            "{name}: default pair is {ratio:.2}:1, below the AA 4.5:1 floor"
        );
    }
}

/// The light theme was designed, not assumed: every ANSI slot keeps AA on
/// the light background. Measured minimum: 5.07:1 (`bright green`).
#[test]
fn light_theme_keeps_aa_on_every_theme_owned_foreground() {
    let (min, slot) = ThemeName::Light
        .palette()
        .min_contrast_on_default_background();
    assert!(
        min >= 4.5,
        "light theme's worst slot ({slot}) is {min:.2}:1, below the AA 4.5:1 floor"
    );
}

/// High-contrast genuinely exceeds the others: it clears AAA (7:1) and its
/// measured minimum strictly beats both other themes' minima. Measured
/// minimum: 7.84:1 (`bright black`), against light's 5.07:1 and dark's
/// 1.06:1.
#[test]
fn high_contrast_meets_aaa_and_strictly_exceeds_the_other_themes() {
    let (hc_min, hc_slot) = ThemeName::HighContrast
        .palette()
        .min_contrast_on_default_background();
    assert!(
        hc_min >= 7.0,
        "high-contrast's worst slot ({hc_slot}) is {hc_min:.2}:1, below the AAA 7:1 floor"
    );
    let (light_min, _) = ThemeName::Light
        .palette()
        .min_contrast_on_default_background();
    let (dark_min, _) = ThemeName::Dark
        .palette()
        .min_contrast_on_default_background();
    assert!(
        hc_min > light_min && hc_min > dark_min,
        "high-contrast minimum {hc_min:.2}:1 must exceed light's {light_min:.2}:1 \
         and dark's {dark_min:.2}:1"
    );
}

/// The reported finding, pinned: the existing default palette's worst
/// theme-owned foreground is ANSI black at 1.06:1 — far below AA — and four
/// more slots (blue 2.10, red 3.38, bright blue 4.16, magenta 4.21) also
/// fail. The value is pinned so that *any* change to the default palette —
/// a fix or a regression — breaks this test and forces the decision into
/// the open. Fixing it is follow-up work precisely because the defaults
/// must keep rendering byte-identically to the pre-theme renderer.
#[test]
fn default_dark_palette_minimum_is_pinned_below_the_aa_floor() {
    let (min, slot) = ThemeName::Dark
        .palette()
        .min_contrast_on_default_background();
    assert_eq!(slot, "black", "the default's worst slot is ANSI black");
    assert!(
        (min - 1.0639).abs() < 0.001,
        "default dark minimum moved to {min:.4}:1 (was 1.0639:1) — changing the \
         shipped default palette is a deliberate decision, not a side effect"
    );
    assert!(
        min < 4.5,
        "if the default now passes AA, update the finding documentation"
    );
}

/// The dark theme is the pre-theme renderer's exact colours: the raw shader
/// floats, the xterm ANSI table, and the shared cube/grayscale tail. This
/// is the palette-level half of the defaults-preservation contract; the
/// frame oracle proves the pixel-level half.
#[test]
fn dark_theme_is_byte_identical_to_the_pre_theme_renderer() {
    let dark = ThemeName::Dark.palette();
    // The exact floats the fragment shader returned as constants before
    // themes existed (issue #107/#112); the frame oracle pins their captured
    // form at [204, 235, 209].
    assert_eq!(dark.foreground(), [0.80, 0.92, 0.82]);
    assert_eq!(dark.background(), [0.035, 0.045, 0.04]);
    // The xterm sixteen.
    assert_eq!(
        dark.ansi(),
        [
            [0, 0, 0],
            [205, 0, 0],
            [0, 205, 0],
            [205, 205, 0],
            [0, 0, 238],
            [205, 0, 205],
            [0, 205, 205],
            [229, 229, 229],
            [127, 127, 127],
            [255, 0, 0],
            [0, 255, 0],
            [255, 255, 0],
            [92, 92, 255],
            [255, 0, 255],
            [0, 255, 255],
            [255, 255, 255],
        ]
    );
    // The shared tail: cube corners and grayscale endpoints.
    let palette = dark.indexed_palette();
    assert_eq!(palette[16], [0, 0, 0]);
    assert_eq!(palette[196], [255, 0, 0]);
    assert_eq!(palette[231], [255, 255, 255]);
    assert_eq!(palette[232], [8, 8, 8]);
    assert_eq!(palette[244], [128, 128, 128]);
    assert_eq!(palette[255], [238, 238, 238]);
    // The default theme is dark, and dark is the Theme default.
    assert_eq!(Theme::default(), ThemeName::default().palette());
    assert_eq!(ThemeName::default(), ThemeName::Dark);
}

/// The theme's reported 8-bit foreground/background are the values the
/// RGBA8 target stores: `(channel * 255).round()`. Measuring anything else
/// would hide the quantization every drawn pixel undergoes.
#[test]
fn theme_u8_values_match_the_drawn_quantization() {
    for name in [ThemeName::Dark, ThemeName::Light, ThemeName::HighContrast] {
        let theme = name.palette();
        for (f, u) in theme.foreground().into_iter().zip(theme.foreground_u8()) {
            assert_eq!(u, (f * 255.0).round() as u8, "{name} foreground");
        }
        for (f, u) in theme.background().into_iter().zip(theme.background_u8()) {
            assert_eq!(u, (f * 255.0).round() as u8, "{name} background");
        }
    }
    assert_eq!(ThemeName::Dark.palette().foreground_u8(), [204, 235, 209]);
    assert_eq!(ThemeName::Dark.palette().background_u8(), [9, 11, 10]);
}

/// Theme names are a closed, case-sensitive vocabulary with no fallback:
/// exactly the three documented spellings parse, and everything else —
/// including near-misses like `Dark`, `highcontrast`, or the empty string —
/// is rejected for the typed configuration error to report.
#[test]
fn theme_names_parse_exactly_the_documented_vocabulary() {
    assert_eq!(ThemeName::parse("dark"), Some(ThemeName::Dark));
    assert_eq!(ThemeName::parse("light"), Some(ThemeName::Light));
    assert_eq!(
        ThemeName::parse("high-contrast"),
        Some(ThemeName::HighContrast)
    );
    for rejected in [
        "Dark",
        "LIGHT",
        "highcontrast",
        "high_contrast",
        "",
        "sepia",
        " ",
    ] {
        assert_eq!(ThemeName::parse(rejected), None, "{rejected:?} parsed");
    }
    let names: Vec<&str> = ThemeName::NAMES.to_vec();
    assert_eq!(names, ["dark", "light", "high-contrast"]);
    for name in [ThemeName::Dark, ThemeName::Light, ThemeName::HighContrast] {
        assert_eq!(ThemeName::parse(name.as_str()), Some(name));
    }
}

/// The three themes are genuinely different palettes, not three names for
/// one table — the property the frame oracle proves at the pixel level.
#[test]
fn the_built_in_themes_are_distinct_palettes() {
    let themes = [
        ThemeName::Dark.palette(),
        ThemeName::Light.palette(),
        ThemeName::HighContrast.palette(),
    ];
    for (index, first) in themes.iter().enumerate() {
        for second in themes.iter().skip(index + 1) {
            assert_ne!(first, second);
            assert_ne!(first.ansi(), second.ansi());
            assert_ne!(first.foreground(), second.foreground());
            assert_ne!(first.background(), second.background());
        }
    }
    // Every theme keeps the shared cube tail identical, so the two
    // spellings of one indexed colour agree under every theme.
    for (index, color) in themes[0].indexed_palette().iter().enumerate().skip(16) {
        assert_eq!(themes[1].indexed_palette()[index], *color);
        assert_eq!(themes[2].indexed_palette()[index], *color);
    }
}
