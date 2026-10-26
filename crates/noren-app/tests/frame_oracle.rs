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
//! - scrollback offsets select the exact expected logical lines in captured
//!   pixels (`C/D/E`, `B/C/D`, then `A/B/C`), clamp at the oldest row, and
//!   retain a wide glyph's lead/continuation pair across the history seam;
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
//!   explicit UI opt-out removes those pixels (issue #191); the scrollback
//!   indicator likewise draws first and names the active return binding;
//! - after the last session closes, recovery copy and its active create chord
//!   draw in the otherwise blank terminal area, with status chrome following
//!   the recovery rows instead of overwriting them (issue #201);
//! - the cursor is drawn by default at the tracked position, inverse to its
//!   actual cell with a 4.5:1 safety fallback on arbitrary SGR backgrounds,
//!   spanning both columns of a wide character, hidden by DECTCEM (`CSI ?25l`)
//!   and restored by `CSI ?25h`, with focused and unfocused treatments that
//!   differ in pixels (issues #197/#200).
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
use noren_app::cursor::CursorShape;
use noren_app::session::{SessionKind, SessionRegistry, SessionStatus};
use noren_app::sidebar::{EntryKind, SessionLifecycle, SidebarEntry, SidebarView};
use noren_app::sidebar_text::{SidebarTextRow, visible_sidebar_text_rows_at_width};
use noren_app::theme::{DARK, HIGH_CONTRAST, LIGHT, Theme, contrast_ratio};
use noren_app::ui::{empty_workspace_recovery, palette_hint, scrollback_indicator};
use noren_app::{
    GridGeometry, MAX_RENDER_COLS, MAX_RENDER_ROWS, POC_CELL_HEIGHT as CELL_HEIGHT,
    POC_CELL_WIDTH as CELL_WIDTH,
};
use noren_terminal::{GridPoint, Selection, SelectionMode, TerminalSnapshot, TerminalState};
use renderer_capture::renderer_source::{
    CLEAR_COLOR, CursorStyle, FrameChrome, SIDEBAR_COLS, Target, Vertex, glyph_vertices_for_chrome,
    glyph_vertices_for_sidebar_rows,
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

/// Render through the production chrome path with an explicit history offset.
fn render_with_scroll_offset(
    renderer: &OffscreenRenderer,
    snapshot: &TerminalSnapshot,
    offset: usize,
) -> CapturedFrame {
    let width = u32::from(snapshot.cols()) * CELL_WIDTH;
    let height = u32::from(snapshot.rows()) * CELL_HEIGHT;
    renderer.capture_chrome(
        Target::new(&Theme::default(), width, height, poc_metrics()),
        Some(snapshot),
        FrameChrome::new(None, None).with_scroll_offset(offset),
    )
}

/// Render one app-owned selection through the live frame seam.
fn render_with_selection(
    renderer: &OffscreenRenderer,
    theme: &Theme,
    snapshot: &TerminalSnapshot,
    selection: &Selection,
) -> CapturedFrame {
    render_with_selection_and_scroll_offset(renderer, theme, snapshot, selection, 0)
}

/// Render a selection and history offset together through the production seam.
fn render_with_selection_and_scroll_offset(
    renderer: &OffscreenRenderer,
    theme: &Theme,
    snapshot: &TerminalSnapshot,
    selection: &Selection,
    offset: usize,
) -> CapturedFrame {
    let width = u32::from(snapshot.cols()) * CELL_WIDTH;
    let height = u32::from(snapshot.rows()) * CELL_HEIGHT;
    renderer.capture_chrome(
        Target::new(theme, width, height, poc_metrics()),
        Some(snapshot),
        FrameChrome::new(None, None)
            .with_scroll_offset(offset)
            .with_selection(Some(selection)),
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

/// Every RGBA pixel in cell `(row, col)`, including its background.
fn cell_pixels(frame: &CapturedFrame, row: u32, col: u32) -> Vec<[u8; 4]> {
    (0..CELL_HEIGHT)
        .flat_map(|y| {
            (0..CELL_WIDTH).map(move |x| frame.pixel(col * CELL_WIDTH + x, row * CELL_HEIGHT + y))
        })
        .collect()
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

/// Number of pixels inside one cell matching a concrete rendered colour.
fn cell_color_pixel_count(frame: &CapturedFrame, row: u32, col: u32, color: [u8; 3]) -> usize {
    let mut count = 0;
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let pixel = frame.pixel(col * CELL_WIDTH + x, row * CELL_HEIGHT + y);
            if colors_match([pixel[0], pixel[1], pixel[2]], color) {
                count += 1;
            }
        }
    }
    count
}

/// Cell coordinates whose captured pixels differ between two equal-size
/// frames, in row-major order. Comparing every pixel in every cell makes this
/// an exact positional oracle: moving the treatment to a neighbour, widening
/// it to a whole row, or dropping it changes the returned coordinate set.
fn changed_cells(
    before: &CapturedFrame,
    after: &CapturedFrame,
    rows: u16,
    cols: u16,
) -> Vec<(u32, u32)> {
    assert_eq!(before.width, after.width, "frame widths must agree");
    assert_eq!(
        before.rgba.len(),
        after.rgba.len(),
        "frame sizes must agree"
    );
    let mut changed = Vec::new();
    for row in 0..u32::from(rows) {
        for col in 0..u32::from(cols) {
            let differs = (row * CELL_HEIGHT..(row + 1) * CELL_HEIGHT).any(|y| {
                (col * CELL_WIDTH..(col + 1) * CELL_WIDTH)
                    .any(|x| before.pixel(x, y) != after.pixel(x, y))
            });
            if differs {
                changed.push((row, col));
            }
        }
    }
    changed
}

/// Full-cell selection-background rectangles emitted into the production
/// frame vertex stream, converted back to grid coordinates. This positional
/// oracle runs even when Metal capture is unavailable; the pixel tests below
/// remain the authoritative raster proof when an adapter exists.
fn selection_background_vertex_cells(
    snapshot: &TerminalSnapshot,
    selection: &Selection,
    theme: &Theme,
) -> Vec<(u32, u32)> {
    selection_background_vertex_cells_with_scroll_offset(snapshot, selection, theme, 0)
}

/// Selection-background cells emitted for an explicit history viewport.
fn selection_background_vertex_cells_with_scroll_offset(
    snapshot: &TerminalSnapshot,
    selection: &Selection,
    theme: &Theme,
    scroll_offset: usize,
) -> Vec<(u32, u32)> {
    let target = Target::new(
        theme,
        u32::from(snapshot.cols()) * CELL_WIDTH,
        u32::from(snapshot.rows()) * CELL_HEIGHT,
        poc_metrics(),
    );
    let vertices = glyph_vertices_for_chrome(
        target,
        Some(snapshot),
        FrameChrome::new(None, None)
            .with_scroll_offset(scroll_offset)
            .with_selection(Some(selection)),
    );
    let expected_color = theme.selection_background();
    let mut cells = Vec::new();
    for rectangle in vertices.chunks_exact(6) {
        if !rectangle
            .iter()
            .all(|vertex| vertex.color == expected_color)
        {
            continue;
        }
        let (x0, y0, x1, y1) = rectangle_pixel_bounds(rectangle, target);
        if x1.saturating_sub(x0) != CELL_WIDTH || y1.saturating_sub(y0) != CELL_HEIGHT {
            continue;
        }
        assert_eq!(x0 % CELL_WIDTH, 0, "selection rectangle is off-grid");
        assert_eq!(y0 % CELL_HEIGHT, 0, "selection rectangle is off-grid");
        cells.push((y0 / CELL_HEIGHT, x0 / CELL_WIDTH));
    }
    cells
}

/// Convert one six-vertex axis-aligned rectangle from NDC back to pixel bounds.
fn rectangle_pixel_bounds(rectangle: &[Vertex], target: Target) -> (u32, u32, u32, u32) {
    let left = rectangle
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::INFINITY, f32::min);
    let right = rectangle
        .iter()
        .map(|vertex| vertex.position[0])
        .fold(f32::NEG_INFINITY, f32::max);
    let top = rectangle
        .iter()
        .map(|vertex| vertex.position[1])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = rectangle
        .iter()
        .map(|vertex| vertex.position[1])
        .fold(f32::INFINITY, f32::min);
    let x = |ndc: f32| (((ndc + 1.0) * target.width as f32) / 2.0).round() as u32;
    let y = |ndc: f32| (((1.0 - ndc) * target.height as f32) / 2.0).round() as u32;
    (x(left), y(top), x(right), y(bottom))
}

/// Rasterize the production rectangle stream exactly enough to keep pixel
/// mutation guards active on adapter-less machines. The real wgpu readback is
/// still asserted when Metal exists; this fallback has no parallel glyph or
/// cursor logic because all positions, ordering, and colours come from the
/// shipped vertex generator and its constant-colour fragment shader.
fn rasterized_vertex_frame(target: Target, vertices: &[Vertex]) -> CapturedFrame {
    let [red, green, blue] = target.theme.background_u8();
    let pixel_count = usize::try_from(target.width)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(target.height).unwrap_or(0));
    let mut rgba = vec![0; pixel_count.saturating_mul(4)];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[red, green, blue, u8::MAX]);
    }
    assert_eq!(
        vertices.len() % 6,
        0,
        "the production stream must contain complete rectangles"
    );
    for rectangle in vertices.chunks_exact(6) {
        assert!(
            rectangle
                .iter()
                .all(|vertex| vertex.color == rectangle[0].color),
            "one renderer rectangle must use one constant colour"
        );
        let [red, green, blue] = rectangle[0]
            .color
            .map(|channel| (channel * 255.0).round() as u8);
        let (x0, y0, x1, y1) = rectangle_pixel_bounds(rectangle, target);
        for y in y0.min(target.height)..y1.min(target.height) {
            for x in x0.min(target.width)..x1.min(target.width) {
                let index = (usize::try_from(y).unwrap_or(0)
                    * usize::try_from(target.width).unwrap_or(0)
                    + usize::try_from(x).unwrap_or(0))
                    * 4;
                rgba[index..index + 4].copy_from_slice(&[red, green, blue, u8::MAX]);
            }
        }
    }
    CapturedFrame {
        width: target.width,
        rgba,
    }
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
/// The cells the cursor mark covers this frame: `(row, column span)`.
///
/// Mirrors the renderer's placement rule: nothing when DECTCEM hides the
/// caret; the lead column plus two columns when the caret sits on a wide
/// character (#174/#176 — the block must cover the character, not half of
/// it), one column otherwise.
fn cursor_cell_span(snapshot: &TerminalSnapshot) -> Option<(u32, std::ops::Range<u32>)> {
    if !snapshot.is_cursor_visible() {
        return None;
    }
    let cursor = snapshot.cursor();
    let (row, column) = (u32::from(cursor.row()), u32::from(cursor.column()));
    let span = match snapshot.screen().cell(cursor.row(), cursor.column()) {
        Some(cell) if cell.is_continuation() || cell.width() == 2 => 2,
        _ => 1,
    };
    Some((row, column..column + span))
}

