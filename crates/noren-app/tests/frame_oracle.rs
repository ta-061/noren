//! FR-005 rendered-frame oracle.
//!
//! This is the missing rendered-frame half of FR-005: it drives the **real**
//! `wgpu` render pipeline offscreen, reads the pixels back, and asserts that
//! what the renderer *draws* matches what the terminal state *says*. The
//! state-snapshot half already existed; this mechanically verifies glyph
//! correctness and grid mapping for the first time (see `ROADMAP.md:67-70`).
//!
//! ## What it proves (and how)
//!
//! Assertions are structural, never golden-image (a byte-exact PNG would flake
//! across GPUs and be disabled within a week):
//!
//! - a cell the state says is blank contains no lit pixels;
//! - a cell the state says holds `A` is lit and its lit pattern differs from a
//!   cell holding `B`;
//! - text at `(row, col)` lights pixels **inside** that cell's rectangle and
//!   not in its neighbours — this catches off-by-one grid mapping, the most
//!   likely real defect;
//! - the drawn grid dimensions match the state's dimensions;
//! - per-cell, lit/blank agrees with `TerminalSnapshot` across the FR-005
//!   fixture classes (prompt, ASCII, UTF-8, control, scrolling);
//! - two cells carrying different SGR foreground colours draw *different*
//!   pixel colours, each matching the palette; a cell with no SGR colour keeps
//!   the unchanged default; truecolor and 256-colour both resolve to their
//!   expected values (issue #107).
//!
//! ## The lit/blank gate
//!
//! The oracle distinguishes an untouched clear pixel from a terminal-painted
//! background pixel. `is_clear` is exact and drives lit/blank and neighbour
//! assertions; `is_background` means that a pixel matches the specific
//! expected terminal background for its cell (or the clear colour when the
//! cell has no explicit background). Neither predicate accepts arbitrary dark
//! pixels, so dropping a glyph, leaking into a neighbour, or skipping a
//! background rectangle remains observable.
//!
//! ## Faithfulness
//!
//! [`renderer_capture`] re-includes the shipped `renderer.rs` and draws with the
//! same shader + glyph vertex generation, so a vertex/glyph/grid defect in the
//! binary is caught here, not hidden behind a parallel implementation.
//!
//! ## Defects surfaced
//!
//! The assertions below are written the way a *correct* renderer requires:
//!
//! - `lowercase_distinct_from_uppercase` guards the fixed case-folding defect:
//!   `a` and `A` must render differently.
//! - `non_ascii_glyph_is_not_the_question_mark`: every non-ASCII code point
//!   falls to the `?` default arm, so `日` renders as `?`.

#[path = "../src/renderer_capture.rs"]
mod renderer_capture;

use noren_app::{
    GridGeometry, MAX_RENDER_COLS, MAX_RENDER_ROWS, POC_CELL_HEIGHT as CELL_HEIGHT,
    POC_CELL_WIDTH as CELL_WIDTH,
};
use noren_terminal::{TerminalSnapshot, TerminalState};
use renderer_capture::renderer_source::{
    CLEAR_COLOR, DEFAULT_ANSI_PALETTE, DEFAULT_FOREGROUND, DEFAULT_PALETTE, SIDEBAR_COLS,
};
use renderer_capture::{CaptureError, CapturedFrame, OffscreenRenderer};

/// The PoC default cell metrics, used by every frame-oracle capture call so
/// the renderer draws at the same size the geometry computed.
fn poc_metrics() -> noren_app::CellMetrics {
    GridGeometry::poc().cell_metrics()
}

/// Construct a snapshot by feeding `bytes` through the real terminal state.
fn snapshot(rows: u16, cols: u16, bytes: &[u8]) -> TerminalSnapshot {
    let mut terminal = TerminalState::new(rows, cols).expect("valid test terminal");
    terminal.feed_bytes(bytes);
    terminal.snapshot()
}

/// Render a snapshot at exactly its grid size in PoC pixels.
fn render(renderer: &OffscreenRenderer, snapshot: &TerminalSnapshot) -> CapturedFrame {
    let width = u32::from(snapshot.cols()) * CELL_WIDTH;
    let height = u32::from(snapshot.rows()) * CELL_HEIGHT;
    renderer.capture(Some(snapshot), None, None, width, height, poc_metrics())
}

/// The clear colour as captured bytes, derived from the renderer's own
/// [`CLEAR_COLOR`] rather than a literal, so a change to the clear colour
/// cannot silently invalidate every lit/blank assertion below.
fn clear_rgb() -> [u8; 3] {
    [CLEAR_COLOR.r, CLEAR_COLOR.g, CLEAR_COLOR.b].map(|channel| (channel * 255.0).round() as u8)
}

