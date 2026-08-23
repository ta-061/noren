#[test]
fn pixel_row_index_truncates_and_rejects_non_finite() {
    assert_eq!(pixel_row_index(0.0, 20), Some(0));
    assert_eq!(pixel_row_index(39.0, 20), Some(1));
    assert_eq!(pixel_row_index(40.0, 20), Some(2));
    assert_eq!(pixel_row_index(f64::NAN, 20), None);
    assert_eq!(pixel_row_index(f64::INFINITY, 20), None);
}

// =========================================================================
// Sidebar geometry: the terminal width, the PTY winsize, and the renderer's
// drawn region must all agree once the sidebar reserves 16 columns.
// =========================================================================

/// Number of terminal cell columns the renderer drew, measured from its
/// vertex output rather than restating the column formula. Each terminal
/// column is fed a glyph the renderer lights starting at the cell's left
/// pixel edge (`B` lights glyph column 0), so a drawn column is detectable
/// as a glyph rect whose LEFT edge sits on that boundary. Scanning runs
/// rightward from the first terminal column (`SIDEBAR_COLS`) until a column
/// has no glyph — terminal content is contiguous, so the first gap marks
/// the end of the drawn region.
///
/// Matching a rect's left edge (its top-left corner) — not *any* vertex on
/// the boundary — is essential: a glyph's rightmost lit pixel column (e.g.
/// `B`, whose rows `17 = 0b10001` light glyph column 4) produces a rect
/// whose RIGHT edge lands exactly on the next column's left edge. Matching
/// arbitrary vertices would count that bleed as a drawn column and over-
/// count by one. Each rect is emitted as a 6-vertex fan whose first vertex
/// is its top-left corner, so the left edges are read from every 6th group.
fn rendered_terminal_columns(vertices: &[renderer::Vertex], width: u32, cell_width: u32) -> usize {
    let rect_lefts: Vec<f32> = vertices
        .chunks_exact(6)
        .map(|rect| rect[0].position[0])
        .collect();
    let mut drawn = 0;
    for col in renderer::SIDEBAR_COLS..usize::from(MAX_RENDER_COLS) {
        let edge = ((col as u32) * cell_width) as f32 / width as f32 * 2.0 - 1.0;
        if rect_lefts.iter().any(|left| (left - edge).abs() < 1e-5) {
            drawn += 1;
        } else {
            break;
        }
    }
    drawn
}

/// Drive the three terminal-width consumers — `TerminalState`'s stored
/// column count, the PTY winsize, and the columns the renderer actually
/// draws — at one window width and cell size, asserting they all agree on
/// `terminal_cols(window_cols)`. Asserting `terminal_cols() == window_cols -
/// 16` would merely restate the formula and pass any consistent-but-wrong
/// value; instead this exercises the three real consumers and is shared by
/// the swept agreement test below across every regime where they can drift
/// apart — including non-default cell sizes.
fn assert_three_consumers_agree_at(width: u32, metrics: CellMetrics) {
    let height = 600_u32;
    let cell_width = metrics.width();
    let cell_height = metrics.height();
    let window_cols = u16::try_from(width / cell_width).expect("fits in u16");
    let cols = terminal_cols(window_cols);

    // Consumer 1: the terminal state stores the sidebar-adjusted width.
    let rows = u16::try_from(height / cell_height).expect("fits in u16");
    let mut terminal = TerminalState::new(rows, cols).expect("valid terminal");
    terminal.feed_bytes(&vec![b'B'; usize::from(cols)]);
    let (_, term_cols) = terminal.size();
    assert_eq!(
        term_cols,
        cols,
        "at {width}px cell {}x{}: terminal must store \
             terminal_cols({window_cols}) = {cols}",
        metrics.width(),
        metrics.height(),
    );

    // Consumer 2: the PTY winsize carries the same column count.
    let pty = PtySize::from_raw(rows, cols).expect("valid pty size");
    assert_eq!(
        pty.cols(),
        cols,
        "at {width}px cell {}x{}: PTY winsize must agree",
        metrics.width(),
        metrics.height(),
    );

    // Consumer 3: the renderer draws exactly that many terminal columns —
    // measured from vertex output, independent of `terminal_cols`. The cell
    // metrics are threaded through so a renderer still drawing at the
    // compile-time default is exposed at a non-default size.
    let snapshot = terminal.snapshot();
    let sidebar: Vec<String> = Vec::new();
    let vertices = renderer::glyph_vertices(
        Some(&snapshot),
        Some(sidebar.as_slice()),
        None,
        width,
        height,
        metrics,
    );
    let drawn = rendered_terminal_columns(&vertices, width, cell_width);
    assert_eq!(
        drawn,
        usize::from(cols),
        "at {width}px cell {}x{} (window_cols={window_cols}): \
             renderer drew {drawn} terminal columns but terminal/PTY agree on {cols} — \
             the sidebar width is not consistently subtracted, the upper clamp is missing, \
             or the renderer ignored the configured cell size",
        metrics.width(),
        metrics.height(),
    );
}

