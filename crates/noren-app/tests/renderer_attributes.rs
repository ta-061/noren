//! Attribute wiring for the renderer, verified without a GPU.
//!
//! `renderer.rs` belongs to the binary, and the lease for this lane keeps
//! `lib.rs` and `main.rs` untouched, so the module is included directly and
//! its pure color-resolution functions and vertex assembly are called here.
//! No rendered-frame oracle exists; these tests cover the resolution math and
//! the vertex stream it produces, not pixels on screen.

#[path = "../src/renderer.rs"]
// The GPU-facing surface compiles into this headless test crate but cannot
// run without a window; silence dead-code analysis for that part only.
#[allow(dead_code)]
mod renderer;

use noren_terminal::{AnsiColor, CellAttributes, Color, TerminalSnapshot, TerminalState};
use renderer::{
    DEFAULT_ANSI_PALETTE, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND, DEFAULT_PALETTE,
    ResolvedCellColors, Vertex, glyph_vertices, resolve_cell_colors, resolve_color,
};

const WINDOW_WIDTH: u32 = 900;
const WINDOW_HEIGHT: u32 = 600;
const CELL_WIDTH: f32 = 10.0;
const CELL_HEIGHT: f32 = 20.0;

fn snapshot(rows: u16, cols: u16, bytes: &[u8]) -> TerminalSnapshot {
    let mut terminal = TerminalState::new(rows, cols).expect("valid test terminal");
    terminal.feed_bytes(bytes);
    terminal.snapshot()
}

fn vertices(rows: u16, cols: u16, bytes: &[u8]) -> Vec<Vertex> {
    glyph_vertices(
        Some(&snapshot(rows, cols, bytes)),
        None,
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    )
}

fn rgb_f32([red, green, blue]: [u8; 3]) -> [f32; 3] {
    [
        f32::from(red) / 255.0,
        f32::from(green) / 255.0,
        f32::from(blue) / 255.0,
    ]
}

fn ndc_left(column: u32) -> f32 {
    column as f32 * CELL_WIDTH / WINDOW_WIDTH as f32 * 2.0 - 1.0
}

fn ndc_width(pixels: f32) -> f32 {
    pixels / WINDOW_WIDTH as f32 * 2.0
}

#[test]
fn every_color_variant_resolves_to_a_concrete_rgb() {
    assert_eq!(
        resolve_color(Color::Default, DEFAULT_FOREGROUND),
        DEFAULT_FOREGROUND
    );
    assert_eq!(
        resolve_color(Color::Default, DEFAULT_BACKGROUND),
        DEFAULT_BACKGROUND
    );

    for ansi in AnsiColor::ALL {
        assert_eq!(
            resolve_color(Color::Ansi(ansi), DEFAULT_FOREGROUND),
            DEFAULT_PALETTE[usize::from(ansi.palette_index())]
        );
    }
    assert_eq!(
        resolve_color(Color::Ansi(AnsiColor::Red), DEFAULT_FOREGROUND),
        [205, 0, 0]
    );
    assert_eq!(
        resolve_color(Color::Ansi(AnsiColor::BrightWhite), DEFAULT_FOREGROUND),
        [255, 255, 255]
    );

    // Indexed entries 0..=15 are the ANSI palette; the cube corners and the
    // grayscale ramp endpoints follow the xterm definition.
    for index in 0..16_u8 {
        assert_eq!(
            resolve_color(Color::Indexed(index), DEFAULT_BACKGROUND),
            DEFAULT_ANSI_PALETTE[usize::from(index)]
        );
    }
    assert_eq!(
        resolve_color(Color::Indexed(16), DEFAULT_BACKGROUND),
        [0, 0, 0]
    );
    assert_eq!(
        resolve_color(Color::Indexed(196), DEFAULT_BACKGROUND),
        [255, 0, 0]
    );
    assert_eq!(
        resolve_color(Color::Indexed(231), DEFAULT_BACKGROUND),
        [255, 255, 255]
    );
    assert_eq!(
        resolve_color(Color::Indexed(232), DEFAULT_BACKGROUND),
        [8, 8, 8]
    );
    assert_eq!(
        resolve_color(Color::Indexed(255), DEFAULT_BACKGROUND),
        [238, 238, 238]
    );

    assert_eq!(
        resolve_color(Color::Rgb(1, 2, 3), DEFAULT_FOREGROUND),
        [1, 2, 3]
    );
}