/// Per-channel tolerance for comparing a captured pixel to an expected colour.
///
/// The offscreen target is linear `Rgba8Unorm` and the shader writes the
/// colour through unchanged, so the only error is float-to-byte rounding in
/// the GPU's blend/store stage: at most one unit. Two units gives a margin for
/// driver rounding-mode differences while staying far tighter than the gap
/// between any two palette entries (the closest pair differs by 10).
const CHANNEL_TOLERANCE: u8 = 2;

/// Whether two colours match within [`CHANNEL_TOLERANCE`] on every channel.
fn colors_match(actual: [u8; 3], expected: [u8; 3]) -> bool {
    actual
        .iter()
        .zip(expected.iter())
        .all(|(a, e)| a.abs_diff(*e) <= CHANNEL_TOLERANCE)
}

/// Whether a pixel is untouched by every draw primitive.
fn is_clear(rgba: [u8; 4]) -> bool {
    [rgba[0], rgba[1], rgba[2]] == clear_rgb()
}

/// Whether a pixel matches the expected background of its cell.
///
/// `None` means the cell has no SGR background, so the only valid background
/// is the exact clear colour. `Some` names one concrete terminal colour and is
/// compared with the same tight tolerance used for shader output.
fn is_background(rgba: [u8; 4], expected: Option<[u8; 3]>) -> bool {
    let actual = [rgba[0], rgba[1], rgba[2]];
    expected.map_or_else(|| is_clear(rgba), |color| colors_match(actual, color))
}

/// Whether any lit (non-background) pixel falls inside cell `(row, col)`.
fn cell_is_lit(frame: &CapturedFrame, row: u32, col: u32) -> bool {
    let x0 = col * CELL_WIDTH;
    let y0 = row * CELL_HEIGHT;
    for y in y0..y0 + CELL_HEIGHT {
        for x in x0..x0 + CELL_WIDTH {
            if !is_clear(frame.pixel(x, y)) {
                return true;
            }
        }
    }
    false
}

/// The per-pixel lit/blank signature of a cell — a fingerprint of which glyph
/// was drawn there. Two cells holding different glyphs must differ here; two
/// holding the same glyph must match.
fn cell_pattern(frame: &CapturedFrame, row: u32, col: u32) -> Vec<bool> {
    let mut pattern = Vec::with_capacity((CELL_WIDTH * CELL_HEIGHT) as usize);
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            pattern.push(!is_clear(
                frame.pixel(col * CELL_WIDTH + x, row * CELL_HEIGHT + y),
            ));
        }
    }
    pattern
}

/// The distinct lit (non-background) colours inside cell `(row, col)`.
///
/// This is the colour-aware counterpart of [`cell_pattern`]: where that
/// records only *which* pixels are lit, this records *what colour* they are,
/// which is what makes "two cells with different SGR colours differ" an
/// assertion about colour rather than about glyph shape.
fn cell_colors(frame: &CapturedFrame, row: u32, col: u32) -> Vec<[u8; 3]> {
    let mut colors: Vec<[u8; 3]> = Vec::new();
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let pixel = frame.pixel(col * CELL_WIDTH + x, row * CELL_HEIGHT + y);
            if is_clear(pixel) {
                continue;
            }
            let rgb = [pixel[0], pixel[1], pixel[2]];
            if !colors.iter().any(|seen| colors_match(*seen, rgb)) {
                colors.push(rgb);
            }
        }
    }
    colors
}

/// The single colour a cell's glyph is drawn in.
///
/// Panics if the cell is unlit or holds more than one distinct colour — a
/// glyph is drawn in exactly one foreground colour, so either case means the
/// renderer is not doing what the test assumes and the test must fail loudly
/// rather than silently pick a colour.
fn cell_color(frame: &CapturedFrame, row: u32, col: u32) -> [u8; 3] {
    let colors = cell_colors(frame, row, col);
    assert_eq!(
        colors.len(),
        1,
        "cell ({row},{col}) should hold exactly one lit colour, found {colors:?}"
    );
    colors[0]
}

/// The default foreground as captured bytes, derived from the renderer's own
/// [`DEFAULT_FOREGROUND`] so the "unstyled cells are unchanged" assertion
/// tracks the renderer rather than a literal copied here.
fn default_foreground_rgb() -> [u8; 3] {
    DEFAULT_FOREGROUND.map(|channel| (channel * 255.0).round() as u8)
}

/// State-driven blankness: a cell is blank when the row is absent, the column
/// is past the line end, or the cell is an ASCII space (the space glyph is the
/// all-zero row, so it draws nothing).
///
/// Reads `display_lines()` — the renderer's coordinate model — not `lines()`.
/// They agree for ASCII; only `display_lines()` keeps a wide character's
/// continuation column aligned with the renderer's per-column enumeration, so
/// the first wide-character fixture is compared against the same columns the
/// renderer actually draws.
fn state_cell_blank(snapshot: &TerminalSnapshot, row: u32, col: u32) -> bool {
    let line = snapshot
        .display_lines()
        .get(row as usize)
        .map(String::as_str);
    match line {
        None => true,
        Some(line) => match line.chars().nth(col as usize) {
            None | Some(' ') => true,
            Some(_) => false,
        },
    }
}