/// Assert every visible cell agrees: state-blank ⟺ render-unlit, with the
/// cursor cell as the one sanctioned exception — a visible caret is
/// *supposed* to light an otherwise blank cell (issues #197/#200). Returns
/// the number of cells checked so the oracle can report coverage.
fn assert_cells_agree(frame: &CapturedFrame, snapshot: &TerminalSnapshot) -> usize {
    let rows = u32::from(snapshot.rows());
    let cols = u32::from(snapshot.cols());
    let cursor = cursor_cell_span(snapshot);
    let mut checked = 0;
    for row in 0..rows {
        for col in 0..cols {
            let state_blank = state_cell_blank(snapshot, row, col);
            let render_blank = !cell_is_lit(frame, row, col);
            let is_cursor_cell = cursor
                .as_ref()
                .is_some_and(|(cursor_row, columns)| *cursor_row == row && columns.contains(&col));
            if is_cursor_cell {
                assert!(
                    !render_blank,
                    "cursor cell ({row},{col}) must be lit — the caret ships drawn"
                );
            } else {
                assert_eq!(
                    state_blank,
                    render_blank,
                    "cell ({row},{col}): state says {}, renderer drew {}",
                    if state_blank { "blank" } else { "char" },
                    if render_blank { "blank" } else { "lit" },
                );
            }
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
    // With the cursor hidden (`CSI ?25l`) a blank screen must still draw
    // nothing at all — the visible-cursor cases, including the blank screen,
    // have their own tests below (issues #197/#200).
    let Some(renderer) = renderer_or_skip("blank_screen_has_no_lit_pixels") else {
        return;
    };
    let snap = snapshot(3, 8, b"\x1b[?25l");
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
    // Each fixture hides the cursor: this test pins glyph containment, and
    // the caret — which by design lights the cell typing lands in — has its
    // own tests.
    let snap = snapshot(1, 4, b"A\x1b[?25l");
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
    let snap2 = snapshot(2, 4, b"A\x1b[?25l");
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
    let snap3 = snapshot(3, 3, b"\x1b[2;2HA\x1b[?25l");
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
fn scrollback_viewport_captures_the_exact_lines_for_each_offset() {
    let Some(renderer) =
        renderer_or_skip("scrollback_viewport_captures_the_exact_lines_for_each_offset")
    else {
        return;
    };
    let snap = snapshot(3, 4, b"\x1b[?25lA\r\nB\r\nC\r\nD\r\nE");
    assert_eq!(snap.scrollback_lines(), ["A", "B"], "fixture history");
    assert_eq!(snap.lines(), ["C", "D", "E"], "fixture live rows");

    // Each expected glyph is rendered independently in a one-row terminal.
    // Comparing its complete cell mask pins identity and row placement; a
    // global "some pixels changed" assertion would not catch a row shift.
    let glyph_pattern = |character: char| {
        let bytes = format!("\x1b[?25l{character}");
        let reference = snapshot(1, 4, bytes.as_bytes());
        cell_pattern(&render(&renderer, &reference), 0, 0)
    };

    for (offset, expected) in [
        (0, ['C', 'D', 'E']),
        (1, ['B', 'C', 'D']),
        (2, ['A', 'B', 'C']),
        (usize::MAX, ['A', 'B', 'C']),
    ] {
        let frame = render_with_scroll_offset(&renderer, &snap, offset);
        for (row, character) in expected.into_iter().enumerate() {
            assert_eq!(
                cell_pattern(&frame, u32::try_from(row).unwrap_or(u32::MAX), 0),
                glyph_pattern(character),
                "offset {offset} row {row} must render exact glyph {character:?}"
            );
            for col in 1..u32::from(snap.cols()) {
                assert!(
                    !cell_is_lit(&frame, u32::try_from(row).unwrap_or(u32::MAX), col),
                    "offset {offset} row {row} leaked content into blank column {col}"
                );
            }
        }
    }
}

#[test]
fn scrollback_vertex_oracle_pins_exact_lines_without_a_gpu() {
    let source = snapshot(3, 4, b"\x1b[?25lA\r\nB\r\nC\r\nD\r\nE");
    let target = Target::new(
        &Theme::default(),
        u32::from(source.cols()) * CELL_WIDTH,
        u32::from(source.rows()) * CELL_HEIGHT,
        poc_metrics(),
    );

    for (offset, expected_bytes, expected_lines) in [
        (0, b"\x1b[?25lC\r\nD\r\nE".as_slice(), ["C", "D", "E"]),
        (1, b"\x1b[?25lB\r\nC\r\nD".as_slice(), ["B", "C", "D"]),
        (2, b"\x1b[?25lA\r\nB\r\nC".as_slice(), ["A", "B", "C"]),
        (
            usize::MAX,
            b"\x1b[?25lA\r\nB\r\nC".as_slice(),
            ["A", "B", "C"],
        ),
    ] {
        let expected = snapshot(3, 4, expected_bytes);
        assert_eq!(expected.lines(), expected_lines, "expected frame fixture");
        let actual_vertices = glyph_vertices_for_chrome(
            target,
            Some(&source),
            FrameChrome::new(None, None).with_scroll_offset(offset),
        );
        let expected_vertices =
            glyph_vertices_for_chrome(target, Some(&expected), FrameChrome::new(None, None));
        assert_eq!(
            actual_vertices, expected_vertices,
            "offset {offset} must produce the complete frame for exact lines {expected_lines:?}"
        );
    }
}

#[test]
fn scrollback_viewport_keeps_a_wide_pair_and_following_glyph_in_captured_pixels() {
    let Some(renderer) = renderer_or_skip(
        "scrollback_viewport_keeps_a_wide_pair_and_following_glyph_in_captured_pixels",
    ) else {
        return;
    };
    let snap = snapshot(2, 5, "\x1b[?25l日A\r\n語B\r\n界C".as_bytes());
    assert_eq!(snap.scrollback_lines(), ["日A"], "fixture history");

    let history_frame = render_with_scroll_offset(&renderer, &snap, 1);
    let expected_row = render(&renderer, &snapshot(1, 5, "\x1b[?25l日A".as_bytes()));
    for col in 0..u32::from(snap.cols()) {
        assert_eq!(
            cell_pattern(&history_frame, 0, col),
            cell_pattern(&expected_row, 0, col),
            "scrolled wide history row differs at display column {col}"
        );
    }
    assert!(cell_is_lit(&history_frame, 0, 0), "wide lead is absent");
    assert!(
        !cell_is_lit(&history_frame, 0, 1),
        "wide continuation column must remain blank"
    );
    let a_reference = render(&renderer, &snapshot(1, 1, b"\x1b[?25lA"));
    assert_eq!(
        cell_pattern(&history_frame, 0, 2),
        cell_pattern(&a_reference, 0, 0),
        "glyph after the wide pair must stay in display column 2"
    );
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
    // Cursor hidden: this test pins the *grid* mapping of glyphs, and the
    // caret is not grid content.
    let snap = snapshot(rows, cols, b"ABCDE\r\nFGHI\r\nKL\x1b[?25l");
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
    // One column: the cursor would wrap onto this very cell, so hide it —
    // the subject here is the background/glyph pair, not the caret.
    let snap = snapshot(1, 1, b"\x1b[?25l\x1b[38;2;241;207;33;48;2;12;98;201mA");
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
    // Cursor hidden: the subject is the background rect on an empty cell.
    let snap = snapshot(1, 1, b"\x1b[?25l\x1b[48;2;73;18;146m ");
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
    state.feed_bytes(b"\x1b[?25l");
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
    // Cursor hidden in every fixture: this test pins where the *glyph*
    // after a wide character lands, and the caret would light exactly the
    // kind of trailing cell asserted blank here.
    let b_reference = render(&renderer, &snapshot(1, 4, b"b\x1b[?25l"));
    for (label, bytes) in [
        ("CJK 日", "日b".as_bytes()),
        ("wide emoji 😀", "😀b".as_bytes()),
    ] {
        let snap = snapshot(1, 4, &[bytes, b"\x1b[?25l"].concat());
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
    // Cursor hidden in both fixtures: the subject is cell consumption, and
    // the trailing columns asserted blank are exactly where the caret lands.
    let marked = snapshot(1, 4, "e\u{0301}f\x1b[?25l".as_bytes());
    let plain = snapshot(1, 4, b"ef\x1b[?25l");

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
    // two empty columns whose pixels show the theme's clear colour. The
    // cursor is hidden: the empty columns must stay clear, and the caret
    // has its own theme tests below.
    let snap = snapshot(1, 6, b"\x1b[31mb\x1b[32mc\x1b[0ma\x1b[41md\x1b[?25l");

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
    let snap = snapshot(1, 6, b"\x1b[31mb\x1b[32mc\x1b[0ma\x1b[41md\x1b[?25l");

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
// Selection highlight (issue #202): the app-owned range about to be copied
// must change exactly those frame cells. These tests compare every pixel of
// every cell before/after selection, so whole-row, shifted-column, and dropped
// overlays cannot satisfy the oracle by merely lighting some distinct pixels.
// ===========================================================================

#[test]
fn selection_highlight_vertex_oracle_pins_the_ascii_range() {
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes(b"abcdef\x1b[?25l");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 1),
        GridPoint::new(0, 2),
    );
    assert_eq!(
        selection_background_vertex_cells(&state.snapshot(), &selection, &DARK),
        [(0, 1), (0, 2)]
    );
}

#[test]
fn selection_highlight_vertex_oracle_pins_the_wrapped_range() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"abcdef\x1b[?25l");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 2),
        GridPoint::new(1, 0),
    );
    assert_eq!(
        selection_background_vertex_cells(&state.snapshot(), &selection, &DARK),
        [(0, 2), (0, 3), (1, 0)]
    );
}

