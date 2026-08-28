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
//!   expected values (issue #107);
//! - the five issue-168 AA fixes to the default dark palette are pinned on
//!   the drawn pixels — each fixed slot draws its new value against a
//!   literal, and the drawn colour measurably clears WCAG AA (4.5:1)
//!   computed on the readback bytes against the readback clear colour;
//! - a wide (CJK/emoji) character lights its lead column with the replacement
//!   glyph, leaves its continuation column unlit, and the glyph after the
//!   pair lands at its display column — the width contract whose loss corrupts
//!   the rest of the line;
//! - a combining mark is drawn over its base cell without consuming a cell.
//! - the default palette affordance is drawn in permanent terminal-side
//!   chrome, rebinding `palette_open` changes the captured key glyph, and the
//!   explicit UI opt-out removes those pixels (issue #191);
//! - after the last session closes, recovery copy and its active create chord
//!   draw in the otherwise blank terminal area, with status chrome following
//!   the recovery rows instead of overwriting them (issue #201).
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
//! - `non_ascii_glyph_is_not_the_question_mark` guards the fixed fallback
//!   defect: unsupported Unicode such as `日` uses a visible replacement glyph,
//!   while Latin-1 Supplement and Box Drawing have built-in coverage.
//!
//! ## Skip policy (issue #144)
//!
//! When this machine has no GPU adapter at all, each test returns early after
//! printing `SKIP: [...]` to the REAL stderr (bypassing the harness's output
//! capture) — an adapter-less machine stays green while the output states
//! explicitly that rendered-frame evidence was NOT gathered. A skip is never
//! reported as gathered evidence. An adapter that EXISTS but fails to yield a
//! device or a frame is a real failure and stays red; only total adapter
//! absence skips. `NOREN_FRAME_ORACLE_ADAPTER=absent|device-fails` (see
//! `renderer_capture.rs`) forces either headless failure mode on
//! adapter-equipped machines so the skip behaviour itself is testable.

#[path = "../src/renderer_capture.rs"]
mod renderer_capture;

use std::io::Write;
use std::process::Command;

use noren_app::config::AppConfig;
use noren_app::theme::{DARK, HIGH_CONTRAST, LIGHT, Theme, contrast_ratio};
use noren_app::ui::{empty_workspace_recovery, palette_hint};
use noren_app::{
    GridGeometry, MAX_RENDER_COLS, MAX_RENDER_ROWS, POC_CELL_HEIGHT as CELL_HEIGHT,
    POC_CELL_WIDTH as CELL_WIDTH,
};
use noren_terminal::{TerminalSnapshot, TerminalState};
use renderer_capture::renderer_source::{CLEAR_COLOR, FrameChrome, SIDEBAR_COLS, Target};
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
///
/// Every pre-theme test renders through the default (`dark`) theme, exactly
/// what the binary drew before `[theme]` existed; theme-specific behaviour
/// has its own helpers and tests below.
fn render(renderer: &OffscreenRenderer, snapshot: &TerminalSnapshot) -> CapturedFrame {
    render_with_theme(renderer, &Theme::default(), snapshot)
}