/// Assert every visible cell agrees: state-blank ⟺ render-unlit. Returns the
/// number of cells checked so the oracle can report coverage.
fn assert_cells_agree(frame: &CapturedFrame, snapshot: &TerminalSnapshot) -> usize {
    let rows = u32::from(snapshot.rows());
    let cols = u32::from(snapshot.cols());
    let mut checked = 0;
    for row in 0..rows {
        for col in 0..cols {
            let state_blank = state_cell_blank(snapshot, row, col);
            let render_blank = !cell_is_lit(frame, row, col);
            assert_eq!(
                state_blank,
                render_blank,
                "cell ({row},{col}): state says {}, renderer drew {}",
                if state_blank { "blank" } else { "char" },
                if render_blank { "blank" } else { "lit" },
            );
            checked += 1;
        }
    }
    checked
}

// ===========================================================================
// Gate 0: can wgpu initialise headlessly on this machine? If not, the oracle
// reports `offscreen=blocked` with the exact failure rather than faking success.
// ===========================================================================

#[test]
fn offscreen_wgpu_pipeline_initialises() {
    match OffscreenRenderer::new() {
        Ok(_) => { /* offscreen=ok */ }
        Err(CaptureError::AdapterUnavailable) => {
            panic!("offscreen=blocked: no Metal adapter without a display surface");
        }
        Err(CaptureError::DeviceUnavailable) => {
            panic!("offscreen=blocked: adapter present but device request failed");
        }
    }
}

// ===========================================================================
// Core oracle: geometry, containment, and state agreement.
// ===========================================================================

#[test]
fn blank_screen_has_no_lit_pixels() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(3, 8, b"");
    let frame = render(&renderer, &snap);

    for row in 0..u32::from(snap.rows()) {
        for col in 0..u32::from(snap.cols()) {
            assert!(
                !cell_is_lit(&frame, row, col),
                "blank cell ({row},{col}) is lit"
            );
        }
    }
}

#[test]
fn an_ascii_cell_is_lit_and_its_neighbours_are_blank() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(1, 4, b"A");
    let frame = render(&renderer, &snap);

    // The 'A' must light its own cell ...
    assert!(cell_is_lit(&frame, 0, 0), "cell holding 'A' is not lit");
    // ... and must not bleed into any neighbour. This is the off-by-one guard.
    for col in 1..u32::from(snap.cols()) {
        assert!(
            !cell_is_lit(&frame, 0, col),
            "glyph bled from column 0 into neighbour column {col}"
        );
    }
    // Vertical neighbours across a row boundary too.
    let snap2 = snapshot(2, 4, b"A");
    let frame2 = render(&renderer, &snap2);
    assert!(cell_is_lit(&frame2, 0, 0));
    assert!(
        !cell_is_lit(&frame2, 1, 0),
        "glyph bled from row 0 into row 1"
    );

    // An interior placement: 'A' at (1,1) in a 3x3 grid, so all four cardinal
    // neighbours exist and must stay blank. The corner placements above can
    // only reach rightward and downward; this additionally catches upward and
    // leftward bleed from a sub-cell origin offset at interior positions.
    let snap3 = snapshot(3, 3, b"\x1b[2;2HA");
    let frame3 = render(&renderer, &snap3);
    assert!(
        cell_is_lit(&frame3, 1, 1),
        "interior cell holding 'A' is not lit"
    );
    for (nr, nc) in [(0_u32, 1_u32), (2, 1), (1, 0), (1, 2)] {
        assert!(
            !cell_is_lit(&frame3, nr, nc),
            "glyph bled from (1,1) into neighbour ({nr},{nc})"
        );
    }
}

#[test]
fn distinct_ascii_glyphs_have_distinct_lit_patterns() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(1, 2, b"AB");
    let frame = render(&renderer, &snap);

    let a = cell_pattern(&frame, 0, 0);
    let b = cell_pattern(&frame, 0, 1);
    assert!(!a.iter().all(|&lit| !lit), "'A' cell drew nothing");
    assert!(!b.iter().all(|&lit| !lit), "'B' cell drew nothing");
    assert_ne!(a, b, "'A' and 'B' rendered the same lit pattern");
}