#[test]
fn selection_highlight_vertex_oracle_pins_both_wide_columns() {
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("a日bc\x1b[?25l".as_bytes());
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 2),
        GridPoint::new(0, 2),
    );
    assert_eq!(
        selection_background_vertex_cells(&state.snapshot(), &selection, &DARK),
        [(0, 1), (0, 2)]
    );
}

#[test]
fn selection_made_while_scrolled_highlights_visible_rows_and_matches_extract() {
    let mut state = TerminalState::new(3, 8).expect("valid terminal");
    state.feed_bytes(b"AAAA  \r\nBBBB  \r\nCCCC  \r\nDDDD  \r\nEEEE\x1b[?25l");
    assert_eq!(state.scrollback_len(), 2, "fixture history");

    // Offset one exposes logical rows B/C/D. Model a drag from B's second
    // column through the row's blank tail: extraction must trim the blanks,
    // and the renderer must paint only the surviving `BBB` span on frame row
    // zero. At the live-tail mapping this absolute row would not be painted at
    // all, so the assertion directly guards the merge's row-index boundary.
    const SCROLL_OFFSET: usize = 1;
    let visible_logical_start = state.scrollback_len() - SCROLL_OFFSET;
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(visible_logical_start, 1),
        GridPoint::new(visible_logical_start, 7),
    );
    let copied = selection.extract(&state);
    assert_eq!(copied, "BBB", "copy trims the visible row's blank tail");

    let snap = state.snapshot();
    let highlighted = selection_background_vertex_cells_with_scroll_offset(
        &snap,
        &selection,
        &DARK,
        SCROLL_OFFSET,
    );
    assert_eq!(
        highlighted,
        [(0, 1), (0, 2), (0, 3)],
        "the absolute B-row selection must appear on the scrolled frame's first row"
    );

    let mut highlighted_text = String::new();
    for &(frame_row, column) in &highlighted {
        let logical_line = visible_logical_start + frame_row as usize;
        let cells = snap
            .logical_row(u32::try_from(logical_line).expect("fixture line fits u32"))
            .expect("highlighted logical row exists");
        let cell = &cells[column as usize];
        if !cell.is_continuation() {
            highlighted_text.push_str(cell.text());
        }
    }
    assert_eq!(
        highlighted_text, copied,
        "text under the highlighted frame cells must equal extract"
    );

    let Some(renderer) = renderer_or_skip(
        "selection_made_while_scrolled_highlights_visible_rows_and_matches_extract",
    ) else {
        return;
    };
    let before = render_with_scroll_offset(&renderer, &snap, SCROLL_OFFSET);
    let after =
        render_with_selection_and_scroll_offset(&renderer, &DARK, &snap, &selection, SCROLL_OFFSET);
    assert_eq!(
        changed_cells(&before, &after, snap.rows(), snap.cols()),
        highlighted,
        "captured pixels must agree with the production vertex oracle"
    );
}

#[test]
fn selection_highlight_changes_only_the_selected_cells() {
    let Some(renderer) = renderer_or_skip("selection_highlight_changes_only_the_selected_cells")
    else {
        return;
    };
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes(b"abcdef\x1b[?25l");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 1),
        GridPoint::new(0, 2),
    );
    assert_eq!(selection.extract(&state), "bc");
    let snap = state.snapshot();
    let before = render(&renderer, &snap);
    let after = render_with_selection(&renderer, &DARK, &snap, &selection);

    assert_ne!(
        before.rgba, after.rgba,
        "a real selection must change pixels"
    );
    assert_eq!(
        changed_cells(&before, &after, snap.rows(), snap.cols()),
        [(0, 1), (0, 2)],
        "only the two copied cells may change"
    );
}

#[test]
fn selection_highlight_tracks_the_exact_wrapped_range() {
    let Some(renderer) = renderer_or_skip("selection_highlight_tracks_the_exact_wrapped_range")
    else {
        return;
    };
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    // `e` triggers the pending wrap after `abcd`; the selected copy range is
    // the tail `cd` on row 0 plus `e` on row 1.
    state.feed_bytes(b"abcdef\x1b[?25l");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 2),
        GridPoint::new(1, 0),
    );
    assert_eq!(selection.extract(&state), "cd\ne");
    let snap = state.snapshot();
    let before = render(&renderer, &snap);
    let after = render_with_selection(&renderer, &DARK, &snap, &selection);

    assert_eq!(
        changed_cells(&before, &after, snap.rows(), snap.cols()),
        [(0, 2), (0, 3), (1, 0)],
        "wrapping must not widen either selected row"
    );
}

#[test]
fn selection_highlight_covers_both_wide_columns_and_never_continuation_alone() {
    let Some(renderer) = renderer_or_skip(
        "selection_highlight_covers_both_wide_columns_and_never_continuation_alone",
    ) else {
        return;
    };
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    // Columns: a=0, 日=1(+2 continuation), b=3, c=4. Deliberately capture
    // both endpoints on column 2: normalization must paint the lead and its
    // continuation, with neither neighbour changed.
    state.feed_bytes("a日bc\x1b[?25l".as_bytes());
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 2),
        GridPoint::new(0, 2),
    );
    assert_eq!(selection.extract(&state), "日");
    let snap = state.snapshot();
    let before = render(&renderer, &snap);
    let after = render_with_selection(&renderer, &DARK, &snap, &selection);

    assert_eq!(
        changed_cells(&before, &after, snap.rows(), snap.cols()),
        [(0, 1), (0, 2)],
        "a wide character is one selection unit spanning exactly two columns"
    );
    for col in [1, 2] {
        assert!(
            cell_color_pixel_count(&after, 0, col, DARK.selection_background_u8()) > 0,
            "wide selection column {col} lacks the theme background"
        );
    }
}

#[test]
fn selection_highlight_follows_each_configured_theme_in_frame_pixels() {
    let Some(renderer) =
        renderer_or_skip("selection_highlight_follows_each_configured_theme_in_frame_pixels")
    else {
        return;
    };
    let mut state = TerminalState::new(1, 3).expect("valid terminal");
    state.feed_bytes(b"abc\x1b[?25l");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 1),
        GridPoint::new(0, 1),
    );
    let snap = state.snapshot();

    for name in ["dark", "light", "high-contrast"] {
        let config = AppConfig::parse(&format!("[theme]\nname = \"{name}\"\n"))
            .expect("shipped theme is configurable");
        let theme = config.theme().palette();
        let before = render_with_theme(&renderer, &theme, &snap);
        let after = render_with_selection(&renderer, &theme, &snap, &selection);
        assert_eq!(
            changed_cells(&before, &after, snap.rows(), snap.cols()),
            [(0, 1)],
            "{name}: only the configured theme's selected cell may change"
        );
        let corner = after.pixel(2 * CELL_WIDTH - 1, CELL_HEIGHT - 1);
        assert!(
            colors_match(
                [corner[0], corner[1], corner[2]],
                theme.selection_background_u8()
            ),
            "{name}: selected-cell corner {corner:?} is not the theme background {:?}",
            theme.selection_background_u8()
        );
        assert!(
            cell_color_pixel_count(&after, 0, 1, theme.selection_foreground_u8()) > 0,
            "{name}: selected glyph does not use the readable theme foreground"
        );
    }
}