#[test]
fn default_cells_resolve_to_the_default_palette_entries() {
    let resolved = resolve_cell_colors(&CellAttributes::default());
    let expected = ResolvedCellColors {
        foreground: DEFAULT_FOREGROUND,
        background: DEFAULT_BACKGROUND,
        underline: DEFAULT_FOREGROUND,
    };
    assert_eq!(resolved, expected);

    // A plain cell drawn through the full snapshot path uses the default
    // foreground for every emitted vertex and no background rect.
    let output = vertices(1, 2, b"A");
    assert!(!output.is_empty());
    let fg = rgb_f32(DEFAULT_FOREGROUND);
    assert!(output.iter().all(|vertex| vertex.color == fg));
}

#[test]
fn reverse_swaps_foreground_and_background() {
    let resolved = resolve_cell_colors(
        &CellAttributes::new()
            .with_foreground(Color::Ansi(AnsiColor::Red))
            .with_background(Color::Ansi(AnsiColor::Green))
            .with_reverse(true),
    );
    assert_eq!(resolved.foreground, DEFAULT_ANSI_PALETTE[2]);
    assert_eq!(resolved.background, DEFAULT_ANSI_PALETTE[1]);
    // A default underline color follows the swapped foreground.
    assert_eq!(resolved.underline, DEFAULT_ANSI_PALETTE[2]);
}

#[test]
fn reverse_applies_after_palette_resolution_and_composes_with_explicit_colors() {
    // Default foreground plus indexed background: reverse must swap the
    // resolved colors, so the foreground becomes the indexed cube red and the
    // background becomes the default foreground — not the other way around.
    let resolved = resolve_cell_colors(
        &CellAttributes::new()
            .with_background(Color::Indexed(196))
            .with_reverse(true),
    );
    assert_eq!(resolved.foreground, [255, 0, 0]);
    assert_eq!(resolved.background, DEFAULT_FOREGROUND);

    // Rgb foreground and ANSI background compose through the same swap.
    let resolved = resolve_cell_colors(
        &CellAttributes::new()
            .with_foreground(Color::Rgb(10, 20, 30))
            .with_background(Color::Ansi(AnsiColor::Blue))
            .with_reverse(true),
    );
    assert_eq!(resolved.foreground, [0, 0, 238]);
    assert_eq!(resolved.background, [10, 20, 30]);

    // An explicit underline color is not moved by reverse.
    let resolved = resolve_cell_colors(
        &CellAttributes::new()
            .with_underline_color(Color::Rgb(9, 8, 7))
            .with_reverse(true),
    );
    assert_eq!(resolved.underline, [9, 8, 7]);
}

#[test]
fn reverse_video_swaps_colors_in_the_draw_output() {
    // SGR 7 on a default-colored A: the cell background rect draws the
    // default foreground and the glyph draws the default background.
    let output = vertices(1, 2, b"\x1b[7mA");

    let background = output.chunks_exact(6).find(|rect| {
        rect[0].position == [-1.0, 1.0]
            && (rect[2].position[0] - rect[0].position[0] - ndc_width(CELL_WIDTH)).abs() < 1e-6
            && (rect[0].position[1]
                - rect[2].position[1]
                - 2.0 * CELL_HEIGHT / WINDOW_HEIGHT as f32)
                .abs()
                < 1e-6
    });
    let background = background.expect("a reversed default cell paints a background rect");
    assert!(
        background
            .iter()
            .all(|vertex| vertex.color == rgb_f32(DEFAULT_FOREGROUND))
    );

    let glyph_color = rgb_f32(DEFAULT_BACKGROUND);
    assert!(
        output
            .chunks_exact(6)
            .filter(|rect| rect[0].position[1] < 1.0)
            .flat_map(|rect| rect.iter())
            .all(|vertex| vertex.color == glyph_color),
        "every glyph vertex draws the swapped (default background) color"
    );
}