#[test]
fn drawn_grid_dimensions_match_state_dimensions() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let rows = 3_u16;
    let cols = 5_u16;
    // A deliberately ragged layout: row 0 fills the grid width, row 1 stops at
    // 4 chars, row 2 stops at 2. Asserting on *drawn pixels* — not on frame
    // metadata like `frame.width`, which `render()` sets from these same
    // `rows`/`cols` and which no broken renderer can change — is what gives
    // this test teeth.
    //
    // The ragged rows are the dimension check the old version missed entirely:
    // a renderer that paints every cell out to `cols` (ignoring per-row content
    // width) lights the trailing cells the state leaves blank; one that shifts
    // the column or row origin lights the wrong cell entirely. Both surface as
    // a lit/blank disagreement below.
    let snap = snapshot(rows, cols, b"ABCDE\r\nFGHI\r\nKL");
    let frame = render(&renderer, &snap);

    for row in 0..u32::from(rows) {
        for col in 0..u32::from(cols) {
            let state_has_char = !state_cell_blank(&snap, row, col);
            let drawn_lit = cell_is_lit(&frame, row, col);
            assert_eq!(
                state_has_char,
                drawn_lit,
                "cell ({row},{col}): state says {}, renderer drew {} \
                 (drawn grid does not match the state grid)",
                if state_has_char { "char" } else { "blank" },
                if drawn_lit { "lit" } else { "blank" },
            );
        }
    }
}

#[test]
fn state_and_render_agree_across_the_fr005_fixtures() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");

    // Prompt: a shell-like "$ ".
    let prompt = snapshot(2, 8, b"$ ");
    // ASCII: plain printable text.
    let ascii = snapshot(2, 8, b"hello\r\nworld");
    // Control: backspace repositions the cursor, then overwrite.
    let control = snapshot(1, 8, b"ABC\x08\x08X");
    // Scrolling: feed more lines than rows; only the last rows stay visible.
    let scrolling = {
        let mut terminal = TerminalState::new(3, 6).expect("valid test terminal");
        terminal.feed_bytes(b"aaaaaa\r\nbbbbbb\r\ncccccc\r\ndddddd\r\neeeeee");
        terminal.snapshot()
    };

    let mut checked = 0_usize;
    for (name, snap) in [
        ("prompt", &prompt),
        ("ascii", &ascii),
        ("control", &control),
        ("scrolling", &scrolling),
    ] {
        let frame = render(&renderer, snap);
        let checked_here = assert_cells_agree(&frame, snap);
        assert!(checked_here > 0, "fixture `{name}` checked zero cells");
        checked += checked_here;
    }
    // Sanity: the four fixtures must have actually exercised cells.
    assert!(
        checked >= 4 * 2 * 6,
        "agreement checks covered {checked} cells"
    );
}

// ===========================================================================
// Render clamps (issue #109): the renderer's `MAX_RENDER_ROWS` and
// `MAX_RENDER_COLS` clamps must keep every glyph inside the drawable grid.
//
// The old guard — `renderer.rs::glyph_input_is_bounded_to_visible_poc_grid` —
// asserted `vertices.len() <= MAX_VERTICES`. That cannot tell a clamp from the
// `MAX_VERTICES` early-return backstop: both cap vertex output at the same
// ceiling, and the fixture only ever reached ~half the cap (1,036,800 of
// 2,016,000), so deleting any one guard left it green — confirmed by deletion
// before this test was written. A count-based test is structurally unable to
// pin this property.
//
// What needs guarding is *where glyphs land*, so this test reads pixels back
// from the real pipeline and asserts directly that no cell past either clamp
// boundary is lit. The fixture places three single-cell markers via CUP on a
// grid one past each clamp limit, rather than filling it: a row detector at the
// overflow display row, a column detector at the overflow column, and a stable
// interior marker. Three glyphs keep the capture at small-grid cost (the full
// 61×161 fill took ~4.5s; this takes ~0.1s).
//
// Mutation protocol (run before trusting this test): removing the row clamp
// lights grid row `MAX_RENDER_ROWS`; removing the column clamp lights grid
// column `MAX_RENDER_COLS`. The `MAX_VERTICES` backstop cannot produce a visual
// change while the clamps hold (it is a memory guard, not a geometry guard), so
// this test does not catch its removal — see the report line.
// ===========================================================================