/// A selected cell still exposes the focused block caret by reversing the
/// selection pair. The control frame differs only by DECTCEM visibility, so
/// every compared pixel belongs to the same glyph in the same selected cell.
#[test]
fn cursor_inside_selection_inverts_the_same_cell_pixels() {
    // Reuse the passing cursor oracle's visible/hidden mechanism: `Target::new`
    // supplies the focused block style, while DECTCEM alone controls whether
    // the otherwise-identical frame draws it.
    let visible = snapshot(1, 3, b"A\r\x1b[?25h");
    let hidden = snapshot(1, 3, b"A\r\x1b[?25l");
    assert_eq!(visible.cursor(), hidden.cursor());
    assert_eq!(
        (visible.cursor().row(), visible.cursor().column()),
        (0, 0),
        "fixture must put the caret inside the selected cell"
    );
    assert!(visible.is_cursor_visible());
    assert!(!hidden.is_cursor_visible());

    let selection = Selection::new(
        &visible,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 0),
    );
    assert_eq!(selection.extract(&visible), "A");
    assert_eq!(selection.extract(&hidden), "A");

    let foreground = DARK.selection_foreground_u8();
    let background = DARK.selection_background_u8();
    let assert_same_cell_inversion = |caret: &CapturedFrame, plain: &CapturedFrame| {
        let caret_pixels = cell_pixels(caret, 0, 0);
        let plain_pixels = cell_pixels(plain, 0, 0);
        assert_ne!(
            caret_pixels, plain_pixels,
            "the caret must change its own selected cell"
        );
        for (index, (caret, plain)) in caret_pixels.iter().zip(&plain_pixels).enumerate() {
            let caret_rgb = [caret[0], caret[1], caret[2]];
            let plain_rgb = [plain[0], plain[1], plain[2]];
            let expected = if colors_match(plain_rgb, foreground) {
                background
            } else {
                assert!(
                    colors_match(plain_rgb, background),
                    "plain selected-cell pixel {index} is outside the selection pair: {plain:?}"
                );
                foreground
            };
            assert!(
                colors_match(caret_rgb, expected),
                "caret pixel {index} must invert the same plain selected-cell pixel: \
                 plain={plain:?}, caret={caret:?}, expected RGB {expected:?}"
            );
            assert_eq!(
                caret[3], plain[3],
                "caret pixel {index} must preserve the same cell pixel's alpha"
            );
        }
    };

    let target = Target::new(
        &DARK,
        u32::from(visible.cols()) * CELL_WIDTH,
        u32::from(visible.rows()) * CELL_HEIGHT,
        poc_metrics(),
    );
    let chrome = FrameChrome::new(None, None).with_selection(Some(&selection));
    let caret_vertices = glyph_vertices_for_chrome(target, Some(&visible), chrome);
    let plain_vertices = glyph_vertices_for_chrome(target, Some(&hidden), chrome);
    let caret_vertex_frame = rasterized_vertex_frame(target, &caret_vertices);
    let plain_vertex_frame = rasterized_vertex_frame(target, &plain_vertices);
    assert_same_cell_inversion(&caret_vertex_frame, &plain_vertex_frame);

    let Some(renderer) = renderer_or_skip("cursor_inside_selection_inverts_the_same_cell_pixels")
    else {
        return;
    };
    let caret_frame = render_with_selection(&renderer, &DARK, &visible, &selection);
    let plain_frame = render_with_selection(&renderer, &DARK, &hidden, &selection);
    assert_same_cell_inversion(&caret_frame, &plain_frame);
}

// ===========================================================================
// The cursor (issues #197/#200): every terminal a user has ever used draws a
// caret; Noren drew none, so every keystroke edited blind. The tests below
// prove the caret through the frame oracle — real pixels from the real
// pipeline — not by asserting state flags. The default is *drawn*: a user
// who reads nothing gets a caret with no configuration, and `[cursor]`
// configuration only changes how it looks (shape, colour), never whether it
// is there.
// ===========================================================================

/// Render a snapshot under an explicit theme and cursor style.
fn render_with_cursor_style(
    renderer: &OffscreenRenderer,
    theme: &Theme,
    cursor: CursorStyle,
    snapshot: &TerminalSnapshot,
) -> CapturedFrame {
    let width = u32::from(snapshot.cols()) * CELL_WIDTH;
    let height = u32::from(snapshot.rows()) * CELL_HEIGHT;
    renderer.capture(
        Target::new(theme, width, height, poc_metrics()).with_cursor_style(cursor),
        Some(snapshot),
        None,
        None,
    )
}

/// Whether every pixel of cell `(row, col)` is the given colour — the shape
/// of a focused block cursor over an otherwise blank cell.
fn cell_is_solid(frame: &CapturedFrame, row: u32, col: u32, color: [u8; 3]) -> bool {
    (0..CELL_HEIGHT).all(|y| {
        (0..CELL_WIDTH).all(|x| {
            colors_match(
                {
                    let pixel = frame.pixel(col * CELL_WIDTH + x, row * CELL_HEIGHT + y);
                    [pixel[0], pixel[1], pixel[2]]
                },
                color,
            )
        })
    })
}

/// The theme's unstyled cursor baseline as captured bytes (the same
/// quantization `Theme::cursor_u8` performs).
fn cursor_rgb(theme: &Theme) -> [u8; 3] {
    theme.cursor_u8()
}

/// The headline assertion of issues #197/#200: with no configuration of any
/// kind, the caret is drawn at the tracked position. A renderer that stops
/// drawing the cursor fails here at the pixel level — mutation (a) of the
/// cursor work, run before trusting this test.
#[test]
fn the_cursor_is_drawn_by_default_at_the_tracked_position() {
    let Some(renderer) = renderer_or_skip("the_cursor_is_drawn_by_default_at_the_tracked_position")
    else {
        return;
    };
    // 'ab' leaves the tracked cursor at display column 2; nothing else on
    // row 1 or columns 3+.
    let snap = snapshot(2, 4, b"ab");
    assert_eq!(
        (snap.cursor().row(), snap.cursor().column()),
        (0, 2),
        "fixture must leave the cursor at (0,2)"
    );
    let frame = render(&renderer, &snap);

    // The cursor cell is a solid block of the dark theme's cursor colour.
    assert!(
        cell_is_solid(&frame, 0, 2, cursor_rgb(&DARK)),
        "the cursor cell (0,2) must be a solid block in the theme cursor \
         colour {:?} — found pixels: see failure",
        cursor_rgb(&DARK)
    );
    // The text cells are untouched by the caret.
    assert!(cell_is_lit(&frame, 0, 0) && cell_is_lit(&frame, 0, 1));
    // Everything past the caret, and every other row, stays clear.
    assert!(!cell_is_lit(&frame, 0, 3));
    for col in 0..u32::from(snap.cols()) {
        assert!(
            !cell_is_lit(&frame, 1, col),
            "row 1 col {col} must stay clear — the caret is on row 0"
        );
    }
}

/// The fresh-screen case the display model used to trim away: a completely
/// blank screen still shows its caret at (0,0) — exactly the row typing
/// lands in. With DECTCEM hiding it, the screen returns to fully clear.
#[test]
fn blank_screen_draws_only_the_cursor() {
    let Some(renderer) = renderer_or_skip("blank_screen_draws_only_the_cursor") else {
        return;
    };
    let snap = snapshot(3, 8, b"");
    let frame = render(&renderer, &snap);
    assert!(
        cell_is_solid(&frame, 0, 0, cursor_rgb(&DARK)),
        "a blank screen must still draw its caret at (0,0)"
    );
    for row in 0..u32::from(snap.rows()) {
        for col in 0..u32::from(snap.cols()) {
            if row == 0 && col == 0 {
                continue;
            }
            assert!(
                !cell_is_lit(&frame, row, col),
                "cell ({row},{col}) must stay clear on a blank screen"
            );
        }
    }

    let hidden = snapshot(3, 8, b"\x1b[?25l");
    let hidden_frame = render(&renderer, &hidden);
    for row in 0..u32::from(hidden.rows()) {
        for col in 0..u32::from(hidden.cols()) {
            assert!(
                !cell_is_lit(&hidden_frame, row, col),
                "with the caret hidden, cell ({row},{col}) must be clear"
            );
        }
    }
}

/// Issue #200's own proposed guard: moving the cursor must change pixels.
/// The same text with the caret in two positions yields two different
/// frames, and each caret cell is exactly where its snapshot says.
#[test]
fn moving_the_cursor_changes_pixels() {
    let Some(renderer) = renderer_or_skip("moving_the_cursor_changes_pixels") else {
        return;
    };
    // Same text, caret moved between two blank cells (a caret on a glyph
    // inverts it — covered by the wide-character and shape tests below).
    let at_third = snapshot(1, 4, b"hi\x1b[1;3H");
    let at_fourth = snapshot(1, 4, b"hi\x1b[1;4H");
    assert_eq!(
        (at_third.cursor().column(), at_fourth.cursor().column()),
        (2, 3)
    );

    let frame_third = render(&renderer, &at_third);
    let frame_fourth = render(&renderer, &at_fourth);
    assert_ne!(
        frame_third.rgba, frame_fourth.rgba,
        "moving the cursor must change the drawn pixels"
    );
    // Each caret is where its snapshot tracks it, and only there.
    assert!(
        cell_is_solid(&frame_third, 0, 2, cursor_rgb(&DARK)),
        "caret at column 2 must mark cell (0,2)"
    );
    assert!(
        !cell_is_solid(&frame_third, 0, 3, cursor_rgb(&DARK)),
        "the caret is not at column 3 in the first frame"
    );
    assert!(
        cell_is_solid(&frame_fourth, 0, 3, cursor_rgb(&DARK)),
        "caret at column 3 must mark cell (0,3)"
    );
    assert!(
        !cell_is_solid(&frame_fourth, 0, 2, cursor_rgb(&DARK)),
        "the caret is not at column 2 in the second frame"
    );
    // The typed text is identical in both frames.
    for col in 0..2_u32 {
        assert_eq!(
            cell_pattern(&frame_third, 0, col),
            cell_pattern(&frame_fourth, 0, col)
        );
    }
}