#[test]
fn bold_is_represented_in_the_draw_output() {
    let plain = vertices(1, 2, b"A");
    let bold = vertices(1, 2, b"\x1b[1mA");

    assert_eq!(
        plain.len(),
        bold.len(),
        "bold adds no rects, it widens them"
    );
    assert_ne!(plain, bold);
    for (plain_rect, bold_rect) in plain.chunks_exact(6).zip(bold.chunks_exact(6)) {
        assert_eq!(
            plain_rect[0].position, bold_rect[0].position,
            "bold keeps the glyph's top-left placement"
        );
        let plain_width = plain_rect[2].position[0] - plain_rect[0].position[0];
        let bold_width = bold_rect[2].position[0] - bold_rect[0].position[0];
        assert!((plain_width - ndc_width(2.0)).abs() < 1e-6);
        assert!((bold_width - ndc_width(3.0)).abs() < 1e-6);
    }
}

#[test]
fn underline_is_represented_in_the_draw_output() {
    let plain = vertices(1, 2, b"A");
    let underlined = vertices(1, 2, b"\x1b[4mA");
    assert!(underlined.len() > plain.len());

    // The underline bar sits at the cell bottom, spans the cell, and uses the
    // default foreground when no explicit underline color is set.
    let bar_top = 1.0 - (CELL_HEIGHT - 2.0) / WINDOW_HEIGHT as f32 * 2.0;
    let bar = underlined.chunks_exact(6).find(|rect| {
        rect[0].position == [-1.0, bar_top]
            && (rect[2].position[0] - rect[0].position[0] - ndc_width(CELL_WIDTH)).abs() < 1e-6
    });
    let bar = bar.expect("an underlined cell draws a bar across the cell bottom");
    assert!(
        bar.iter()
            .all(|vertex| vertex.color == rgb_f32(DEFAULT_FOREGROUND))
    );

    // SGR 58 selects an explicit underline color that the bar follows.
    let colored = vertices(1, 2, b"\x1b[4;58;2;200;100;50mA");
    let bar = colored
        .chunks_exact(6)
        .find(|rect| rect[0].position == [-1.0, bar_top]);
    let bar = bar.expect("the explicit underline color still draws the bar");
    assert!(
        bar.iter()
            .all(|vertex| vertex.color == rgb_f32([200, 100, 50]))
    );
}

#[test]
fn wide_characters_keep_their_two_column_footprint_with_attributes() {
    // SGR 44 (blue background) on a wide 日 followed by a reset and b.
    let output = vertices(1, 6, b"\x1b[44m\xe6\x97\xa5\x1b[0mb");
    let blue = rgb_f32(DEFAULT_ANSI_PALETTE[4]);

    // Exactly one blue rect, spanning both columns of the wide lead.
    let blue_rects: Vec<_> = output
        .chunks_exact(6)
        .filter(|rect| rect[0].color == blue)
        .collect();
    assert_eq!(blue_rects.len(), 1, "the continuation column adds no fill");
    let rect = blue_rects[0];
    assert_eq!(rect[0].position, [-1.0, 1.0]);
    assert!((rect[2].position[0] - rect[0].position[0] - ndc_width(2.0 * CELL_WIDTH)).abs() < 1e-6);
    assert!(
        (rect[0].position[1] - rect[2].position[1] - 2.0 * CELL_HEIGHT / WINDOW_HEIGHT as f32)
            .abs()
            < 1e-6
    );

    // Glyphs remain positioned by display column: b starts at display column
    // 2 and nothing draws at the continuation column's lead edge.
    let glyph_top = 1.0 - 3.0 / WINDOW_HEIGHT as f32 * 2.0;
    let has_glyph_at = |column: u32| {
        output
            .chunks_exact(6)
            .any(|rect| rect[0].position == [ndc_left(column), glyph_top])
    };
    assert!(has_glyph_at(2));
    assert!(!has_glyph_at(1));
}