#[test]
fn glyphs_stay_inside_the_render_clamp_grid() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");

    // A grid one past each clamp limit in both dimensions. Three CUP-placed
    // markers carry the only lit content:
    //   - row detector  at display (60, 0):   maps to grid row 59 with the row
    //                                         clamp, grid row 60 (out of bounds)
    //                                         without it;
    //   - column detector at display (30, 160): column 160 is inside the grid
    //                                         but past the MAX_RENDER_COLS cut,
    //                                         so it draws only without the
    //                                         column clamp;
    //   - interior marker at display (30, 30): always drawn in bounds, at grid
    //                                         row 29 or 30 depending on the row
    //                                         clamp — a stable positive control.
    let rows = MAX_RENDER_ROWS + 1;
    let cols = MAX_RENDER_COLS + 1;
    let bytes = "\x1b[61;1HA\x1b[31;161HA\x1b[31;31HA".as_bytes().to_vec();
    let snap = snapshot(rows, cols, &bytes);
    // Rendered at the terminal's own grid size: one cell larger than the clamp
    // grid each way, so overflow content has pixel room to land in.
    let frame = render(&renderer, &snap);

    let max_row = u32::from(MAX_RENDER_ROWS);
    let max_col = u32::from(MAX_RENDER_COLS);

    // Positive control: the interior marker lands at grid row 29 or 30
    // (depending on the row clamp), never outside the budget. If neither is
    // lit the renderer drew nothing and the boundary checks below are vacuous.
    assert!(
        cell_is_lit(&frame, 29, 30) || cell_is_lit(&frame, 30, 30),
        "interior marker must light grid (29,30) or (30,30) — \
         otherwise the render produced nothing and the boundary checks are vacuous"
    );

    // Row clamp: no lit cell at row MAX_RENDER_ROWS. With the clamp the row
    // detector sits at grid row 59; without it, it spills into row max_row.
    // Neither the column clamp nor the MAX_VERTICES backstop can light this
    // row, so a lit cell here pins a row-clamp regression.
    for col in 0..max_col {
        assert!(
            !cell_is_lit(&frame, max_row, col),
            "cell ({max_row},{col}) is lit at MAX_RENDER_ROWS — \
             the row clamp failed to contain the grid"
        );
    }

    // Column clamp: no lit cell at column MAX_RENDER_COLS. Symmetric to the
    // row check; only a column-clamp regression lights any cell here.
    for row in 0..max_row {
        assert!(
            !cell_is_lit(&frame, row, max_col),
            "cell ({row},{max_col}) is lit at MAX_RENDER_COLS — \
             the column clamp failed to contain the grid"
        );
    }
}

// ===========================================================================
// Colour (issue #107): colour was modelled in terminal state but never reached
// drawing, so every cell rendered in one shade of green. These assertions read
// the *colour* of drawn pixels, not merely whether they are lit — the
// distinction the brightness gate above could not make.
// ===========================================================================

#[test]
fn cells_with_different_sgr_foregrounds_render_different_colours() {
    // The headline defect: before colour was wired, these two cells drew the
    // same green and this test could not exist. Both cells hold 'A', so the
    // glyph shape is identical and only the colour can differ.
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(1, 4, b"\x1b[31mA\x1b[32mA");
    let frame = render(&renderer, &snap);

    let red_cell = cell_color(&frame, 0, 0);
    let green_cell = cell_color(&frame, 0, 1);

    // Not merely both lit — actually different colours.
    assert!(
        !colors_match(red_cell, green_cell),
        "SGR 31 and SGR 32 cells rendered the same colour {red_cell:?} — \
         colour is not reaching the renderer (issue #107)"
    );
    // And each is the colour the palette says it is.
    assert!(
        colors_match(red_cell, DEFAULT_ANSI_PALETTE[1]),
        "SGR 31 should draw palette red {:?}, drew {red_cell:?}",
        DEFAULT_ANSI_PALETTE[1]
    );
    assert!(
        colors_match(green_cell, DEFAULT_ANSI_PALETTE[2]),
        "SGR 32 should draw palette green {:?}, drew {green_cell:?}",
        DEFAULT_ANSI_PALETTE[2]
    );
}

#[test]
fn a_cell_with_no_sgr_colour_keeps_the_default_appearance() {
    // The compatibility half of issue #107: wiring colour must not change how
    // an unstyled prompt looks.
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(1, 4, b"$ A");
    let frame = render(&renderer, &snap);

    let expected = default_foreground_rgb();
    for col in [0_u32, 2] {
        let drawn = cell_color(&frame, 0, col);
        assert!(
            colors_match(drawn, expected),
            "unstyled cell ({col}) drew {drawn:?}, expected the unchanged \
             default foreground {expected:?}"
        );
    }
    // Pin the default to the exact shade the fragment shader returned as a
    // constant before colour existed: 0.80/0.92/0.82 -> 204/235/209.
    assert_eq!(expected, [204, 235, 209]);
}

#[test]
fn truecolor_and_256_colour_both_resolve_to_their_expected_pixels() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");

    // Truecolor: SGR 38;2;R;G;B must produce exactly those channels. The
    // value is deliberately not a palette entry, so only a real truecolor
    // path can produce it.
    let truecolor = snapshot(1, 4, b"\x1b[38;2;17;119;221mA");
    let truecolor_frame = render(&renderer, &truecolor);
    let drawn = cell_color(&truecolor_frame, 0, 0);
    assert!(
        colors_match(drawn, [17, 119, 221]),
        "truecolor 38;2;17;119;221 drew {drawn:?}, expected [17, 119, 221] \
         within ±{CHANNEL_TOLERANCE} per channel"
    );

    // 256-colour: index 196 is the colour cube's pure red (5,0,0).
    let indexed = snapshot(1, 4, b"\x1b[38;5;196mA");
    let indexed_frame = render(&renderer, &indexed);
    let drawn_indexed = cell_color(&indexed_frame, 0, 0);
    assert!(
        colors_match(drawn_indexed, DEFAULT_PALETTE[196]),
        "256-colour index 196 drew {drawn_indexed:?}, expected {:?}",
        DEFAULT_PALETTE[196]
    );
    assert_eq!(DEFAULT_PALETTE[196], [255, 0, 0]);

    // A grayscale-ramp index resolves too, and differs from both above.
    let gray = snapshot(1, 4, b"\x1b[38;5;244mA");
    let gray_frame = render(&renderer, &gray);
    let drawn_gray = cell_color(&gray_frame, 0, 0);
    assert!(
        colors_match(drawn_gray, DEFAULT_PALETTE[244]),
        "256-colour index 244 drew {drawn_gray:?}, expected {:?}",
        DEFAULT_PALETTE[244]
    );
    assert!(!colors_match(drawn_gray, drawn_indexed));
}