/// The wide-character contract for the caret (#174/#176): a block cursor
/// after a wide character sits at its *display* column (two past the lead),
/// and a caret on the wide character itself covers both columns — never
/// half a character. A renderer that counts cells instead of display
/// columns draws the block one column early and fails both halves below;
/// that is mutation (b) of the cursor work, run before trusting this test.
#[test]
fn a_block_cursor_on_a_wide_character_covers_both_columns() {
    let Some(renderer) = renderer_or_skip("a_block_cursor_on_a_wide_character_covers_both_columns")
    else {
        return;
    };
    // 界 (columns 0–1), red 'X' (column 2), tracked cursor at display
    // column 3. The X is deliberately SGR-red so the glyph colour differs
    // from the cursor colour and a misdrawn block cannot hide in the match.
    let snap = snapshot(1, 6, "界\x1b[31mX".as_bytes());
    assert_eq!((snap.cursor().row(), snap.cursor().column()), (0, 3));
    let frame = render(&renderer, &snap);

    assert!(
        cell_is_solid(&frame, 0, 3, cursor_rgb(&DARK)),
        "the caret must sit at display column 3 — the columns a wide \
         character actually occupies are 0 and 1, not 0 alone"
    );
    assert!(
        !cell_is_solid(&frame, 0, 2, cursor_rgb(&DARK)),
        "the block must not cover the X at column 2"
    );
    let x_color = cell_color(&frame, 0, 2);
    assert!(
        colors_match(x_color, DARK.ansi()[1]),
        "the X at column 2 must keep its red glyph, found {x_color:?}"
    );

    // Now the caret on the wide character itself: it must cover BOTH
    // columns — the continuation column is solid cursor colour (it has no
    // glyph of its own), and the lead column shows the inverted glyph over
    // the block (cursor colour and background colour only).
    let on_wide = snapshot(1, 6, "界\x1b[31mX\x1b[1;1H".as_bytes());
    assert_eq!(on_wide.cursor().column(), 0);
    let on_wide_frame = render(&renderer, &on_wide);
    assert!(
        cell_is_solid(&on_wide_frame, 0, 1, cursor_rgb(&DARK)),
        "the continuation column of the wide character must be covered by \
         the block — covering only the lead is half a character"
    );
    let lead_colors = cell_colors(&on_wide_frame, 0, 0);
    for color in lead_colors {
        assert!(
            colors_match(color, cursor_rgb(&DARK)) || colors_match(color, DARK.background_u8()),
            "the inverted glyph over the block may draw only the cursor \
             colour and the background colour, found {color:?}"
        );
    }
    // The X past the pair is untouched.
    assert!(colors_match(
        cell_color(&on_wide_frame, 0, 2),
        DARK.ansi()[1]
    ));
}

/// DECTCEM is a contract with programs, not a preference: vim hides the
/// caret during redraw (`CSI ?25l`) and a caret painted over a hidden state
/// is worse than none. Hidden draws no mark anywhere; `CSI ?25h` restores
/// it — and the hidden frame is byte-identical to one that never knew the
/// cursor existed.
#[test]
fn dectcem_hides_and_restores_the_cursor_in_pixels() {
    let Some(renderer) = renderer_or_skip("dectcem_hides_and_restores_the_cursor_in_pixels") else {
        return;
    };
    let hidden = snapshot(2, 4, b"ab\x1b[?25l");
    let shown = snapshot(2, 4, b"ab\x1b[?25h");
    let hidden_frame = render(&renderer, &hidden);
    let shown_frame = render(&renderer, &shown);

    // Hidden: the cursor cell reverts to clear; nothing else may appear.
    assert!(
        !cell_is_lit(&hidden_frame, 0, 2),
        "a DECTCEM-hidden cursor must not be drawn at its tracked position"
    );
    for row in 0..2 {
        for col in 0..4 {
            let expected_lit = row == 0 && col < 2;
            assert_eq!(
                cell_is_lit(&hidden_frame, row, col),
                expected_lit,
                "hidden-cursor frame ({row},{col}): only the typed 'ab' may be lit"
            );
        }
    }
    // Restored: the caret is back exactly at the tracked position.
    assert!(
        cell_is_solid(&shown_frame, 0, 2, cursor_rgb(&DARK)),
        "CSI ?25h must restore the caret at the tracked position"
    );
    assert_ne!(
        hidden_frame.rgba, shown_frame.rgba,
        "hide and show must differ in pixels"
    );
}

/// On an unstyled cell the cursor baseline is theme-owned, so the caret must
/// be visible — with measured WCAG contrast — on every theme's background,
/// asserted on the readback bytes themselves (dark 15.39:1, light 14.56:1,
/// high-contrast 21.0:1; `tests/theme.rs` pins the same numbers at the palette
/// level). SGR cells have their separate cell-relative regression below.
#[test]
fn the_cursor_colour_comes_from_the_theme_and_clears_contrast_in_pixels() {
    let Some(renderer) =
        renderer_or_skip("the_cursor_colour_comes_from_the_theme_and_clears_contrast_in_pixels")
    else {
        return;
    };
    for theme in [&DARK, &LIGHT, &HIGH_CONTRAST] {
        let snap = snapshot(1, 4, b"\x1b[?25h");
        let frame = render_with_theme(&renderer, theme, &snap);
        assert!(
            cell_is_solid(&frame, 0, 0, cursor_rgb(theme)),
            "the caret must draw this theme's cursor colour {:?}",
            cursor_rgb(theme)
        );
        // The centre of the block, read back as bytes, against this
        // theme's readback ground: the visibility claim, measured.
        let block = {
            let pixel = frame.pixel(CELL_WIDTH / 2, CELL_HEIGHT / 2);
            [pixel[0], pixel[1], pixel[2]]
        };
        assert!(colors_match(block, cursor_rgb(theme)));
        let ground = {
            let pixel = frame.pixel(3 * CELL_WIDTH, CELL_HEIGHT / 2);
            [pixel[0], pixel[1], pixel[2]]
        };
        assert!(colors_match(ground, theme.background_u8()));
        let ratio = contrast_ratio(block, ground);
        let floor = if *theme == HIGH_CONTRAST { 7.0 } else { 4.5 };
        assert!(
            ratio >= floor,
            "{theme:?} cursor {block:?} on {ground:?} is {ratio:.2}:1, below {floor}:1"
        );
    }
}

/// Regression for the SGR-background defect found after issues #197/#200:
/// cursor ink is resolved against the cell it covers, not the theme ground.
/// These are real terminal attributes driven through the parser and real
/// readback pixels from the shipped render pipeline.
#[test]
fn cursor_contrast_tracks_four_real_sgr_backgrounds() {
    let Some(renderer) = renderer_or_skip("cursor_contrast_tracks_four_real_sgr_backgrounds")
    else {
        return;
    };
    let cases: [(&str, &[u8], [u8; 3]); 4] = [
        ("light ANSI SGR 47", b"\x1b[47m", DARK.ansi()[7]),
        ("dark ANSI SGR 40", b"\x1b[40m", DARK.ansi()[0]),
        (
            "background equal to the theme cursor",
            b"\x1b[48;2;204;235;209m",
            cursor_rgb(&DARK),
        ),
        (
            "truecolor SGR 48;2;17;119;221",
            b"\x1b[48;2;17;119;221m",
            [17, 119, 221],
        ),
    ];

    for (label, sgr, expected_background) in cases {
        // Two background-painted spaces, then return the cursor to the first:
        // cell 0 is the caret and cell 1 exposes the actual comparison ground.
        let mut bytes = sgr.to_vec();
        bytes.extend_from_slice(b"  \r\x1b[?25h");
        let snap = snapshot(1, 3, &bytes);
        let frame = render(&renderer, &snap);

        assert!(
            cell_is_solid(&frame, 0, 0, [0, 0, 0]),
            "{label}: the unusable inverse foreground must fall back to black"
        );
        assert!(
            cell_is_solid(&frame, 0, 1, expected_background),
            "{label}: comparison cell must expose SGR background {expected_background:?}"
        );
        let ink_pixel = frame.pixel(CELL_WIDTH / 2, CELL_HEIGHT / 2);
        let ground_pixel = frame.pixel(CELL_WIDTH + CELL_WIDTH / 2, CELL_HEIGHT / 2);
        let ink = [ink_pixel[0], ink_pixel[1], ink_pixel[2]];
        let ground = [ground_pixel[0], ground_pixel[1], ground_pixel[2]];
        let ratio = contrast_ratio(ink, ground);
        eprintln!("cursor SGR oracle: {label} ink={ink:?} ground={ground:?} ratio={ratio:.6}:1");
        assert!(
            ratio >= 4.5,
            "{label}: cursor {ink:?} on actual cell ground {ground:?} is only {ratio:.6}:1"
        );
    }
}