/// The PR's headline property: the three consumers agree once the sidebar
/// reserves 16 columns — swept across the input range rather than pinned at
/// one width. A single point cannot support "agreement across the range",
/// and the original 900px point sits squarely inside the band (17..=160
/// columns) where the pre-fix geometry already agreed. Each swept width
/// targets a distinct regime so a regression of either pre-fix defect is
/// caught:
///   - 80px   (8 cols, below the sidebar width): the floor regime. A
///     renderer that floored at zero terminal columns while the terminal
///     and PTY held one is exposed.
///   - 900px  (90 cols, a typical window): the common case — the very width
///     a prior commit message wrongly cited as the divergence (both the
///     pre- and post-fix geometry agree at 74 here).
///   - 1600px (160 cols, exactly `MAX_RENDER_COLS`): the budget boundary,
///     where the terminal fills the whole drawable budget (144).
///   - 2000px (200 cols, above `MAX_RENDER_COLS`): the upper-clamp regime.
///     A `terminal_cols` with no upper clamp would claim 184 columns while
///     the renderer draws 144, silently clipping 40 columns of output.
///
/// This is a regression guard, not a reproduced bug: at the moment this
/// test was added all three consumers already agreed at every swept width,
/// and it exists to hold that line. Mutating `terminal_cols` to drop the
/// sidebar subtraction breaks the typical/above-max widths, and dropping
/// the upper clamp breaks the 2000px width.
///
/// The test is also swept across **cell sizes**. Issue #76: a configured
/// `cell_width = 20` flows to the geometry/PTY but the renderer ignored it,
/// drawing at the 10px compile-time constant. At 20px every width in the
/// sweep produces half the window_cols, so the renderer — if still on the
/// constant — would draw *twice* as many terminal columns as the terminal
/// and PTY agree on, and the three consumers diverge. This is the
/// acceptance criterion from the Issue.
#[test]
fn terminal_cols_pty_winsize_and_renderer_agree_across_the_width_range() {
    let poc = GridGeometry::poc().cell_metrics();
    for width in [80_u32, 900, 1600, 2000] {
        assert_three_consumers_agree_at(width, poc);
    }
    let big = GridGeometry::with_cells(20, 40)
        .expect("valid geometry")
        .cell_metrics();
    for width in [160_u32, 1800, 3200, 4000] {
        assert_three_consumers_agree_at(width, big);
    }
}

#[test]
fn terminal_rows_pty_winsize_and_renderer_agree_with_permanent_status_chrome() {
    for window_rows in [1_u16, 2, 30, noren_app::MAX_RENDER_ROWS] {
        let mut app = NorenApp {
            status: "Noren PoC ready",
            show_status: false,
            ..Default::default()
        };
        let metrics = app.geometry.cell_metrics();
        let height = u32::from(window_rows) * metrics.height();
        let grid = app
            .geometry
            .update(Resize::new(WINDOW_WIDTH, height))
            .expect("non-zero window grid");
        assert_eq!(grid.rows(), window_rows);

        // Drive the exact initialization seam used before PtySession::spawn
        // instead of rebuilding its dimensions inside the test.
        let pty = app
            .prepare_initial_terminal(grid)
            .expect("valid runtime grid");
        let terminal = app.terminal.as_ref().expect("terminal installed");
        let terminal_rows = terminal.size().0;
        let status = app.rendered_status_row(window_rows);
        let layout = renderer::FrameRowLayout::new(
            height,
            metrics,
            usize::from(terminal_rows),
            status.is_some(),
        )
        .expect("non-zero frame layout");

        assert_eq!(terminal.size().0, terminal_rows);
        assert_eq!(pty.rows(), terminal_rows);
        assert_eq!(layout.row_at(0), Some(renderer::FrameRow::Terminal(0)));
        assert_eq!(
            layout.row_at(usize::from(terminal_rows - 1)),
            Some(renderer::FrameRow::Terminal(usize::from(terminal_rows - 1)))
        );
        if window_rows == 1 {
            assert_eq!(status, None);
            assert_eq!(layout.rendered_rows(), 1);
        } else {
            assert!(status.is_some());
            assert_eq!(terminal_rows, window_rows - 1);
            assert_eq!(
                layout.row_at(usize::from(window_rows - 1)),
                Some(renderer::FrameRow::Status)
            );
            assert_eq!(layout.rendered_rows(), usize::from(window_rows));
        }
    }
}