#[test]
fn the_16_colour_and_256_colour_forms_of_one_colour_agree() {
    // `SGR 31` and `SGR 38;5;1` name the same colour. If they resolved through
    // different tables they could drift apart; both must reach one palette.
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(1, 4, b"\x1b[31mA\x1b[38;5;1mA");
    let frame = render(&renderer, &snap);

    let ansi_form = cell_color(&frame, 0, 0);
    let indexed_form = cell_color(&frame, 0, 1);
    assert!(
        colors_match(ansi_form, indexed_form),
        "SGR 31 drew {ansi_form:?} but SGR 38;5;1 drew {indexed_form:?} — \
         the two forms of ANSI red must resolve through one palette"
    );
}

#[test]
fn truecolor_background_paints_the_whole_cell_and_keeps_glyph_foreground() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let background = [12, 98, 201];
    let foreground = [241, 207, 33];
    let snap = snapshot(1, 1, b"\x1b[38;2;241;207;33;48;2;12;98;201mA");
    let frame = render(&renderer, &snap);

    let mut background_pixels = 0;
    let mut foreground_pixels = 0;
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let pixel = frame.pixel(x, y);
            if is_background(pixel, Some(background)) {
                background_pixels += 1;
            } else if colors_match([pixel[0], pixel[1], pixel[2]], foreground) {
                foreground_pixels += 1;
            } else {
                panic!("cell pixel ({x},{y}) is neither background nor foreground: {pixel:?}");
            }
        }
    }

    assert!(
        background_pixels > 0,
        "the cell rectangle must paint pixels outside glyph strokes"
    );
    assert!(
        foreground_pixels > 0,
        "the glyph must remain visible over its background"
    );
    // The glyph starts three pixels below the cell top and never reaches the
    // final row, so these corners must be painted by the full-cell rectangle.
    for (x, y) in [
        (0, 0),
        (CELL_WIDTH - 1, 0),
        (0, CELL_HEIGHT - 1),
        (CELL_WIDTH - 1, CELL_HEIGHT - 1),
    ] {
        assert!(
            is_background(frame.pixel(x, y), Some(background)),
            "cell corner ({x},{y}) was not painted by the background rectangle"
        );
    }
}

#[test]
fn background_on_a_space_paints_even_without_glyph_strokes() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let background = [73, 18, 146];
    let snap = snapshot(1, 1, b"\x1b[48;2;73;18;146m ");
    let frame = render(&renderer, &snap);

    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            assert!(
                is_background(frame.pixel(x, y), Some(background)),
                "background-painted space leaked clear colour at ({x},{y})"
            );
        }
    }
}

#[test]
fn indexed_background_matches_its_truecolor_palette_entry() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let truecolor = snapshot(1, 1, b"\x1b[48;2;255;0;0mA");
    let indexed = snapshot(1, 1, b"\x1b[48;5;196mA");

    let truecolor_frame = render(&renderer, &truecolor);
    let indexed_frame = render(&renderer, &indexed);
    assert_eq!(
        truecolor_frame.rgba, indexed_frame.rgba,
        "truecolor red and indexed palette entry 196 must render identically"
    );
    assert_eq!(DEFAULT_PALETTE[196], [255, 0, 0]);
}

#[test]
fn no_background_keeps_the_existing_pixel_exact_appearance() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let plain = render(&renderer, &snapshot(1, 1, b"A"));
    let reset_background = render(&renderer, &snapshot(1, 1, b"\x1b[49mA"));

    assert_eq!(
        plain.rgba, reset_background.rgba,
        "default background must not alter the historical glyph frame"
    );
}