/// A focused block swaps the readable cell pair: cell foreground becomes the
/// block and cell background becomes the glyph. The glyph keeps the same
/// raster coverage it has without a cursor, proving readability rather than
/// merely proving that two colours occur somewhere in the cell.
#[test]
fn block_cursor_keeps_the_sgr_glyph_readable_after_inversion() {
    let Some(renderer) =
        renderer_or_skip("block_cursor_keeps_the_sgr_glyph_readable_after_inversion")
    else {
        return;
    };
    const FOREGROUND: [u8; 3] = [20, 30, 40];
    const BACKGROUND: [u8; 3] = [240, 240, 240];
    let visible = snapshot(1, 3, b"\x1b[38;2;20;30;40;48;2;240;240;240mA\r\x1b[?25h");
    let hidden = snapshot(1, 3, b"\x1b[38;2;20;30;40;48;2;240;240;240mA\r\x1b[?25l");
    let visible_frame = render(&renderer, &visible);
    let hidden_frame = render(&renderer, &hidden);

    let ordinary_glyph_pixels = cell_color_pixel_count(&hidden_frame, 0, 0, FOREGROUND);
    let inverted_glyph_pixels = cell_color_pixel_count(&visible_frame, 0, 0, BACKGROUND);
    assert!(ordinary_glyph_pixels > 0, "the control A must draw a glyph");
    assert_eq!(
        inverted_glyph_pixels, ordinary_glyph_pixels,
        "the inverted A must retain every glyph pixel"
    );
    assert_eq!(
        cell_color_pixel_count(&visible_frame, 0, 0, FOREGROUND) + inverted_glyph_pixels,
        (CELL_WIDTH * CELL_HEIGHT) as usize,
        "the cursor cell may contain only its block and readable glyph"
    );
    let ratio = contrast_ratio(FOREGROUND, BACKGROUND);
    assert!(ratio >= 4.5, "fixture pair must itself be readable");
}

/// `[cursor] color` remains meaningful, but cannot command invisible output:
/// a safe colour is used exactly, while a colour matching the cell background
/// falls back first to the cell's readable inverse foreground.
#[test]
fn cursor_override_is_used_when_safe_and_falls_back_when_not() {
    let Some(renderer) =
        renderer_or_skip("cursor_override_is_used_when_safe_and_falls_back_when_not")
    else {
        return;
    };
    let white = [1.0, 1.0, 1.0];
    let dark = snapshot(1, 3, b"\x1b[48;2;8;18;28m  \r");
    let safe = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_color_override(Some(white)),
        &dark,
    );
    assert!(
        cell_is_solid(&safe, 0, 0, [255, 255, 255]),
        "a safe override must be drawn exactly"
    );

    let light = snapshot(1, 3, b"\x1b[38;2;20;30;40;48;2;240;240;240m  \r");
    let matching_background = [240.0 / 255.0; 3];
    let fallback = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_color_override(Some(matching_background)),
        &light,
    );
    assert!(
        cell_is_solid(&fallback, 0, 0, [20, 30, 40]),
        "an invisible override must fall back to the readable inverse foreground"
    );
    assert!(
        !cell_is_solid(&fallback, 0, 0, [240, 240, 240]),
        "an override equal to the ground must never erase the caret"
    );
}

/// Vim could not be driven in the independent review's WindowServer-less UI,
/// so exercise the relevant contract directly: DECTCEM hide and show while
/// the cursor sits on an SGR-painted light cell.
#[test]
fn dectcem_hide_and_show_work_on_an_sgr_background() {
    let mut terminal = TerminalState::new(1, 3).expect("valid test terminal");
    terminal.feed_bytes(b"\x1b[47m  \r\x1b[?25l");
    let hidden = terminal.snapshot();
    assert!(!hidden.is_cursor_visible());

    terminal.feed_bytes(b"\x1b[?25h");
    let shown = terminal.snapshot();
    assert!(shown.is_cursor_visible());

    // Keep the parser/state half of this contract live even on hosts where
    // the rendered-frame half must skip because no GPU adapter is exposed.
    let Some(renderer) = renderer_or_skip("dectcem_hide_and_show_work_on_an_sgr_background") else {
        return;
    };
    let hidden_frame = render(&renderer, &hidden);
    let shown_frame = render(&renderer, &shown);

    let light = DARK.ansi()[7];
    assert!(cell_is_solid(&hidden_frame, 0, 0, light));
    assert!(cell_is_solid(&shown_frame, 0, 0, [0, 0, 0]));
    assert!(cell_is_solid(&hidden_frame, 0, 1, light));
    assert!(cell_is_solid(&shown_frame, 0, 1, light));
    assert_ne!(hidden_frame.rgba, shown_frame.rgba);
}

/// Shape is configuration, not existence: bar and underline marks draw their
/// strokes without inverting the glyph beneath them (only the focused block
/// inverts), over the glyph so the mark stays solid.
#[test]
fn bar_and_underline_shapes_mark_the_cell_without_inverting_the_glyph() {
    let Some(renderer) =
        renderer_or_skip("bar_and_underline_shapes_mark_the_cell_without_inverting_the_glyph")
    else {
        return;
    };
    let stroke = 2_u32; // CURSOR_STROKE in the renderer

    // A bar on a blank cell: the left stroke runs the full cell height in
    // the cursor colour; the rest of the cell stays clear.
    let blank = snapshot(1, 4, b"");
    let bar = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_shape(CursorShape::Bar),
        &blank,
    );
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let pixel = bar.pixel(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];
            let in_bar = x < stroke;
            assert_eq!(
                colors_match(rgb, cursor_rgb(&DARK)),
                in_bar,
                "bar cursor: pixel ({x},{y}) must be {} — bar pixels are the \
                 left {stroke} columns only",
                if in_bar { "cursor-coloured" } else { "clear" }
            );
        }
    }

    // An underline on a blank cell: the bottom stroke spans the cell width.
    let underline = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_shape(CursorShape::Underline),
        &blank,
    );
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let pixel = underline.pixel(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];
            let in_underline = y >= CELL_HEIGHT - stroke;
            assert_eq!(
                colors_match(rgb, cursor_rgb(&DARK)),
                in_underline,
                "underline cursor: pixel ({x},{y}) must be {} — underline \
                 pixels are the bottom {stroke} rows only",
                if in_underline {
                    "cursor-coloured"
                } else {
                    "clear"
                }
            );
        }
    }

    // Neither shape inverts the glyph it shares a cell with: the 'A' under
    // a bar keeps the default foreground outside the stroke.
    let over_a = snapshot(1, 4, b"A\r");
    let bar_over_a = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_shape(CursorShape::Bar),
        &over_a,
    );
    let colors = cell_colors(&bar_over_a, 0, 0);
    for color in colors {
        assert!(
            colors_match(color, cursor_rgb(&DARK)) || colors_match(color, default_foreground_rgb()),
            "a bar must not invert the glyph beneath it; found {color:?}"
        );
    }
    // And both differ from the block default.
    let block = render(&renderer, &over_a);
    assert_ne!(bar_over_a.rgba, block.rgba);
}

/// Focus loss is visible (issue #200): the unfocused caret is a hollow
/// outline of the block footprint — border lit, interior clear — and the
/// two treatments differ in pixels.
#[test]
fn an_unfocused_cursor_is_a_hollow_outline_and_differs_from_focused() {
    let Some(renderer) =
        renderer_or_skip("an_unfocused_cursor_is_a_hollow_outline_and_differs_from_focused")
    else {
        return;
    };
    let stroke = 2_u32; // CURSOR_STROKE in the renderer
    let snap = snapshot(1, 4, b"");
    let focused = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_focus(true),
        &snap,
    );
    let unfocused = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_focus(false),
        &snap,
    );

    assert_ne!(
        focused.rgba, unfocused.rgba,
        "focused and unfocused carets must differ in pixels"
    );
    assert!(cell_is_solid(&focused, 0, 0, cursor_rgb(&DARK)));

    // Hollow: the border ring is cursor-coloured, the interior is clear.
    for y in 0..CELL_HEIGHT {
        for x in 0..CELL_WIDTH {
            let pixel = unfocused.pixel(x, y);
            let rgb = [pixel[0], pixel[1], pixel[2]];
            let on_border =
                x < stroke || y < stroke || x >= CELL_WIDTH - stroke || y >= CELL_HEIGHT - stroke;
            assert_eq!(
                colors_match(rgb, cursor_rgb(&DARK)),
                on_border,
                "unfocused caret: pixel ({x},{y}) must be {} — a hollow ring of \
                 {stroke}px around a clear interior",
                if on_border {
                    "cursor-coloured"
                } else {
                    "clear"
                }
            );
        }
    }
}

