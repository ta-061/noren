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
//!   fixture classes (prompt, ASCII, UTF-8, control, scrolling).
//!
//! ## Faithfulness
//!
//! [`renderer_capture`] re-includes the shipped `renderer.rs` and draws with the
//! same shader + glyph vertex generation, so a vertex/glyph/grid defect in the
//! binary is caught here, not hidden behind a parallel implementation.
//!
//! ## Defects surfaced (reported, not fixed)
//!
//! The assertions below are written the way a *correct* renderer requires. Two
//! of them fail today and are reported as defects rather than weakened:
//!
//! - `lowercase_distinct_from_uppercase`: the bitmap font folds lower to upper
//!   case (`renderer.rs` `glyph_rows` uses `to_ascii_uppercase`), so `a` and
//!   `A` render identically.
//! - `non_ascii_glyph_is_not_the_question_mark`: every non-ASCII code point
//!   falls to the `?` default arm, so `日` renders as `?`.

#[path = "../src/renderer_capture.rs"]
mod renderer_capture;

use noren_app::{GridGeometry, POC_CELL_HEIGHT as CELL_HEIGHT, POC_CELL_WIDTH as CELL_WIDTH};
use noren_terminal::{TerminalSnapshot, TerminalState};
use renderer_capture::renderer_source::SIDEBAR_COLS;
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

/// A pixel counts as "background" when it is close to the clear colour. Glyph
/// pixels are drawn at the fragment shader's constant `0.80/0.92/0.82` (bright
/// green), the clear at `0.035/0.045/0.04` (near-black), so a brightness gate
/// is robust across minor driver rounding.
fn is_background(rgba: [u8; 4]) -> bool {
    rgba[0] < 48 && rgba[1] < 48 && rgba[2] < 48
}

/// Whether any lit (non-background) pixel falls inside cell `(row, col)`.
fn cell_is_lit(frame: &CapturedFrame, row: u32, col: u32) -> bool {
    let x0 = col * CELL_WIDTH;
    let y0 = row * CELL_HEIGHT;
    for y in y0..y0 + CELL_HEIGHT {
        for x in x0..x0 + CELL_WIDTH {
            if !is_background(frame.pixel(x, y)) {
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
            pattern.push(!is_background(
                frame.pixel(col * CELL_WIDTH + x, row * CELL_HEIGHT + y),
            ));
        }
    }
    pattern
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
// Defect tests: written the way a correct renderer requires. Both fail today;
// see the module docs and the report. They are NOT weakened to pass.
// ===========================================================================

#[test]
#[ignore = "known defect: the bitmap font case-folds, so 'a' and 'A' draw the \
            same glyph. Ignored rather than deleted or weakened — this assertion \
            is the executable specification of the fix. `cargo test -- --ignored` \
            confirms it still fails; remove this attribute when a real font \
            stack lands."]
fn lowercase_distinct_from_uppercase() {
    // DEFECT (reported): the bitmap font folds case via `to_ascii_uppercase`
    // (renderer.rs `glyph_rows`), so 'a' and 'A' produce identical pixels. A
    // correct terminal font must distinguish them.
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