#[test]
fn a_dark_coloured_cell_is_still_seen_as_lit() {
    // This is the case the old brightness gate got wrong: ANSI black is a
    // legitimate foreground whose channels are all far below the old `< 48`
    // threshold, so a threshold-based oracle would call the cell blank and
    // agree with a renderer that dropped it. `is_background` now compares to
    // the clear colour, so the cell reads as lit and the colour is checked.
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(1, 4, b"\x1b[30mA");
    let frame = render(&renderer, &snap);

    assert!(
        cell_is_lit(&frame, 0, 0),
        "an ANSI-black 'A' must count as lit — it is drawn, just dark"
    );
    let drawn = cell_color(&frame, 0, 0);
    assert!(
        colors_match(drawn, DEFAULT_ANSI_PALETTE[0]),
        "SGR 30 should draw palette black {:?}, drew {drawn:?}",
        DEFAULT_ANSI_PALETTE[0]
    );
    // The old gate would have mistaken this for background; the new one must not.
    assert!(drawn.iter().all(|channel| *channel < 48));
}

#[test]
fn colour_follows_the_cell_across_wide_characters_and_rows() {
    // Colour must be addressed per cell in the renderer's coordinate model,
    // not per character of a flattened string: a wide lead's continuation
    // column must not shift the colour of the glyph that follows it.
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    // 'a' default, then a red wide char (columns 1-2), then green 'b' at
    // display column 3.
    let snap = snapshot(2, 6, "a\x1b[31m日\x1b[32mb\r\n\x1b[34mc".as_bytes());
    let frame = render(&renderer, &snap);

    assert!(
        colors_match(cell_color(&frame, 0, 0), default_foreground_rgb()),
        "the unstyled 'a' must keep the default foreground"
    );
    assert!(
        colors_match(cell_color(&frame, 0, 1), DEFAULT_ANSI_PALETTE[1]),
        "the wide lead at column 1 must be red"
    );
    assert!(
        !cell_is_lit(&frame, 0, 2),
        "the wide continuation column must stay unlit"
    );
    assert!(
        colors_match(cell_color(&frame, 0, 3), DEFAULT_ANSI_PALETTE[2]),
        "'b' at display column 3 must be green — colour tracked the cell \
         across the continuation column"
    );
    assert!(
        colors_match(cell_color(&frame, 1, 0), DEFAULT_ANSI_PALETTE[4]),
        "'c' on row 1 must be blue"
    );
}

// ===========================================================================
// Defect tests: written the way a correct renderer requires. They are not
// weakened to pass; fixed behaviours are removed from the ignored set.
// ===========================================================================

#[test]
fn lowercase_distinct_from_uppercase() {
    // Regression guard: the bitmap font used to fold case via
    // `to_ascii_uppercase`, making 'a' and 'A' produce identical pixels.
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let lower = snapshot(1, 2, b"a");
    let upper = snapshot(1, 2, b"A");
    let lower_frame = render(&renderer, &lower);
    let upper_frame = render(&renderer, &upper);

    let a_lower = cell_pattern(&lower_frame, 0, 0);
    let a_upper = cell_pattern(&upper_frame, 0, 0);
    assert!(!a_lower.iter().all(|&lit| !lit), "'a' drew nothing");
    assert!(!a_upper.iter().all(|&lit| !lit), "'A' drew nothing");
    assert_ne!(
        a_lower, a_upper,
        "font does not distinguish 'a' from 'A' (case-folded glyph table)"
    );
}

#[test]
#[ignore = "known defect: every non-ASCII code point falls through to the '?' \
            arm, so '日' and '?' draw identically. Ignored rather than deleted \
            or weakened — this assertion is the executable specification of the \
            fix. `cargo test -- --ignored` confirms it still fails; remove this \
            attribute when a real font stack lands."]
fn non_ascii_glyph_is_not_the_question_mark() {
    // DEFECT (reported): every non-ASCII code point hits the `?` default arm in
    // `glyph_rows`, so '日' renders as '?'. A correct UTF-8 terminal must at
    // least draw a different glyph for a different character.
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let kanji = snapshot(1, 4, "日".as_bytes());
    let question = snapshot(1, 4, b"?");
    let kanji_frame = render(&renderer, &kanji);
    let question_frame = render(&renderer, &question);

    let kanji_pattern = cell_pattern(&kanji_frame, 0, 0);
    let question_pattern = cell_pattern(&question_frame, 0, 0);
    assert!(!kanji_pattern.iter().all(|&lit| !lit), "'日' drew nothing");
    assert!(
        !question_pattern.iter().all(|&lit| !lit),
        "'?' drew nothing"
    );
    assert_ne!(
        kanji_pattern, question_pattern,
        "non-ASCII '日' renders identically to '?' (font maps all non-ASCII to '?')"
    );
}

#[test]
fn utf8_cell_is_at_least_lit_so_the_pipeline_handles_wide_input() {
    // Unlike the glyph-correctness defect above, this only asserts the wide
    // input lights its lead cell at all — the pipeline must not silently drop
    // non-ASCII. (It draws '?' today, which still lights the cell.)
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let snap = snapshot(1, 4, "café".as_bytes());
    let frame = render(&renderer, &snap);
    // c, a, f are ASCII and lit; the final cell holds the non-ASCII 'é' lead.
    assert!(cell_is_lit(&frame, 0, 3), "non-ASCII cell drew nothing");
}