/// MINOR-1: below ~160px the window fits inside the sidebar. `terminal_cols`
/// floors at one (the terminal/PTY reject zero columns); the renderer must
/// floor at the same one rather than drawing zero terminal columns while the
/// terminal still holds one. Drives the real renderer so the agreement is
/// measured, not assumed.
#[test]
fn terminal_cols_and_renderer_floor_at_one_below_the_sidebar() {
    let cell_width = GridGeometry::poc().cell_width();
    // A window exactly SIDEBAR_COLS wide: visible_cols == SIDEBAR_COLS, so
    // the terminal region has no room — both floors must keep it at one.
    let width = (renderer::SIDEBAR_COLS as u32) * cell_width;
    let height = 600_u32;
    let window_cols = u16::try_from(width / cell_width).expect("fits in u16");
    assert_eq!(window_cols, u16::try_from(renderer::SIDEBAR_COLS).unwrap());
    let cols = terminal_cols(window_cols);
    assert_eq!(cols, 1, "terminal_cols floors at one, never zero");

    let mut terminal = TerminalState::new(2, cols).expect("valid terminal");
    terminal.feed_bytes(&vec![b'B'; usize::from(cols)]);
    let snapshot = terminal.snapshot();
    let sidebar: Vec<String> = Vec::new();
    let vertices = renderer::glyph_vertices(
        Some(&snapshot),
        Some(sidebar.as_slice()),
        None,
        width,
        height,
        GridGeometry::poc().cell_metrics(),
    );
    let drawn = rendered_terminal_columns(&vertices, width, cell_width);
    assert_eq!(
        drawn,
        usize::from(cols),
        "renderer must draw the terminal's one column, not zero — the floor \
             disagrees with terminal_cols below the sidebar width"
    );
}

/// MINOR-3: `grid_point_at`'s sidebar boundary. A click in the last sidebar
/// column must be rejected, and a click at the first terminal column must
/// map to terminal cell 0. `grid_point_at` itself needs a live window this
/// harness cannot create, so this drives the extracted column mapper that
/// `grid_point_at` delegates to.
#[test]
fn terminal_column_at_rejects_the_sidebar_and_starts_the_terminal_at_zero() {
    let cols = 40_u16;
    let cell_width = GridGeometry::poc().cell_width();
    let sidebar_edge = sidebar_pixel_width(cell_width);

    // The last sidebar column — just inside the sidebar's right edge — does
    // not address the terminal grid.
    assert_eq!(
        terminal_column_at(sidebar_edge - 1.0, cols, cell_width),
        None,
        "a click in the last sidebar column must be rejected"
    );
    // The first terminal column, exactly at the sidebar's right edge, maps
    // to terminal cell 0.
    assert_eq!(
        terminal_column_at(sidebar_edge, cols, cell_width),
        Some(0),
        "the first terminal column must map to cell 0"
    );
    // One cell width further in lands in terminal cell 1.
    assert_eq!(
        terminal_column_at(sidebar_edge + f64::from(cell_width), cols, cell_width),
        Some(1)
    );
    // The last terminal column maps to the highest valid cell.
    assert_eq!(
        terminal_column_at(
            sidebar_edge + f64::from(cell_width) * f64::from(cols - 1),
            cols,
            cell_width
        ),
        Some(usize::from(cols - 1))
    );
    // A click past the last column clamps to the last cell, never overflows.
    assert_eq!(
        terminal_column_at(
            sidebar_edge + f64::from(cell_width) * f64::from(cols),
            cols,
            cell_width
        ),
        Some(usize::from(cols - 1))
    );
    // Negative and non-finite clicks are rejected.
    assert_eq!(terminal_column_at(-1.0, cols, cell_width), None);
    assert_eq!(terminal_column_at(f64::NAN, cols, cell_width), None);
}