/// A safe colour preference is the power-user half of the contract: on this
/// dark unstyled cell the configured orange clears 4.5:1 and therefore
/// replaces the inverse default without touching any other drawn colour.
#[test]
fn a_cursor_colour_override_replaces_only_the_caret_colour() {
    let Some(renderer) =
        renderer_or_skip("a_cursor_colour_override_replaces_only_the_caret_colour")
    else {
        return;
    };
    let orange: [f32; 3] = [255.0 / 255.0, 140.0 / 255.0, 0.0];
    let snap = snapshot(1, 4, b"a");
    let overridden = render_with_cursor_style(
        &renderer,
        &DARK,
        CursorStyle::theme_default(&DARK).with_color_override(Some(orange)),
        &snap,
    );
    let orange_u8 = orange.map(|channel| (channel * 255.0).round() as u8);
    assert!(
        cell_is_solid(&overridden, 0, 1, orange_u8),
        "the override colour {orange_u8:?} must draw the caret"
    );
    // The 'a' keeps the theme's default foreground.
    assert!(colors_match(
        cell_color(&overridden, 0, 0),
        default_foreground_rgb()
    ));
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
    // A one-column terminal leaves delayed autowrap armed with the cursor on
    // the glyph cell. Hide that cursor so these controls measure the ordinary
    // P/K glyph masks, not #206's inverse-video caret over those glyphs.
    let p_reference = render(&renderer, &snapshot(1, 1, b"\x1b[?25lP"));
    let k_reference = render(&renderer, &snapshot(1, 1, b"\x1b[?25lK"));
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

#[test]
fn scrollback_indicator_is_drawn_first_with_the_configured_return_chord() {
    let Some(renderer) =
        renderer_or_skip("scrollback_indicator_is_drawn_first_with_the_configured_return_chord")
    else {
        return;
    };
    const TERMINAL_COLS: u16 = 48;
    let config = AppConfig::parse("[keys]\nscroll_page_down = \"ctrl+j\"\n")
        .expect("custom history return chord is valid");
    let indicator = scrollback_indicator(7, config.keys()).expect("nonzero offset is visible");
    assert_eq!(indicator, "History -7 | Ctrl+J Latest");

    let empty_sidebar: &[String] = &[];
    let frame = renderer.capture_chrome(
        Target::new(
            &config.theme().palette(),
            (SIDEBAR_COLS as u32 + u32::from(TERMINAL_COLS)) * CELL_WIDTH,
            CELL_HEIGHT,
            poc_metrics(),
        ),
        None,
        FrameChrome::new(Some(empty_sidebar), None)
            .with_viewport_indicator(Some(indicator.as_str())),
    );

    let mut reference_bytes = b"\x1b[?25l".to_vec();
    reference_bytes.extend_from_slice(indicator.as_bytes());
    let reference = render(
        &renderer,
        &snapshot(1, TERMINAL_COLS, reference_bytes.as_slice()),
    );
    for col in 0..u32::from(TERMINAL_COLS) {
        assert_eq!(
            cell_pattern(&frame, 0, SIDEBAR_COLS as u32 + col),
            cell_pattern(&reference, 0, col),
            "viewport indicator differs from configured copy at column {col}"
        );
    }
    assert!(
        cell_is_lit(&frame, 0, SIDEBAR_COLS as u32),
        "the leading History indicator glyph was not drawn"
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

    // These are ordinary-glyph controls for cursor-free application chrome.
    // On a one-column terminal the delayed-wrap cursor occupies the glyph
    // cell, so hide it rather than comparing against inverse-video masks.
    let c_reference = render(&renderer, &snapshot(1, 1, b"\x1b[?25lC"));
    let n_reference = render(&renderer, &snapshot(1, 1, b"\x1b[?25lN"));
    let p_reference = render(&renderer, &snapshot(1, 1, b"\x1b[?25lP"));
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

/// Build one real session row through the domain model and production text
/// projection. No process is involved: the registry records an explicit
/// observation and the view projects it into the same line the app renders.
fn lifecycle_sidebar_row(status: SessionStatus) -> SidebarTextRow {
    let mut registry = SessionRegistry::new();
    let id = registry.create(SessionKind::Local);
    if status != SessionStatus::Starting {
        registry
            .observe(id, status)
            .expect("fixture lifecycle advances monotonically");
    }
    let entries = [SidebarEntry::Session(
        registry.get(id).expect("fixture id remains live"),
    )];
    let view = SidebarView::build(&entries, Some(id));
    visible_sidebar_text_rows_at_width(&view, 0, 1, SIDEBAR_COLS)
        .into_iter()
        .next()
        .expect("one session produces one sidebar row")
}

fn lifecycle_cases() -> [(SessionStatus, char, usize, [u8; 7]); 4] {
    [
        (
            SessionStatus::Starting,
            '⌛',
            3,
            [31, 27, 14, 4, 14, 27, 31],
        ),
        (SessionStatus::Running, '▶', 2, [8, 12, 14, 15, 14, 12, 8]),
        (
            SessionStatus::Exited { code: Some(0) },
            '■',
            8,
            [0, 14, 14, 14, 14, 14, 0],
        ),
        (
            SessionStatus::Failed {
                reason: "frame fixture".to_string(),
            },
            '✕',
            1,
            [17, 10, 4, 14, 4, 10, 17],
        ),
    ]
}

/// Build each kind through the real domain view and width-aware projection.
/// The four non-session kinds deliberately share identical identity/detail
/// text; project, SSH, and agent also share one lifecycle. Their frames can
/// therefore differ only in the fixed kind cell, while worktree additionally
/// differs in the intentionally blank lifecycle cell.
fn kind_sidebar_row(kind: EntryKind) -> SidebarTextRow {
    let entry = match kind {
        EntryKind::Project => SidebarEntry::Project {
            name: "same".to_owned(),
            root: "same-detail".to_owned(),
            lifecycle: SessionLifecycle::Exited,
        },
        EntryKind::Worktree => SidebarEntry::Worktree {
            name: "same".to_owned(),
            branch: "same-detail".to_owned(),
        },
        EntryKind::SshConnection => SidebarEntry::SshConnection {
            label: "same".to_owned(),
            host: "same-detail".to_owned(),
            selected: false,
            lifecycle: SessionLifecycle::Exited,
        },
        EntryKind::Agent => SidebarEntry::Agent {
            label: "same".to_owned(),
            status: "same-detail".to_owned(),
            lifecycle: SessionLifecycle::Exited,
        },
        EntryKind::Session => {
            let mut registry = SessionRegistry::new();
            let id = registry.create(SessionKind::Local);
            registry
                .observe(id, SessionStatus::Exited { code: Some(0) })
                .expect("fixture lifecycle advances monotonically");
            SidebarEntry::Session(registry.get(id).expect("fixture id remains live"))
        }
    };
    let view = SidebarView::build(&[entry], None);
    visible_sidebar_text_rows_at_width(&view, 0, 1, SIDEBAR_COLS)
        .into_iter()
        .next()
        .expect("one entry produces one sidebar row")
}

fn kind_cases() -> [(EntryKind, char, usize, [u8; 7]); 5] {
    [
        (EntryKind::Project, '◆', 13, [4, 14, 31, 27, 31, 14, 4]),
        (EntryKind::Worktree, '⑂', 2, [4, 5, 5, 7, 4, 20, 28]),
        (EntryKind::SshConnection, '⌁', 6, [0, 14, 17, 6, 12, 17, 14]),
        (EntryKind::Agent, '♟', 3, [4, 14, 21, 31, 21, 14, 10]),
        (EntryKind::Session, '▣', 7, [31, 17, 21, 19, 21, 17, 31]),
    ]
}

/// Rectangle fingerprint and colours emitted inside one sidebar cell by the
/// production vertex path. This is a frame oracle that needs no GPU adapter:
/// the same vertices feed the shipped shader and the offscreen pixel capture.
type GlyphRect = (u32, u32, u32, u32);
type SidebarCellVertexOracle = (Vec<GlyphRect>, Vec<[f32; 3]>);

/// Reconstruct the renderer's 5x7 bitmap from the rectangles emitted inside
/// one text cell. Keeping this decoder in the independent frame oracle makes
/// the expected row registration separate from the renderer's glyph table:
/// shifting a marker down while preserving its shape must fail here.
fn glyph_rows_from_rectangles(rectangles: &[GlyphRect]) -> [u8; 7] {
    const GLYPH_SCALE: u32 = 2;
    const GLYPH_TOP: u32 = 3;

    let mut rows = [0_u8; 7];
    for &(left, top, width, height) in rectangles {
        assert_eq!((width, height), (GLYPH_SCALE, GLYPH_SCALE));
        assert_eq!(left % GLYPH_SCALE, 0, "glyph pixel is off the x grid");
        assert!(top >= GLYPH_TOP, "glyph pixel is above the text inset");
        assert_eq!(
            (top - GLYPH_TOP) % GLYPH_SCALE,
            0,
            "glyph pixel is off the y grid"
        );

        let glyph_x = left / GLYPH_SCALE;
        let glyph_y = (top - GLYPH_TOP) / GLYPH_SCALE;
        assert!(glyph_x < 5, "glyph pixel exceeds the 5-column bitmap");
        assert!(glyph_y < 7, "glyph pixel exceeds the 7-row bitmap");
        rows[glyph_y as usize] |= 1 << (4 - glyph_x);
    }
    rows
}

fn sidebar_cell_vertex_oracle(
    row: &SidebarTextRow,
    theme: &Theme,
    column: u32,
) -> SidebarCellVertexOracle {
    let width = SIDEBAR_COLS as u32 * CELL_WIDTH;
    let height = CELL_HEIGHT;
    let vertices = glyph_vertices_for_sidebar_rows(
        Target::new(theme, width, height, poc_metrics()),
        None,
        Some(std::slice::from_ref(row)),
        None,
    );
    let pixel_x = |ndc: f32| (((ndc + 1.0) * 0.5 * width as f32).round()) as u32;
    let pixel_y = |ndc: f32| (((1.0 - ndc) * 0.5 * height as f32).round()) as u32;
    let mut signature = Vec::new();
    let mut colors = Vec::new();
    for rectangle in vertices.chunks_exact(6) {
        let left = pixel_x(rectangle[0].position[0]);
        if left / CELL_WIDTH != column {
            continue;
        }
        let top = pixel_y(rectangle[0].position[1]);
        let right = pixel_x(rectangle[2].position[0]);
        let bottom = pixel_y(rectangle[2].position[1]);
        signature.push((left - column * CELL_WIDTH, top, right - left, bottom - top));
        colors.push(rectangle[0].color);
    }
    (signature, colors)
}

#[test]
fn sidebar_kind_markers_are_identifiable_and_on_row_at_16_columns() {
    let mut signatures = Vec::new();
    for (kind, expected_marker, expected_ansi, expected_rows) in kind_cases() {
        let row = kind_sidebar_row(kind);
        assert_eq!(row.kind(), Some(kind));
        assert_eq!(row.text().chars().count(), SIDEBAR_COLS);
        assert_eq!(
            row.text().chars().nth(1),
            Some(expected_marker),
            "{kind:?} must own its literal in fixed visible column 2"
        );
        assert_eq!(
            row.text().chars().nth(2),
            Some(' '),
            "{kind:?} must retain the separator after its shape"
        );
        let expected_final = if kind == EntryKind::Worktree {
            ' '
        } else {
            '■'
        };
        assert_eq!(
            row.text().chars().nth(SIDEBAR_COLS - 1),
            Some(expected_final),
            "{kind:?} must retain its structured final lifecycle cell"
        );
        if kind != EntryKind::Session {
            assert_eq!(
                row.text().chars().skip(11).take(3).collect::<String>(),
                "...",
                "{kind:?} must keep the exact ellipsis pixels before its suffix"
            );
        }

        let (signature, _) = sidebar_cell_vertex_oracle(&row, &DARK, 1);
        assert!(!signature.is_empty(), "{kind:?} emitted no kind geometry");
        assert_eq!(
            glyph_rows_from_rectangles(&signature),
            expected_rows,
            "{kind:?} moved or changed pixels inside fixed column 2"
        );
        assert!(
            !signatures.contains(&signature),
            "{kind:?} rendered the same shape as another row kind"
        );
        signatures.push(signature);

        for theme in [DARK, LIGHT, HIGH_CONTRAST] {
            let (_, colors) = sidebar_cell_vertex_oracle(&row, &theme, 1);
            let expected = theme.ansi()[expected_ansi].map(|channel| f32::from(channel) / 255.0);
            assert!(
                !colors.is_empty() && colors.iter().all(|color| *color == expected),
                "{kind:?} did not use ANSI slot {expected_ansi} on {:?}: {colors:?}",
                theme.background_u8()
            );
            assert!(
                contrast_ratio(theme.ansi()[expected_ansi], theme.background_u8()) >= 4.5,
                "{kind:?} fails WCAG AA on {:?}",
                theme.background_u8()
            );
        }
    }
    assert_eq!(signatures.len(), 5, "all five kind shapes were checked");
}

#[test]
fn sidebar_kind_markers_reach_exact_distinct_frame_cells_at_16_columns() {
    let Some(renderer) =
        renderer_or_skip("sidebar_kind_markers_reach_exact_distinct_frame_cells_at_16_columns")
    else {
        return;
    };
    let width = SIDEBAR_COLS as u32 * CELL_WIDTH;
    let mut patterns = Vec::new();
    let mut frames = Vec::new();
    for (kind, expected_marker, expected_ansi, _) in kind_cases() {
        let row = kind_sidebar_row(kind);
        let frame = renderer.capture_sidebar_rows(
            Target::new(&DARK, width, CELL_HEIGHT, poc_metrics()),
            None,
            Some(std::slice::from_ref(&row)),
            None,
        );
        assert!(
            cell_is_lit(&frame, 0, 1),
            "{kind:?} marker {expected_marker:?} is not visible in fixed column 2"
        );
        assert!(
            colors_match(cell_color(&frame, 0, 1), DARK.ansi()[expected_ansi]),
            "{kind:?} marker pixels do not use ANSI slot {expected_ansi}"
        );
        let pattern = cell_pattern(&frame, 0, 1);
        assert!(
            !patterns.contains(&pattern),
            "{kind:?} has the same captured marker pixels as another kind"
        );
        patterns.push(pattern);
        frames.push(frame);
    }
    assert_eq!(patterns.len(), 5, "all five captured shapes were checked");

    // Project, SSH, and agent fixtures have byte-identical identity and state
    // cells, so the kind cell is the EXACT full-frame difference. Worktree
    // additionally owns a blank final state cell by design.
    assert_eq!(changed_cells(&frames[0], &frames[2], 1, 16), vec![(0, 1)]);
    assert_eq!(changed_cells(&frames[0], &frames[3], 1, 16), vec![(0, 1)]);
    assert_eq!(changed_cells(&frames[2], &frames[3], 1, 16), vec![(0, 1)]);
    assert_eq!(
        changed_cells(&frames[0], &frames[1], 1, 16),
        vec![(0, 1), (0, 15)]
    );
}

#[test]
fn session_lifecycle_markers_are_identifiable_and_on_row_at_16_columns() {
    let mut signatures = Vec::new();
    for (status, expected_marker, expected_ansi, expected_rows) in lifecycle_cases() {
        let row = lifecycle_sidebar_row(status);
        let line = row.text();
        assert_eq!(
            line.chars().count(),
            SIDEBAR_COLS,
            "the compact session row must end at the shipped sidebar edge: {line:?}"
        );
        assert_eq!(
            line.chars().nth(SIDEBAR_COLS - 1),
            Some(expected_marker),
            "each lifecycle owns an identifiable literal in visible column 16"
        );

        let (signature, _) = sidebar_cell_vertex_oracle(
            &row,
            &DARK,
            u32::try_from(SIDEBAR_COLS - 1).expect("sidebar width fits u32"),
        );
        assert!(
            !signature.is_empty(),
            "marker {expected_marker:?} emitted no geometry in visible column 16"
        );
        assert!(
            !signatures.contains(&signature),
            "marker {expected_marker:?} rendered the same shape as another lifecycle"
        );
        assert_eq!(
            glyph_rows_from_rectangles(&signature),
            expected_rows,
            "marker {expected_marker:?} moved within its cell"
        );
        signatures.push(signature);

        for theme in [DARK, LIGHT, HIGH_CONTRAST] {
            let (_, colors) = sidebar_cell_vertex_oracle(
                &row,
                &theme,
                u32::try_from(SIDEBAR_COLS - 1).expect("sidebar width fits u32"),
            );
            let expected = theme.ansi()[expected_ansi].map(|channel| f32::from(channel) / 255.0);
            assert!(
                colors.iter().all(|color| *color == expected),
                "marker {expected_marker:?} did not use ANSI slot {expected_ansi} on theme \
                 background {:?}: {colors:?}",
                theme.background_u8()
            );
            assert!(
                contrast_ratio(theme.ansi()[expected_ansi], theme.background_u8()) >= 4.5,
                "marker {expected_marker:?} fails WCAG AA on {:?}",
                theme.background_u8()
            );
        }
    }
    assert_eq!(
        signatures.len(),
        4,
        "all four lifecycle shapes were checked"
    );
}

#[test]
fn session_lifecycle_markers_reach_distinct_frame_pixels_at_16_columns() {
    let Some(renderer) =
        renderer_or_skip("session_lifecycle_markers_reach_distinct_frame_pixels_at_16_columns")
    else {
        return;
    };
    let width = SIDEBAR_COLS as u32 * CELL_WIDTH;
    let marker_column = u32::try_from(SIDEBAR_COLS - 1).expect("sidebar width fits u32");
    let mut patterns = Vec::new();
    for (status, expected_marker, expected_ansi, _) in lifecycle_cases() {
        let row = lifecycle_sidebar_row(status);
        let frame = renderer.capture_sidebar_rows(
            Target::new(&DARK, width, CELL_HEIGHT, poc_metrics()),
            None,
            Some(std::slice::from_ref(&row)),
            None,
        );
        assert!(
            cell_is_lit(&frame, 0, marker_column),
            "marker {expected_marker:?} is not visible in column 16"
        );
        assert!(
            colors_match(
                cell_color(&frame, 0, marker_column),
                DARK.ansi()[expected_ansi]
            ),
            "marker {expected_marker:?} pixel colour is not its semantic palette role"
        );
        let pattern = cell_pattern(&frame, 0, marker_column);
        assert!(
            !patterns.contains(&pattern),
            "marker {expected_marker:?} has the same frame pixels as another lifecycle"
        );
        patterns.push(pattern);
    }
    assert_eq!(patterns.len(), 4, "all four rendered markers were checked");
}

#[test]
fn worktree_text_shaped_like_running_keeps_default_vertex_color() {
    let entries = [SidebarEntry::Worktree {
        name: "aaaaaaaaa".to_string(),
        branch: "▶".to_string(),
    }];
    let view = SidebarView::build(&entries, None);
    let rows = visible_sidebar_text_rows_at_width(&view, 0, 1, SIDEBAR_COLS);
    let row = rows.first().expect("one worktree produces one sidebar row");
    assert_eq!(row.kind(), Some(EntryKind::Worktree));
    assert_eq!(row.lifecycle(), None);
    assert_eq!(
        row.text().chars().nth(SIDEBAR_COLS - 3),
        Some('▶'),
        "fixture must place marker-shaped user text at the identity boundary"
    );
    assert_eq!(row.text().chars().last(), Some(' '));

    let (_, colors) = sidebar_cell_vertex_oracle(
        row,
        &DARK,
        u32::try_from(SIDEBAR_COLS - 3).expect("sidebar width fits u32"),
    );
    let false_running_signal = DARK.ansi()[2].map(|channel| f32::from(channel) / 255.0);
    assert_ne!(DARK.foreground(), false_running_signal);
    assert!(
        !colors.is_empty(),
        "worktree's marker-shaped text emitted no vertices"
    );
    assert!(
        colors.iter().all(|color| *color == DARK.foreground()),
        "worktree text borrowed the running lifecycle colour: {colors:?}"
    );
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
/// The successful adapter-absence case needs `--nocapture` so libtest does not
/// suppress its skip notice before the parent can inspect stderr. The failing
/// device case deliberately keeps normal capture so libtest places its panic
/// report on stdout, preserving the two externally distinct contracts.
fn rerun_forced(test: &str, mode: &str) -> (bool, String, String) {
    let mut command = Command::new(std::env::current_exe().expect("locate the test binary"));
    command.arg("--exact").arg(test);
    if mode == "absent" {
        command.arg("--nocapture");
    }
    let output = command
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