// ===========================================================================
// Sidebar rendering: the sidebar is a fixed-width column on the left; the
// terminal occupies the remaining columns to the right. These tests drive the
// real wgpu pipeline offscreen and assert on rendered pixels — the same
// approach as the core oracle above.
// ===========================================================================

/// Render a snapshot plus sidebar text at `(terminal_cols + SIDEBAR_COLS)` cell
/// columns wide — mirroring how the real app partitions the window.
fn render_with_sidebar(
    renderer: &OffscreenRenderer,
    snap: &TerminalSnapshot,
    sidebar: &[String],
) -> CapturedFrame {
    let total_cols = u32::from(snap.cols()) + SIDEBAR_COLS as u32;
    let width = total_cols * CELL_WIDTH;
    let height = u32::from(snap.rows()) * CELL_HEIGHT;
    renderer.capture(
        Some(snap),
        Some(sidebar),
        None,
        width,
        height,
        poc_metrics(),
    )
}

#[test]
fn sidebar_rows_appear_in_the_left_columns() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    // One selected session row: '>' marker, then label and detail.
    let sidebar = vec!["> SESSION-1 LOCAL".to_string()];
    let snap = snapshot(1, 20, b"");
    let frame = render_with_sidebar(&renderer, &snap, &sidebar);

    // The '>' selection marker at column 0 must be lit.
    assert!(
        cell_is_lit(&frame, 0, 0),
        "selection marker '>' at column 0 should be lit"
    );
    // Column 1 is the space separator — blank.
    assert!(
        !cell_is_lit(&frame, 0, 1),
        "space after marker at column 1 should be blank"
    );
    // 'S' of SESSION at column 2 must be lit.
    assert!(
        cell_is_lit(&frame, 0, 2),
        "'S' of SESSION at column 2 should be lit"
    );
    // The lit pattern at (0,0) must differ from a blank cell to prove a real
    // glyph was drawn, not noise.
    let blank = vec![false; (CELL_WIDTH * CELL_HEIGHT) as usize];
    assert_ne!(
        cell_pattern(&frame, 0, 0),
        blank,
        "selection marker cell has a non-blank glyph pattern"
    );
}

#[test]
fn terminal_content_does_not_overlap_sidebar_columns() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    // No sidebar text — the sidebar region stays entirely blank.
    let sidebar: Vec<String> = vec![];
    // Terminal with 'A' at column 0.
    let snap = snapshot(1, 10, b"A");
    let frame = render_with_sidebar(&renderer, &snap, &sidebar);

    let sc = SIDEBAR_COLS as u32;
    // The terminal's column 0 maps to window column SIDEBAR_COLS — the 'A'
    // must appear there, not inside the sidebar.
    assert!(
        cell_is_lit(&frame, 0, sc),
        "terminal 'A' should be lit at window column SIDEBAR_COLS ({sc})"
    );
    // Every sidebar column must be blank.
    for col in 0..sc {
        assert!(
            !cell_is_lit(&frame, 0, col),
            "sidebar column {col} is lit — terminal bled into the sidebar"
        );
    }
}

#[test]
fn sidebar_plus_terminal_columns_equal_window_columns() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let sidebar: Vec<String> = vec![];
    let term_cols = 10u16;
    // Fill every terminal column with 'A'.
    let snap = snapshot(1, term_cols, b"AAAAAAAAAA");
    let frame = render_with_sidebar(&renderer, &snap, &sidebar);

    let sc = SIDEBAR_COLS as u32;
    let total = sc + u32::from(term_cols);

    // Sidebar region: columns 0..SIDEBAR_COLS must be blank.
    for col in 0..sc {
        assert!(
            !cell_is_lit(&frame, 0, col),
            "sidebar column {col} should be blank"
        );
    }
    // Terminal region: columns SIDEBAR_COLS..total must be lit.
    for col in sc..total {
        assert!(
            cell_is_lit(&frame, 0, col),
            "terminal column {col} (window col {col}) should be lit"
        );
    }
}

#[test]
fn empty_state_message_is_drawn_in_the_sidebar() {
    let renderer = OffscreenRenderer::new().expect("offscreen renderer");
    let sidebar = vec!["NO SESSIONS".to_string()];
    let snap = snapshot(1, 10, b"");
    let frame = render_with_sidebar(&renderer, &snap, &sidebar);

    // 'N' at column 0 and 'O' at column 1 must both be lit.
    assert!(
        cell_is_lit(&frame, 0, 0),
        "'N' of NO SESSIONS should be lit"
    );
    assert!(
        cell_is_lit(&frame, 0, 1),
        "'O' of NO SESSIONS should be lit"
    );
    // The patterns must differ — they are different glyphs.
    assert_ne!(
        cell_pattern(&frame, 0, 0),
        cell_pattern(&frame, 0, 1),
        "'N' and 'O' rendered the same pattern"
    );
}