/// Issue #76: at a non-default cell width, the sidebar's drawn pixel
/// boundary and the click-handling boundary must agree. The renderer draws
/// the terminal starting at column `SIDEBAR_COLS`; `sidebar_pixel_width`
/// and `terminal_column_at` must use the same `cell_width` to locate that
/// boundary, or clicks land in the wrong region.
///
/// At `cell_width = 20` the boundary is `16 * 20 = 320px`. If
/// `sidebar_pixel_width` still used `POC_CELL_WIDTH` (10), the boundary
/// would drift to 160px and clicks in the 160–320px strip would be
/// misattributed to the terminal instead of the sidebar.
#[test]
fn sidebar_boundary_and_click_boundary_agree_at_non_default_cell_width() {
    let cell_width = 20_u32;
    let cols = 40_u16;
    let sidebar_edge = sidebar_pixel_width(cell_width);

    // The drawn boundary is SIDEBAR_COLS * cell_width.
    assert_eq!(
        sidebar_edge,
        f64::from((renderer::SIDEBAR_COLS as u32) * cell_width),
        "sidebar pixel width must be SIDEBAR_COLS * cell_width"
    );
    assert_eq!(sidebar_edge, 320.0);

    // A click at the boundary maps to terminal column 0.
    assert_eq!(
        terminal_column_at(sidebar_edge, cols, cell_width),
        Some(0),
        "at cell_width=20, the first terminal column is at the sidebar edge"
    );
    // A click one pixel left of the boundary is still the sidebar.
    assert_eq!(
        terminal_column_at(sidebar_edge - 1.0, cols, cell_width),
        None,
        "at cell_width=20, a click just left of the boundary is the sidebar"
    );
    // One cell width further maps to column 1.
    assert_eq!(
        terminal_column_at(sidebar_edge + f64::from(cell_width), cols, cell_width),
        Some(1),
        "at cell_width=20, one cell past the boundary is column 1"
    );
    // The last terminal column maps to the highest valid cell.
    assert_eq!(
        terminal_column_at(
            sidebar_edge + f64::from(cell_width) * f64::from(cols - 1),
            cols,
            cell_width
        ),
        Some(usize::from(cols - 1)),
    );
}

// ── Sidebar offset and coordinate mapping ───────────────────────────

/// A click on the first terminal column (exactly at the sidebar edge)
/// reports column 1, not 17. This exercises the sidebar subtraction in
/// `terminal_column_at` through the encoder's 1-based conversion — if the
/// sidebar offset were dropped, the column would be 17 (16 sidebar cells
/// + 1).
#[test]
fn sidebar_offset_first_terminal_column_reports_col_1() {
    let cols = 40_u16;
    let cell_width = GridGeometry::poc().cell_width();
    let col = terminal_column_at(sidebar_pixel_width(cell_width), cols, cell_width)
        .expect("first terminal column must map to a cell");
    assert_eq!(col, 0, "sidebar offset: first terminal column = cell 0");

    let grid = MouseGrid::new(10, cols).expect("grid");
    let modes = MouseModes::disabled().with_normal(true).with_sgr(true);
    let event = PointerEvent::press(
        EncoderButton::Left,
        col as u32,
        0,
        PointerModifiers::empty(),
    );
    let bytes = MouseEncoder::encode(event, modes, grid).expect("must encode");
    let report = String::from_utf8(bytes).expect("SGR is ASCII");
    assert_eq!(
        report, "\x1b[<0;1;1M",
        "column must be 1 (sidebar offset applied), not 17"
    );
}

/// A click inside the sidebar produces no terminal column, hence no PTY
/// bytes can be constructed for it.
#[test]
fn sidebar_click_produces_no_terminal_column() {
    let cols = 40_u16;
    let cell_width = GridGeometry::poc().cell_width();
    let edge = sidebar_pixel_width(cell_width);
    assert_eq!(
        terminal_column_at(edge - 1.0, cols, cell_width),
        None,
        "last sidebar column must not map to a terminal cell"
    );
    assert_eq!(
        terminal_column_at(0.0, cols, cell_width),
        None,
        "leftmost pixel is sidebar"
    );
}
