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
//! # The dark-palette fix (issue #168)
//!
//! The default (`dark`) used to fail 4.5:1 for five ANSI slots on its own
//! background, worst at ANSI black 1.06:1 — text a program could emit and a
//! user could not see. Issue #168 made the product decision PR #167 deferred
//! and moved exactly those five entries the minimum distance that clears AA;
//! the before/after measurements are recorded in `src/theme.rs` and pinned
//! below. The theme's measured minimum is now 4.50:1 (`magenta`, the
//! tightest of the five minimum moves).

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
/// AA for every theme, including the default dark theme.
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

/// The cursor colour is theme-owned and therefore inside the contrast
/// contract (issues #197/#200): a caret a user cannot locate is as much a
/// defect as text they cannot read. Every built-in theme draws the cursor
/// in its default foreground, so the measured ratios are the default-pair
/// ratios — dark 15.39:1, light 14.56:1, high-contrast 21.0:1 — each
/// clearing its theme's documented floor (AA for dark/light, AAA for
/// high-contrast) with the same margin as ordinary text.
#[test]
fn every_theme_cursor_colour_meets_its_theme_contrast_floor() {
    for name in [ThemeName::Dark, ThemeName::Light, ThemeName::HighContrast] {
        let theme = name.palette();
        let ratio = contrast_ratio(theme.cursor_u8(), theme.background_u8());
        let floor = if matches!(name, ThemeName::HighContrast) {
            7.0
        } else {
            4.5
        };
        assert!(
            ratio >= floor,
            "{name}: cursor colour is {ratio:.2}:1, below the {floor}:1 floor"
        );
        // A block cursor is the reading pair inverted (block in the cursor
        // colour, glyph in the background colour); WCAG ratios are
        // order-independent, so the glyph inside the block measures the
        // same ratio as the block itself. Pin that identity structurally.
        assert_eq!(
            contrast_ratio(theme.cursor_u8(), theme.background_u8()),
            contrast_ratio(theme.foreground_u8(), theme.background_u8()),
            "{name}: cursor colour must inherit the measured default-pair contrast"
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
/// 4.50:1.
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

/// The dark theme now keeps AA on every theme-owned foreground — the fix
/// issue #168 made after PR #167 had pinned the 1.06:1 failure. Every ANSI
/// slot and the default foreground must clear 4.5:1 on the dark background;
/// the measured minimum is pinned so a future palette edit — regression or
/// further fix — is a deliberate, visible decision.
#[test]
fn dark_theme_keeps_aa_on_every_theme_owned_foreground() {
    let (min, slot) = ThemeName::Dark
        .palette()
        .min_contrast_on_default_background();
    assert!(
        min >= 4.5,
        "dark theme's worst slot ({slot}) is {min:.2}:1, below the AA 4.5:1 floor"
    );
    assert_eq!(
        slot, "magenta",
        "the fixed dark palette's worst slot moved — update this pin and the docs"
    );
    assert!(
        (min - 4.5025).abs() < 0.001,
        "dark minimum moved to {min:.4}:1 (was 4.5025:1 after issue #168) — \
         changing the shipped default palette is a deliberate decision, not a \
         side effect"
    );
}

/// Each of the five entries issue #168 fixed now measures above the floor it
/// used to fail, by the minimum move: the pinned ratios are the before/after
/// record of the fix, and reverting any entry to its pre-fix value fails
/// here (and in the AA test above).
#[test]
fn the_five_fixed_dark_entries_measure_above_their_old_failures() {
    let dark = ThemeName::Dark.palette();
    let ground = dark.background_u8();
    let fixed = [
        ("black", [121, 121, 121], [0, 0, 0], 1.0639),
        ("red", [243, 0, 0], [205, 0, 0], 3.3800),
        ("blue", [0, 113, 255], [0, 0, 238], 2.1005),
        ("magenta", [213, 0, 213], [205, 0, 205], 4.2086),
        ("bright blue", [100, 100, 255], [92, 92, 255], 4.1640),
    ];
    for (slot, after, before, old_ratio) in fixed {
        let ratio = contrast_ratio(after, ground);
        assert!(
            ratio >= 4.5,
            "{slot} at {after:?} measures {ratio:.4}:1 — the AA fix regressed"
        );
        // The old value must still fail, so the pin cannot be satisfied by
        // reverting: the minimum move is what keeps this honest.
        let old = contrast_ratio(before, ground);
        assert!(
            old < 4.5 && (old - old_ratio).abs() < 0.001,
            "{slot}'s recorded pre-fix failure ({old_ratio:.4}:1) moved — the \
             history pin is wrong"
        );
        // And each fixed value is minimal: one channel step down must fail,
        // which is what "moved the minimum distance" means for these ramps.
        let dimmer = after.map(|channel| channel.saturating_sub(1));
        assert!(
            contrast_ratio(dimmer, ground) < 4.5 || dimmer == after,
            "{slot} at {after:?} is not the minimum passing value — {dimmer:?} \
             already clears AA",
        );
    }
    // The fixed values are what the theme actually serves.
    assert_eq!(dark.ansi()[0], [121, 121, 121]);
    assert_eq!(dark.ansi()[1], [243, 0, 0]);
    assert_eq!(dark.ansi()[4], [0, 113, 255]);
    assert_eq!(dark.ansi()[5], [213, 0, 213]);
    assert_eq!(dark.ansi()[12], [100, 100, 255]);
}

/// The dark theme is the pre-theme renderer's colours except the five
/// issue-168 fixes: the raw shader floats, the shared cube/grayscale tail,
/// and eleven of the sixteen ANSI entries are still byte-identical, and the
/// five that moved are pinned to their fixed values (blue kept red at zero,
/// magenta stays symmetric R=B, bright blue keeps its R=G balance, black
/// stays achromatic — slot semantics preserved). This is the palette-level
/// half of the pin; the frame oracle proves the pixel-level half.
#[test]
fn dark_theme_palette_is_pinned_with_the_issue_168_aa_fixes() {
    let dark = ThemeName::Dark.palette();
    // The exact floats the fragment shader returned as constants before
    // themes existed (issue #107/#112); the frame oracle pins their captured
    // form at [204, 235, 209].
    assert_eq!(dark.foreground(), [0.80, 0.92, 0.82]);
    assert_eq!(dark.background(), [0.035, 0.045, 0.04]);
    // The xterm sixteen with the five AA fixes (issue #168): entries 0, 1,
    // 4, 5, and 12 moved the minimum distance to clear 4.5:1; the other
    // eleven are the untouched xterm values.
    assert_eq!(
        dark.ansi(),
        [
            [121, 121, 121],
            [243, 0, 0],
            [0, 205, 0],
            [205, 205, 0],
            [0, 113, 255],
            [213, 0, 213],
            [0, 205, 205],
            [229, 229, 229],
            [127, 127, 127],
            [255, 0, 0],
            [0, 255, 0],
            [255, 255, 0],
            [100, 100, 255],
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
