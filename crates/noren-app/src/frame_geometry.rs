//! Pure calculations shared by frame sizing and pointer hit testing.

use crate::NorenApp;
#[cfg(test)]
use crate::renderer;
use noren_app::MAX_RENDER_COLS;

impl NorenApp {
    /// Whether the permanent status chrome has enough room to own a row.
    pub(super) fn status_row_present(window_rows: u16) -> bool {
        window_rows > 1
    }

    /// Terminal rows available after reserving permanent application chrome.
    ///
    /// The PTY, terminal state, renderer, and pointer mapper must all agree on
    /// this value. A one-row window cannot reserve its only row for chrome;
    /// keeping one terminal row is safer than constructing an invalid zero-row
    /// PTY, so the status line is temporarily suppressed there.
    pub(super) fn content_terminal_rows(window_rows: u16) -> u16 {
        window_rows - u16::from(Self::status_row_present(window_rows))
    }
}

/// Terminal column count for a given window column count, reserving
/// [`renderer::SIDEBAR_COLS`] columns on the left for the sidebar and clamping
/// the remainder to the renderer's drawable budget. The PTY winsize, terminal
/// state's column count, and the renderer's drawn region all use this value so
/// they never disagree.
///
/// Reserve the sidebar first, then clamp the terminal to
/// `MAX_RENDER_COLS - SIDEBAR_COLS` (floored at one). The sidebar sits *inside*
/// the renderer's `MAX_RENDER_COLS` ceiling, so the terminal must never be told
/// it owns more columns than the renderer can draw beside the sidebar —
/// otherwise columns are clipped invisibly. `renderer::glyph_vertices` applies
/// the identical formula independently; the sidebar geometry test pins that the
/// two sites agree.
#[cfg(test)]
pub(super) fn terminal_cols(window_cols: u16) -> u16 {
    terminal_cols_at_width(window_cols, renderer::SIDEBAR_COLS)
}

/// Configured-width form used by the running application. Validation keeps
/// `sidebar_columns` below `MAX_RENDER_COLS`; the saturating conversion keeps
/// this pure seam total for direct tests as well.
pub(super) fn terminal_cols_at_width(window_cols: u16, sidebar_columns: usize) -> u16 {
    let sidebar = u16::try_from(sidebar_columns).unwrap_or(u16::MAX);
    let budget = MAX_RENDER_COLS.saturating_sub(sidebar).max(1);
    window_cols.saturating_sub(sidebar).clamp(1, budget)
}

/// Index of the cell row containing a non-negative pixel coordinate, or
/// `None` when the coordinate is not finite. The cast saturates on overflow,
/// and downstream clamping keeps any saturated index inside the grid.
pub(super) fn pixel_row_index(pixel: f64, cell_size: u32) -> Option<usize> {
    if !pixel.is_finite() {
        return None;
    }
    Some((pixel / f64::from(cell_size)) as usize)
}

/// Pixel width of the sidebar's left strip: `SIDEBAR_COLS` cell columns. The
/// terminal is drawn to the right of this edge, so a click at exactly this x is
/// the first terminal column.
#[cfg(test)]
pub(super) fn sidebar_pixel_width(cell_width: u32) -> f64 {
    sidebar_pixel_width_at_width(cell_width, renderer::SIDEBAR_COLS)
}

/// Configured-width form used by sidebar hit testing.
pub(super) fn sidebar_pixel_width_at_width(cell_width: u32, sidebar_columns: usize) -> f64 {
    let columns = u32::try_from(sidebar_columns).unwrap_or(u32::MAX);
    f64::from(columns.saturating_mul(cell_width))
}

/// Terminal cell column under pixel x, or `None` when the click lands in the
/// sidebar strip, on a non-finite coordinate, or past the grid. The sidebar
/// boundary is exclusive: x exactly at [`sidebar_pixel_width`] is the first
/// terminal column and maps to cell 0; anything strictly left of it is the
/// sidebar and is rejected.
#[cfg(test)]
pub(super) fn terminal_column_at(
    pixel_x: f64,
    terminal_cols: u16,
    cell_width: u32,
) -> Option<usize> {
    terminal_column_at_width(pixel_x, terminal_cols, cell_width, renderer::SIDEBAR_COLS)
}

/// Configured-width form used by terminal selection and mouse reporting.
pub(super) fn terminal_column_at_width(
    pixel_x: f64,
    terminal_cols: u16,
    cell_width: u32,
    sidebar_columns: usize,
) -> Option<usize> {
    let edge = sidebar_pixel_width_at_width(cell_width, sidebar_columns);
    if !pixel_x.is_finite() || pixel_x < edge {
        return None;
    }
    pixel_row_index(pixel_x - edge, cell_width)
        .map(|raw| raw.min(usize::from(terminal_cols).saturating_sub(1)))
}