/// Render a snapshot under an explicit theme at exactly its grid size.
fn render_with_theme(
    renderer: &OffscreenRenderer,
    theme: &Theme,
    snapshot: &TerminalSnapshot,
) -> CapturedFrame {
    let width = u32::from(snapshot.cols()) * CELL_WIDTH;
    let height = u32::from(snapshot.rows()) * CELL_HEIGHT;
    renderer.capture(
        Target::new(theme, width, height, poc_metrics()),
        Some(snapshot),
        None,
        None,
    )
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
/// [`DARK.foreground()`] so the "unstyled cells are unchanged" assertion
/// tracks the renderer rather than a literal copied here.
fn default_foreground_rgb() -> [u8; 3] {
    DARK.foreground()
        .map(|channel| (channel * 255.0).round() as u8)
}

/// State-driven blankness: a cell is blank when the row is absent, the column
/// is past the line end, or the cell is an ASCII space (the space glyph is the
/// all-zero row, so it draws nothing).
///
/// Reads `display_cells()` — one cell per display column, the renderer's
/// coordinate model. `display_lines()` encodes the same columns for wide
/// characters but gives a combining mark its own character index even though
/// the mark consumes no column (it lives inside the lead cell's text), so a
/// string index is not a display column once marks are present. Every cell of
/// a display row is retained here (only whole trailing rows are dropped), so
/// "past the line end" reduces to "the cell is blank" and absent rows stay
/// blank.
fn state_cell_blank(snapshot: &TerminalSnapshot, row: u32, col: u32) -> bool {
    match snapshot.display_cells().nth(row as usize) {
        None => true,
        Some(cells) => match cells.get(col as usize) {
            None => true,
            Some(cell) => cell.is_continuation() || cell.text() == " ",
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
// Gate 0: can wgpu initialise headlessly on this machine? Total adapter
// absence skips with the notice below (evidence NOT gathered); an adapter
// that exists but cannot yield a device is a real failure and stays red.
// ===========================================================================

/// Print the skip notice shared by every oracle test when no GPU adapter
/// exists — the same discipline `tests/zellij_live.rs` uses when Zellij is
/// absent, not a second convention for the same idea.
///
/// The notice is written straight to the process's stderr file descriptor so
/// it survives the test harness's output capture: an early-returning test
/// otherwise reads as a silent pass under default `cargo test` output, and a
/// skip must never be mistaken for gathered evidence.
fn report_skip(test: &str) {
    let notice = format!(
        "SKIP [{test}]: no GPU adapter is available (wgpu request_adapter failed: \
         AdapterUnavailable); rendered-frame evidence was NOT gathered. This is a \
         skip, not a pass."
    );
    // Write the process's inherited stderr handle directly. Opening
    // `/dev/stderr` can resolve to the controlling terminal instead of the
    // child's piped fd on macOS, which makes `Command::output` observe an
    // empty stream and defeats the visibility guard below. A direct `Write`
    // bypasses libtest's print-macro capture while preserving the actual fd.
    // One write, notice and newline together, also prevents parallel skips
    // from interleaving into concatenated lines under the pipe buffer.
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(format!("{notice}\n").as_bytes());
}

/// The offscreen renderer, or `None` after reporting a skip, when this machine
/// has no GPU adapter at all.
///
/// The two headless failures are kept deliberately distinct:
///
/// - `AdapterUnavailable` — wgpu enumerated no adapter at all: the machine
///   cannot render and nothing the renderer did is wrong, so the test SKIPS
///   after printing the notice above. A skip, never a pass.
/// - `DeviceUnavailable` — an adapter exists but the device request failed:
///   a real failure of the render path's environment, which must stay red,
///   so this panics instead of skipping.
///
/// Failures past a working device (shader compile, pipeline build, readback)
/// already panic inside `capture`, so no frame-producing failure can be
/// conflated with a skip either.
fn renderer_or_skip(test: &str) -> Option<OffscreenRenderer> {
    match OffscreenRenderer::new() {
        Ok(renderer) => Some(renderer),
        Err(CaptureError::AdapterUnavailable) => {
            report_skip(test);
            None
        }
        Err(CaptureError::DeviceUnavailable) => {
            panic!("offscreen=blocked: adapter present but device request failed");
        }
    }
}

#[test]
fn offscreen_wgpu_pipeline_initialises() {
    match OffscreenRenderer::new() {
        Ok(_) => { /* offscreen=ok */ }
        Err(CaptureError::AdapterUnavailable) => {
            report_skip("offscreen_wgpu_pipeline_initialises");
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
    let Some(renderer) = renderer_or_skip("blank_screen_has_no_lit_pixels") else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("an_ascii_cell_is_lit_and_its_neighbours_are_blank")
    else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("distinct_ascii_glyphs_have_distinct_lit_patterns")
    else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("drawn_grid_dimensions_match_state_dimensions") else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("state_and_render_agree_across_the_fr005_fixtures")
    else {
        return;
    };

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
    let Some(renderer) = renderer_or_skip("glyphs_stay_inside_the_render_clamp_grid") else {
        return;
    };

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
    let Some(renderer) =
        renderer_or_skip("cells_with_different_sgr_foregrounds_render_different_colours")
    else {
        return;
    };
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
        colors_match(red_cell, DARK.ansi()[1]),
        "SGR 31 should draw palette red {:?}, drew {red_cell:?}",
        DARK.ansi()[1]
    );
    assert!(
        colors_match(green_cell, DARK.ansi()[2]),
        "SGR 32 should draw palette green {:?}, drew {green_cell:?}",
        DARK.ansi()[2]
    );
}

#[test]
fn a_cell_with_no_sgr_colour_keeps_the_default_appearance() {
    // The compatibility half of issue #107: wiring colour must not change how
    // an unstyled prompt looks.
    let Some(renderer) = renderer_or_skip("a_cell_with_no_sgr_colour_keeps_the_default_appearance")
    else {
        return;
    };
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
    let Some(renderer) =
        renderer_or_skip("truecolor_and_256_colour_both_resolve_to_their_expected_pixels")
    else {
        return;
    };

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
        colors_match(drawn_indexed, DARK.indexed_palette()[196]),
        "256-colour index 196 drew {drawn_indexed:?}, expected {:?}",
        DARK.indexed_palette()[196]
    );
    assert_eq!(DARK.indexed_palette()[196], [255, 0, 0]);

    // A grayscale-ramp index resolves too, and differs from both above.
    let gray = snapshot(1, 4, b"\x1b[38;5;244mA");
    let gray_frame = render(&renderer, &gray);
    let drawn_gray = cell_color(&gray_frame, 0, 0);
    assert!(
        colors_match(drawn_gray, DARK.indexed_palette()[244]),
        "256-colour index 244 drew {drawn_gray:?}, expected {:?}",
        DARK.indexed_palette()[244]
    );
    assert!(!colors_match(drawn_gray, drawn_indexed));
}

#[test]
fn the_16_colour_and_256_colour_forms_of_one_colour_agree() {
    // `SGR 31` and `SGR 38;5;1` name the same colour. If they resolved through
    // different tables they could drift apart; both must reach one palette.
    let Some(renderer) = renderer_or_skip("the_16_colour_and_256_colour_forms_of_one_colour_agree")
    else {
        return;
    };
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
    let Some(renderer) =
        renderer_or_skip("truecolor_background_paints_the_whole_cell_and_keeps_glyph_foreground")
    else {
        return;
    };
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
    let Some(renderer) =
        renderer_or_skip("background_on_a_space_paints_even_without_glyph_strokes")
    else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("indexed_background_matches_its_truecolor_palette_entry")
    else {
        return;
    };
    let truecolor = snapshot(1, 1, b"\x1b[48;2;255;0;0mA");
    let indexed = snapshot(1, 1, b"\x1b[48;5;196mA");

    let truecolor_frame = render(&renderer, &truecolor);
    let indexed_frame = render(&renderer, &indexed);
    assert_eq!(
        truecolor_frame.rgba, indexed_frame.rgba,
        "truecolor red and indexed palette entry 196 must render identically"
    );
    assert_eq!(DARK.indexed_palette()[196], [255, 0, 0]);
}

#[test]
fn no_background_keeps_the_existing_pixel_exact_appearance() {
    let Some(renderer) =
        renderer_or_skip("no_background_keeps_the_existing_pixel_exact_appearance")
    else {
        return;
    };
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
    // legitimate foreground whose channels sat far below the old `< 48`
    // threshold (until issue #168 lifted black to grey 121 for AA), so a
    // threshold-based oracle would call the cell blank and agree with a
    // renderer that dropped it. `is_background` compares to the clear
    // colour, so the cell reads as lit and the colour is checked — against
    // the fixed palette value, pinned here as a literal so the issue-168
    // fix is visible at the pixel level too.
    let Some(renderer) = renderer_or_skip("a_dark_coloured_cell_is_still_seen_as_lit") else {
        return;
    };
    let snap = snapshot(1, 4, b"\x1b[30mA");
    let frame = render(&renderer, &snap);

    assert!(
        cell_is_lit(&frame, 0, 0),
        "an ANSI-black 'A' must count as lit — it is drawn, just dark"
    );
    let drawn = cell_color(&frame, 0, 0);
    assert!(
        colors_match(drawn, DARK.ansi()[0]),
        "SGR 30 should draw palette black {:?}, drew {drawn:?}",
        DARK.ansi()[0]
    );
    // The issue-168 value: the smallest achromatic grey clearing WCAG AA
    // (4.53:1) on the dark background — no longer below any brightness gate.
    assert_eq!(
        drawn,
        [121, 121, 121],
        "palette black must draw the AA-fixed grey 121, not the pre-168 near-zero"
    );
}

/// The issue-168 fix must reach drawing, not just the palette table: each
/// of the five lifted dark entries is drawn as its fixed value — asserted
/// against literals here, so reverting the palette to the pre-fix xterm
/// values fails at the pixel level — and each drawn colour measurably
/// clears WCAG AA *as pixels*, the ratio computed on the readback bytes
/// against the readback clear colour. This is the pixel-level half of the
/// fix's evidence; `tests/theme.rs` holds the palette-level half.
#[test]
fn the_issue_168_aa_fixes_reach_the_drawn_pixels() {
    let Some(renderer) = renderer_or_skip("the_issue_168_aa_fixes_reach_the_drawn_pixels") else {
        return;
    };
    // Row 0: the four normal-brightness fixes; row 1: the bright-blue fix
    // (SGR 94). Distinct glyphs so each cell is unambiguously lit.
    let snap = snapshot(2, 4, b"\x1b[30ma\x1b[31mb\x1b[34mc\x1b[35md\r\n\x1b[94me");
    let frame = render(&renderer, &snap);

    let fixed: [(u32, u32, &str, [u8; 3]); 5] = [
        (0, 0, "SGR 30 black", [121, 121, 121]),
        (0, 1, "SGR 31 red", [243, 0, 0]),
        (0, 2, "SGR 34 blue", [0, 113, 255]),
        (0, 3, "SGR 35 magenta", [213, 0, 213]),
        (1, 0, "SGR 94 bright blue", [100, 100, 255]),
    ];
    let ground = clear_rgb();
    for (row, col, label, value) in fixed {
        assert!(
            cell_is_lit(&frame, row, col),
            "{label}: the fixed glyph drew nothing"
        );
        let drawn = cell_color(&frame, row, col);
        assert_eq!(
            drawn, value,
            "{label} must draw the issue-168 fixed value — the fix did not \
             reach the pixels"
        );
        // The readable-text claim, measured on the pixels themselves.
        let ratio = contrast_ratio(drawn, ground);
        assert!(
            ratio >= 4.5,
            "{label} drew {drawn:?}, only {ratio:.2}:1 against the drawn \
             background {ground:?}"
        );
    }
}

#[test]
fn colour_follows_the_cell_across_wide_characters_and_rows() {
    // Colour must be addressed per cell in the renderer's coordinate model,
    // not per character of a flattened string: a wide lead's continuation
    // column must not shift the colour of the glyph that follows it.
    let Some(renderer) =
        renderer_or_skip("colour_follows_the_cell_across_wide_characters_and_rows")
    else {
        return;
    };
    // 'a' default, then a red wide char (columns 1-2), then green 'b' at
    // display column 3.
    let snap = snapshot(2, 6, "a\x1b[31m日\x1b[32mb\r\n\x1b[34mc".as_bytes());
    let frame = render(&renderer, &snap);

    assert!(
        colors_match(cell_color(&frame, 0, 0), default_foreground_rgb()),
        "the unstyled 'a' must keep the default foreground"
    );
    assert!(
        colors_match(cell_color(&frame, 0, 1), DARK.ansi()[1]),
        "the wide lead at column 1 must be red"
    );
    assert!(
        !cell_is_lit(&frame, 0, 2),
        "the wide continuation column must stay unlit"
    );
    assert!(
        colors_match(cell_color(&frame, 0, 3), DARK.ansi()[2]),
        "'b' at display column 3 must be green — colour tracked the cell \
         across the continuation column"
    );
    assert!(
        colors_match(cell_color(&frame, 1, 0), DARK.ansi()[4]),
        "'c' on row 1 must be blue"
    );
}

// ===========================================================================
// Wide-character width contract (Milestone 6): the terminal state core has
// modelled CJK/emoji display width since PR #53 (a width-two character is a
// lead cell plus a continuation cell), and the renderer honours that model by
// reserving the continuation column without drawing it. These tests drive the
// whole chain — `feed_bytes` → state → real GPU pixels — because "the state
// models it" and "the renderer draws it" are different claims, and the pinned
// properties are exactly the ones whose loss corrupts every following column
// on the line. The bitmap font carries no CJK glyphs (issue #141): the pinned
// failure mode is a visible replacement glyph at correct columns, which is
// the deliberate scope of this slice — width handled correctly, glyphs not.
// ===========================================================================

/// `日本語` end to end: each character occupies two cells in the state, the
/// lead column draws the visible replacement glyph, the continuation column
/// draws nothing, and the whole grid still agrees cell-for-cell between state
/// and render.
///
/// The three leads drawing identical pixels is today's truth pinned on
/// purpose — the bitmap font cannot distinguish CJK code points, so uniform
/// replacement is the contract until a real font stack lands (its own
/// milestone decision). Adding CJK glyphs means updating this assertion, not
/// deleting the test.
#[test]
fn cjk_text_occupies_two_cells_per_character_and_fails_visibly_not_corruptingly() {
    let Some(renderer) = renderer_or_skip(
        "cjk_text_occupies_two_cells_per_character_and_fails_visibly_not_corruptingly",
    ) else {
        return;
    };
    let mut state = TerminalState::new(1, 8).expect("valid test terminal");
    state.feed_bytes("日本語".as_bytes());
    let snap = state.snapshot();

    // State half of the claim: three width-two leads at columns 0/2/4, a
    // continuation cell after each, and the cursor six columns on.
    assert_eq!(
        (state.cursor().row(), state.cursor().column()),
        (0, 6),
        "three wide characters must advance the cursor six columns"
    );
    let cells: Vec<_> = snap
        .display_cells()
        .next()
        .expect("one display row")
        .to_vec();
    for (col, expected) in
        [(0usize, "日"), (2, "本"), (4, "語")].map(|(col, text)| (col, Some(text)))
    {
        let cell = &cells[col];
        assert_eq!(cell.text(), expected.unwrap(), "lead at column {col}");
        assert_eq!(cell.width(), 2, "lead at column {col}");
        assert!(!cell.is_continuation());
        assert!(
            cells[col + 1].is_continuation(),
            "column {} must be the continuation of the wide lead before it",
            col + 1
        );
    }

    let frame = render(&renderer, &snap);
    assert_cells_agree(&frame, &snap);

    // The lead columns are lit, all with the same glyph (uniform replacement
    // is the pinned boundary), and that glyph is not the '?' glyph.
    let question = render(&renderer, &snapshot(1, 8, b"?"));
    for col in [0u32, 2, 4] {
        assert!(
            cell_is_lit(&frame, 0, col),
            "wide lead at column {col} drew nothing — the failure must be visible"
        );
        assert_eq!(
            cell_pattern(&frame, 0, col),
            cell_pattern(&frame, 0, 0),
            "CJK leads must draw the uniform replacement glyph (pinned: no CJK coverage)"
        );
        assert_ne!(
            cell_pattern(&frame, 0, col),
            cell_pattern(&question, 0, 0),
            "the replacement glyph must not impersonate a typed '?'"
        );
    }
    // Continuation columns own their column without drawing, and everything
    // after the three wide pairs is untouched.
    for col in [1u32, 3, 5, 6, 7] {
        assert!(!cell_is_lit(&frame, 0, col), "column {col} must stay unlit");
    }
}

/// The width contract that protects the rest of the line: a wide character
/// followed by ASCII must place the ASCII at display column 2 (the wide pair
/// is columns 0–1). A renderer that lets the wide character claim one cell
/// draws the follower at column 1 and every subsequent column is wrong.
#[test]
fn wide_character_then_ascii_keeps_the_ascii_at_its_display_column() {
    let Some(renderer) =
        renderer_or_skip("wide_character_then_ascii_keeps_the_ascii_at_its_display_column")
    else {
        return;
    };
    let b_reference = render(&renderer, &snapshot(1, 4, b"b"));
    for (label, bytes) in [
        ("CJK 日", "日b".as_bytes()),
        ("wide emoji 😀", "😀b".as_bytes()),
    ] {
        let snap = snapshot(1, 4, bytes);
        let frame = render(&renderer, &snap);
        assert!(cell_is_lit(&frame, 0, 0), "{label}: wide lead drew nothing");
        assert!(
            !cell_is_lit(&frame, 0, 1),
            "{label}: the continuation column must own its column without drawing"
        );
        assert_eq!(
            cell_pattern(&frame, 0, 2),
            cell_pattern(&b_reference, 0, 0),
            "{label}: the ASCII after a wide character must land at display column 2"
        );
        assert!(
            !cell_is_lit(&frame, 0, 3),
            "{label}: column 3 must stay unlit"
        );
        assert_cells_agree(&frame, &snap);
    }
}

/// A combining mark must not consume a cell: `e` + U+0301 + `f` keeps `f` at
/// column 1 with its ordinary glyph, while the mark itself is drawn over the
/// `e` (visible, not dropped) — both compared against a plain `ef` render.
#[test]
fn combining_marks_consume_no_cell_through_the_pipeline() {
    let Some(renderer) = renderer_or_skip("combining_marks_consume_no_cell_through_the_pipeline")
    else {
        return;
    };
    let marked = snapshot(1, 4, "e\u{0301}f".as_bytes());
    let plain = snapshot(1, 4, b"ef");

    // State half: the mark lives inside column 0's cell (width unchanged),
    // and `f` stays at column 1.
    let cells: Vec<_> = marked
        .display_cells()
        .next()
        .expect("one display row")
        .to_vec();
    assert_eq!(cells[0].text(), "e\u{0301}");
    assert_eq!(cells[0].width(), 1);
    assert_eq!(cells[1].text(), "f");

    let marked_frame = render(&renderer, &marked);
    let plain_frame = render(&renderer, &plain);
    assert!(
        cell_is_lit(&marked_frame, 0, 0),
        "'e' with its attached mark drew nothing"
    );
    assert_ne!(
        cell_pattern(&marked_frame, 0, 0),
        cell_pattern(&plain_frame, 0, 0),
        "the combining mark drew nothing — it must fail visibly, not vanish"
    );
    assert_eq!(
        cell_pattern(&marked_frame, 0, 1),
        cell_pattern(&plain_frame, 0, 1),
        "'f' must land at column 1 exactly as without the mark — a combining \
         mark must not consume a cell"
    );
    assert_cells_agree(&marked_frame, &marked);
    for col in [2u32, 3] {
        assert!(
            !cell_is_lit(&marked_frame, 0, col),
            "column {col} must stay unlit"
        );
    }
}

// ===========================================================================
// Themes (Milestone 6 foundation): a `[theme]` selection must change what is
// drawn, and the absence of a selection must change nothing. The pipeline is
// the real one — same shader, same vertex generation, same clear path — with
// only the theme differing between captures.
// ===========================================================================

/// The defaults-preservation contract, at the pixel level: with no `[theme]`
/// section the configuration resolves dark, and a frame captured through that
/// resolution is byte-identical to one captured through the explicit dark
/// theme. Together with the pre-theme tests above (which still pin the
/// historical default foreground `[204, 235, 209]`, the dark clear colour,
/// and the dark palette entries against their literals), this proves the
/// default rendering did not move when themes landed.
#[test]
fn theme_absent_config_renders_byte_identically_to_the_explicit_dark_theme() {
    let Some(renderer) =
        renderer_or_skip("theme_absent_config_renders_byte_identically_to_the_explicit_dark_theme")
    else {
        return;
    };
    let snap = snapshot(2, 6, "a\x1b[31m日\x1b[32mb\r\n\x1b[34mc".as_bytes());

    // The real configuration path: no [theme] section resolves the default.
    let unthemed = noren_app::config::AppConfig::parse("# no theme section\n")
        .expect("a theme-less file parses")
        .theme()
        .palette();
    assert_eq!(unthemed, Theme::default());

    let via_config = render_with_theme(&renderer, &unthemed, &snap);
    let via_dark = render_with_theme(&renderer, &Theme::default(), &snap);
    assert_eq!(
        via_config.rgba, via_dark.rgba,
        "a missing [theme] section must render exactly the dark theme's bytes"
    );

    // And those bytes carry the pre-theme appearance: the unstyled 'a' is the
    // historical default foreground and the untouched background is the
    // historical clear colour (y = 1 sits above the glyph's top inset).
    assert!(
        colors_match(cell_color(&via_dark, 0, 0), [204, 235, 209]),
        "the dark default's unstyled foreground moved"
    );
    assert!(
        is_clear(via_dark.pixel(8, 1)),
        "the dark default's clear colour moved"
    );
}

/// The single lit colour inside a cell, measured over a themed ground.
///
/// The shared [`cell_color`] skips pixels matching the *dark* clear colour,
/// which under a light theme would skip nothing (or everything). This
/// variant takes the theme's own background as the ground predicate, so a
/// themed frame's glyph colour can be read the same way.
fn cell_color_over(frame: &CapturedFrame, ground: [u8; 3], row: u32, col: u32) -> [u8; 3] {
    let mut colors: Vec<[u8; 3]> = Vec::new();
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let pixel = frame.pixel(col * CELL_WIDTH + x, row * CELL_HEIGHT + y);
            let rgb = [pixel[0], pixel[1], pixel[2]];
            if colors_match(rgb, ground) {
                continue;
            }
            if !colors.iter().any(|seen| colors_match(*seen, rgb)) {
                colors.push(rgb);
            }
        }
    }
    assert_eq!(
        colors.len(),
        1,
        "cell ({row},{col}) should hold exactly one colour over the ground, \
         found {colors:?}"
    );
    colors[0]
}

/// The theme-reachability headline: a cell drawn with a given SGR colour must
/// produce **different pixels** under `light` than under `dark`, and each must
/// match its own theme's palette. The chain driven here is the real one —
/// `AppConfig::parse` of actual `[theme]` TOML → the theme's palette → the
/// shipped vertex/pipeline/clear path — so a theme that parses but never
/// reaches drawing cannot pass this test (the mutation that ignores the
/// setting fails here).
#[test]
fn a_configured_theme_changes_what_the_renderer_draws() {
    let Some(renderer) = renderer_or_skip("a_configured_theme_changes_what_the_renderer_draws")
    else {
        return;
    };
    // red 'b', green 'c', unstyled 'a', red-background 'd' (default fg), and
    // two empty columns whose pixels show the theme's clear colour.
    let snap = snapshot(1, 6, b"\x1b[31mb\x1b[32mc\x1b[0ma\x1b[41md");

    let theme_for = |text: &str| {
        noren_app::config::AppConfig::parse(text)
            .unwrap_or_else(|error| panic!("{text:?} must parse: {error}"))
            .theme()
            .palette()
    };
    let dark = theme_for("[theme]\nname = \"dark\"\n");
    let light = theme_for("[theme]\nname = \"light\"\n");
    assert_eq!(dark, DARK);
    assert_eq!(light, LIGHT);

    let dark_frame = render_with_theme(&renderer, &dark, &snap);
    let light_frame = render_with_theme(&renderer, &light, &snap);
    assert_ne!(
        dark_frame.rgba, light_frame.rgba,
        "the two themes must produce different pixels for the same snapshot"
    );

    // SGR 31: dark draws xterm red, light draws its darkened red; different.
    let dark_red = cell_color_over(&dark_frame, dark.background_u8(), 0, 0);
    let light_red = cell_color_over(&light_frame, light.background_u8(), 0, 0);
    assert!(
        colors_match(dark_red, DARK.ansi()[1]),
        "dark SGR 31 drew {dark_red:?}, expected {:?}",
        DARK.ansi()[1]
    );
    assert!(
        colors_match(light_red, LIGHT.ansi()[1]),
        "light SGR 31 drew {light_red:?}, expected {:?}",
        LIGHT.ansi()[1]
    );
    assert!(
        !colors_match(dark_red, light_red),
        "SGR 31 must draw different pixels under the two themes"
    );

    // SGR 32 likewise.
    let dark_green = cell_color_over(&dark_frame, dark.background_u8(), 0, 1);
    let light_green = cell_color_over(&light_frame, light.background_u8(), 0, 1);
    assert!(colors_match(dark_green, DARK.ansi()[2]));
    assert!(colors_match(light_green, LIGHT.ansi()[2]));
    assert!(!colors_match(dark_green, light_green));

    // Unstyled text: the theme's default foreground, different per theme.
    let dark_plain = cell_color_over(&dark_frame, dark.background_u8(), 0, 2);
    let light_plain = cell_color_over(&light_frame, light.background_u8(), 0, 2);
    assert!(colors_match(dark_plain, DARK.foreground_u8()));
    assert!(colors_match(light_plain, LIGHT.foreground_u8()));
    assert!(
        !colors_match(dark_plain, light_plain),
        "unstyled text must change with the theme"
    );

    // An explicit SGR 41 background paints its theme's palette entry: the
    // cell corner is glyph-free, so it is pure background.
    let dark_bg = [
        dark_frame.pixel(3 * CELL_WIDTH, 0)[0],
        dark_frame.pixel(3 * CELL_WIDTH, 0)[1],
        dark_frame.pixel(3 * CELL_WIDTH, 0)[2],
    ];
    let light_bg = [
        light_frame.pixel(3 * CELL_WIDTH, 0)[0],
        light_frame.pixel(3 * CELL_WIDTH, 0)[1],
        light_frame.pixel(3 * CELL_WIDTH, 0)[2],
    ];
    assert!(
        colors_match(dark_bg, DARK.ansi()[1]),
        "dark SGR 41 corner drew {dark_bg:?}, expected {:?}",
        DARK.ansi()[1]
    );
    assert!(
        colors_match(light_bg, LIGHT.ansi()[1]),
        "light SGR 41 corner drew {light_bg:?}, expected {:?}",
        LIGHT.ansi()[1]
    );

    // The untouched ground is each theme's clear colour (empty column 4).
    let dark_clear = dark_frame.pixel(4 * CELL_WIDTH, 0);
    let light_clear = light_frame.pixel(4 * CELL_WIDTH, 0);
    assert!(is_clear(dark_clear), "dark clear colour moved");
    assert!(
        colors_match(
            [light_clear[0], light_clear[1], light_clear[2]],
            LIGHT.background_u8()
        ),
        "light clear pixel {light_clear:?} is not the light background {:?}",
        LIGHT.background_u8()
    );
}

/// High-contrast draws its own pastel palette on pure black — the theme whose
/// measured minimum (7.84:1) clears WCAG AAA — and its pixels differ from
/// both other themes'.
#[test]
fn high_contrast_theme_draws_its_own_verified_palette() {
    let Some(renderer) = renderer_or_skip("high_contrast_theme_draws_its_own_verified_palette")
    else {
        return;
    };
    let snap = snapshot(1, 6, b"\x1b[31mb\x1b[32mc\x1b[0ma\x1b[41md");

    let hc = noren_app::config::AppConfig::parse("[theme]\nname = \"high-contrast\"\n")
        .expect("high-contrast is a valid theme name")
        .theme()
        .palette();
    assert_eq!(hc, HIGH_CONTRAST);

    let hc_frame = render_with_theme(&renderer, &hc, &snap);
    let dark_frame = render_with_theme(&renderer, &DARK, &snap);
    let light_frame = render_with_theme(&renderer, &LIGHT, &snap);
    assert_ne!(hc_frame.rgba, dark_frame.rgba);
    assert_ne!(hc_frame.rgba, light_frame.rgba);

    // SGR 31 is the pastel red; unstyled text is pure white on pure black.
    assert!(
        colors_match(
            cell_color_over(&hc_frame, hc.background_u8(), 0, 0),
            HIGH_CONTRAST.ansi()[1]
        ),
        "high-contrast SGR 31 must draw {:?}",
        HIGH_CONTRAST.ansi()[1]
    );
    assert!(
        colors_match(
            cell_color_over(&hc_frame, hc.background_u8(), 0, 2),
            HIGH_CONTRAST.foreground_u8()
        ),
        "high-contrast unstyled text must be pure white"
    );
    let clear = hc_frame.pixel(4 * CELL_WIDTH, 0);
    assert_eq!(
        [clear[0], clear[1], clear[2]],
        [0, 0, 0],
        "high-contrast clear must be pure black"
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
    let Some(renderer) = renderer_or_skip("lowercase_distinct_from_uppercase") else {
        return;
    };
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
fn non_ascii_glyph_is_not_the_question_mark() {
    // Full CJK is intentionally outside this fixed bitmap font. Unsupported
    // Unicode uses a visible replacement glyph, which must not impersonate a
    // literal question mark typed by the terminal application.
    let Some(renderer) = renderer_or_skip("non_ascii_glyph_is_not_the_question_mark") else {
        return;
    };
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
    let Some(renderer) =
        renderer_or_skip("utf8_cell_is_at_least_lit_so_the_pipeline_handles_wide_input")
    else {
        return;
    };
    let snap = snapshot(1, 4, "café".as_bytes());
    let frame = render(&renderer, &snap);
    // c, a, f are ASCII and lit; the final cell holds the non-ASCII 'é' lead.
    assert!(cell_is_lit(&frame, 0, 3), "non-ASCII cell drew nothing");
}

// ===========================================================================
// Palette discoverability (issue #191): the configured opener is permanent
// terminal-side chrome. These assertions parse real AppConfig, derive the
// label from its active KeymapConfig, run the production chrome vertex path,
// and inspect read-back pixels. A string-only assertion cannot satisfy this
// contract: deleting the draw path must make these tests fail.
// ===========================================================================

/// Render only the palette affordance, with a blank 16-column sidebar and
/// enough terminal-side width for every ordinary configured chord.
fn render_palette_hint(
    renderer: &OffscreenRenderer,
    config: &AppConfig,
    status: Option<&str>,
) -> CapturedFrame {
    const TERMINAL_COLS: u32 = 48;
    let hint = palette_hint(config.keys(), config.ui());
    let empty_sidebar: &[String] = &[];
    let chrome = FrameChrome::new(Some(empty_sidebar), status).with_palette_hint(hint.as_deref());
    renderer.capture_chrome(
        Target::new(
            &config.theme().palette(),
            (SIDEBAR_COLS as u32 + TERMINAL_COLS) * CELL_WIDTH,
            CELL_HEIGHT,
            poc_metrics(),
        ),
        None,
        chrome,
    )
}

#[test]
fn palette_hint_default_and_rebind_are_drawn_in_frame_pixels() {
    let Some(renderer) =
        renderer_or_skip("palette_hint_default_and_rebind_are_drawn_in_frame_pixels")
    else {
        return;
    };

    let default_frame = render_palette_hint(&renderer, &AppConfig::default(), Some("Noren ready"));
    let rebound = AppConfig::parse("[keys]\npalette_open = \"super+k\"\n")
        .expect("super+k is a valid palette rebind");
    let rebound_frame = render_palette_hint(&renderer, &rebound, Some("Noren ready"));

    // Both labels begin `Super+`; the configured key is the seventh glyph.
    // Compare that cell against independently rendered P/K glyph pixels so a
    // hard-coded default cannot satisfy the rebound half of the assertion.
    let configured_key_col = SIDEBAR_COLS as u32 + 6;
    let p_reference = render(&renderer, &snapshot(1, 1, b"P"));
    let k_reference = render(&renderer, &snapshot(1, 1, b"K"));
    let default_key_pixels = cell_pattern(&default_frame, 0, configured_key_col);
    let rebound_key_pixels = cell_pattern(&rebound_frame, 0, configured_key_col);

    assert_eq!(
        default_key_pixels,
        cell_pattern(&p_reference, 0, 0),
        "default chrome must draw the configured P key in real pixels"
    );
    assert_eq!(
        rebound_key_pixels,
        cell_pattern(&k_reference, 0, 0),
        "rebinding palette_open to Super+K must draw K, not stale P, in real pixels"
    );
    assert_ne!(
        default_key_pixels, rebound_key_pixels,
        "the rebound opener did not change the captured frame"
    );
    assert!(
        cell_is_lit(&default_frame, 0, SIDEBAR_COLS as u32),
        "the default-on palette hint drew no first glyph in the terminal-side status row"
    );
    assert!(
        !cell_is_lit(&default_frame, 0, 0),
        "the affordance must not consume the narrow sidebar's first column"
    );
}

#[test]
fn palette_hint_opt_out_removes_its_frame_pixels() {
    let Some(renderer) = renderer_or_skip("palette_hint_opt_out_removes_its_frame_pixels") else {
        return;
    };
    let hidden = AppConfig::parse("[ui]\nshow_palette_hint = false\n")
        .expect("the palette-hint opt-out is valid");
    let frame = render_palette_hint(&renderer, &hidden, None);

    assert!(
        frame
            .rgba
            .chunks_exact(4)
            .all(|pixel| is_clear([pixel[0], pixel[1], pixel[2], pixel[3]])),
        "show_palette_hint=false must remove every affordance pixel"
    );
}

/// Render the exact chrome the live app supplies after its last session is
/// closed: the compact sidebar locator, terminal-side recovery copy, and the
/// permanent palette affordance plus runtime status.
fn render_empty_workspace(renderer: &OffscreenRenderer, config: &AppConfig) -> CapturedFrame {
    const TERMINAL_COLS: u32 = 48;
    const FRAME_ROWS: u32 = 4;
    let sidebar = vec!["No sessions".to_owned()];
    let recovery = empty_workspace_recovery(config.keys());
    let hint = palette_hint(config.keys(), config.ui());
    let chrome = FrameChrome::new(Some(&sidebar), Some("Noren last session closed"))
        .with_palette_hint(hint.as_deref())
        .with_workspace_notice(Some(&recovery));
    renderer.capture_chrome(
        Target::new(
            &config.theme().palette(),
            (SIDEBAR_COLS as u32 + TERMINAL_COLS) * CELL_WIDTH,
            FRAME_ROWS * CELL_HEIGHT,
            poc_metrics(),
        ),
        None,
        chrome,
    )
}

#[test]
fn empty_workspace_recovery_action_is_drawn_in_terminal_frame_pixels() {
    let Some(renderer) =
        renderer_or_skip("empty_workspace_recovery_action_is_drawn_in_terminal_frame_pixels")
    else {
        return;
    };
    let default_frame = render_empty_workspace(&renderer, &AppConfig::default());
    let rebound = AppConfig::parse("[keys]\nsession_create = \"n\"\n")
        .expect("n is a valid create-session rebind");
    let rebound_frame = render_empty_workspace(&renderer, &rebound);

    let c_reference = render(&renderer, &snapshot(1, 1, b"C"));
    let n_reference = render(&renderer, &snapshot(1, 1, b"N"));
    let p_reference = render(&renderer, &snapshot(1, 1, b"P"));
    let action_key_col = SIDEBAR_COLS as u32 + 6; // `Press ` then the key.

    assert!(
        cell_is_lit(&default_frame, 0, 0),
        "the 16-column sidebar must retain its compact No sessions locator"
    );
    assert!(
        cell_is_lit(&default_frame, 0, SIDEBAR_COLS as u32),
        "the otherwise blank terminal area must draw No sessions yet"
    );
    assert_eq!(
        cell_pattern(&default_frame, 1, action_key_col),
        cell_pattern(&c_reference, 0, 0),
        "the default direct recovery action must draw C in real pixels"
    );
    assert_eq!(
        cell_pattern(&rebound_frame, 1, action_key_col),
        cell_pattern(&n_reference, 0, 0),
        "rebinding session_create must change the recovery action to N pixels"
    );
    assert_ne!(
        cell_pattern(&default_frame, 1, action_key_col),
        cell_pattern(&rebound_frame, 1, action_key_col),
        "the recovery action stayed stale after rebinding"
    );
    assert_eq!(
        cell_pattern(&default_frame, 2, SIDEBAR_COLS as u32 + 6),
        cell_pattern(&p_reference, 0, 0),
        "two recovery rows must be followed by the configured palette hint in status chrome"
    );
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
        Target::new(&Theme::default(), width, height, poc_metrics()),
        Some(snap),
        Some(sidebar),
        None,
    )
}

#[test]
fn sidebar_rows_appear_in_the_left_columns() {
    let Some(renderer) = renderer_or_skip("sidebar_rows_appear_in_the_left_columns") else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("terminal_content_does_not_overlap_sidebar_columns")
    else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("sidebar_plus_terminal_columns_equal_window_columns")
    else {
        return;
    };
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
    let Some(renderer) = renderer_or_skip("empty_state_message_is_drawn_in_the_sidebar") else {
        return;
    };
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

// ===========================================================================
// Skip-visibility guards (issue #144): a skip that prints nothing, or a
// device failure that skips, are both worse than a red test — they launder
// absence of evidence as evidence. These two tests force each headless
// failure mode through the REAL test binary (`NOREN_FRAME_ORACLE_ADAPTER`,
// see renderer_capture.rs) and pin the observable behaviour from the outside:
// adapter absence must skip with the notice, device failure must stay red
// with no skip notice. Deleting `report_skip`, silently returning on
// AdapterUnavailable, or folding DeviceUnavailable into the skip each fails
// one of these two.
// ===========================================================================

/// Re-run one adapter-dependent oracle test in this very binary under a
/// forced adapter mode, returning its exit status plus both captured streams:
/// the harness prints its failure report (panic text included) to stdout,
/// while the skip notice bypasses capture and lands on real stderr.
fn rerun_forced(test: &str, mode: &str) -> (bool, String, String) {
    let output = Command::new(std::env::current_exe().expect("locate the test binary"))
        .arg("--exact")
        .arg(test)
        .env("NOREN_FRAME_ORACLE_ADAPTER", mode)
        .output()
        .unwrap_or_else(|error| panic!("re-run `{test}` with adapter mode `{mode}`: {error}"));
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn forced_adapter_absence_skips_visibly_instead_of_failing() {
    let (success, _stdout, stderr) = rerun_forced("blank_screen_has_no_lit_pixels", "absent");
    assert!(
        success,
        "adapter absence must skip, not fail; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("SKIP [blank_screen_has_no_lit_pixels]"),
        "adapter-absent run printed no skip notice naming the test — a silent \
         skip launders absence of evidence as evidence; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("rendered-frame evidence was NOT gathered")
            && stderr.contains("This is a skip, not a pass"),
        "the skip notice lost its evidence-not-gathered wording; stderr:\n{stderr}"
    );
}

#[test]
fn forced_device_failure_stays_red_and_prints_no_skip() {
    let (success, stdout, stderr) = rerun_forced("blank_screen_has_no_lit_pixels", "device-fails");
    assert!(
        !success,
        "an adapter that exists but cannot yield a device is a real failure and \
         must stay red, not pass; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("adapter present but device request failed"),
        "the device-failure panic never reached the harness report; stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("SKIP [") && !stderr.contains("SKIP ["),
        "a device failure must not be reported as a skip — that would conflate \
         a broken render environment with absence of hardware; stdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
}
