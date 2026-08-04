//! Renderer-independent grid selection over the visible screen and scrollback.
//!
//! A [`Selection`] is a normalized, inclusive range over the terminal's rows:
//! retained scrollback rows first (oldest eviction order), then the visible
//! screen rows. It supports three granularities ([`SelectionMode`]):
//! character-wise, word-wise, and line-wise. The model is renderer- and
//! app-independent: capture and extraction take any [`SelectionGrid`], which
//! [`TerminalState`] and [`TerminalSnapshot`] both implement.
//!
//! # Wide characters
//!
//! A selection never splits a wide character. Endpoints that land on a
//! continuation cell snap onto the lead cell of the character that owns it, so
//! the whole character is included; extraction emits lead cells only and skips
//! continuation cells rather than emitting empty halves. CJK copy is therefore
//! byte-exact or absent, never corrupted.
//!
//! # Rows keep their own widths
//!
//! Scrollback rows are not reflowed by resize; each keeps the width it had
//! when it scrolled off. Column coordinates are clamped per row, so a
//! selection may span scrollback rows and visible rows of different widths
//! and still yield contiguous text.
//!
//! # Expiration: resize, scroll, and screen switches
//!
//! A selection is only meaningful against the exact grid it was captured on.
//! Every [`Selection`] records a [`GridStamp`] (rows, cols, scrollback length,
//! active screen); [`Selection::extract`] checks the stamp first and returns
//! an empty string on mismatch, so a stale selection can never silently yield
//! wrong text. Concretely:
//!
//! - **Resize** changes rows/cols, so the stamp mismatches and extraction
//!   yields `""`. Cell coordinates do not survive a reflow-free resize; the
//!   selection is invalidated rather than re-anchored.
//! - **Scrolling** that pushes lines off the top of the primary screen changes
//!   the scrollback length, so the stamp mismatches and the selection is
//!   invalidated. Content shifts that keep the stamp identical (in-place
//!   overwrites, `ESC M` scroll-down at the top) are NOT detected by the
//!   stamp; the selection owner must invalidate on every terminal state change
//!   (the app clears its selection on any PTY output and any resize).
//! - **Alternate screen**: while DEC 1049 selects the alternate screen the
//!   stamp mismatches and extraction yields `""`. Returning to the primary
//!   screen restores the stamp because 1049 leaves primary content untouched,
//!   so a pre-alt selection becomes extractable again; in practice the owner
//!   has already invalidated it when the 1049 bytes arrived as output.
//!
//! # Extraction shape
//!
//! Per row, lead-cell text is concatenated (continuations skipped) and
//! trailing blank columns are trimmed, matching the snapshot `lines` policy.
//! Rows are joined with `\n`; trailing blank-only rows are dropped and no
//! trailing newline is emitted. Character selections spanning several rows
//! include the rest of the first row and the start of the last row, like
//! classic terminal selection. A selection over only blank cells yields `""`.

use crate::{Cell, ScreenBuffer, TerminalSnapshot, TerminalState};

/// How a selection expands its endpoints when captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    /// Endpoints are snapped to character boundaries only.
    Char,
    /// Each endpoint expands to the full word containing it. A word is a
    /// maximal run of cells whose first character is alphanumeric or `_`;
    /// every other cell (blank or punctuation) separates words. Expansion
    /// stops at row edges and never crosses rows.
    Word,
    /// Every covered row is taken whole, from its first to its last cell.
    Line,
}

/// A grid coordinate addressed across scrollback and the visible screen.
///
/// `line` is absolute: retained scrollback rows come first in eviction order
/// (oldest at line 0), followed by the visible rows. `column` is zero-based
/// within that row's own width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridPoint {
    line: usize,
    column: usize,
}

impl GridPoint {
    /// Create a grid coordinate.
    #[must_use]
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    /// Absolute line index (scrollback first, then visible rows).
    #[must_use]
    pub const fn line(self) -> usize {
        self.line
    }

    /// Zero-based column within the line.
    #[must_use]
    pub const fn column(self) -> usize {
        self.column
    }
}

/// Immutable row-oriented grid view a [`Selection`] can be captured from and
/// extracted against.
///
/// Implemented by [`TerminalState`] (borrowed rows, no cloning) and
/// [`TerminalSnapshot`] (cloned rows). Selections never mutate the grid.
pub trait SelectionGrid {
    /// Visible row count.
    fn rows(&self) -> u16;

    /// Visible column count.
    fn cols(&self) -> u16;

    /// Number of retained scrollback rows.
    fn scrollback_len(&self) -> usize;

    /// Whether the alternate screen is currently selected.
    fn is_alternate_screen_active(&self) -> bool;

    /// Cells of one absolute line (scrollback first, then visible rows),
    /// each row at its own width.
    fn row_cells(&self, line: usize) -> Option<&[Cell]>;

    /// Total addressable lines: scrollback plus visible rows.
    fn total_lines(&self) -> usize {
        self.scrollback_len() + usize::from(self.rows())
    }
}

fn visible_row_cells(screen: &ScreenBuffer, row: usize) -> Option<&[Cell]> {
    let rows = usize::from(screen.rows());
    let cols = usize::from(screen.cols());
    if row >= rows {
        return None;
    }
    let cells = screen.cells();
    Some(&cells[row * cols..(row + 1) * cols])
}

impl SelectionGrid for TerminalState {
    fn rows(&self) -> u16 {
        self.size().0
    }

    fn cols(&self) -> u16 {
        self.size().1
    }

    fn scrollback_len(&self) -> usize {
        TerminalState::scrollback_len(self)
    }

    fn is_alternate_screen_active(&self) -> bool {
        self.modes().is_alternate_screen_active()
    }

    fn row_cells(&self, line: usize) -> Option<&[Cell]> {
        if let Some(row) = self.scrollback_row(line) {
            return Some(row);
        }
        visible_row_cells(self.screen(), line - TerminalState::scrollback_len(self))
    }
}

impl SelectionGrid for TerminalSnapshot {
    fn rows(&self) -> u16 {
        TerminalSnapshot::rows(self)
    }

    fn cols(&self) -> u16 {
        TerminalSnapshot::cols(self)
    }

    fn scrollback_len(&self) -> usize {
        self.scrollback().len()
    }

    fn is_alternate_screen_active(&self) -> bool {
        self.modes().is_alternate_screen_active()
    }

    fn row_cells(&self, line: usize) -> Option<&[Cell]> {
        let scrollback = self.scrollback();
        if line < scrollback.len() {
            return Some(&scrollback[line]);
        }
        visible_row_cells(self.screen(), line - scrollback.len())
    }
}

/// Structural identity of the grid at capture time.
///
/// Any mismatch means the selection's coordinates no longer address the same
/// content, so extraction refuses to run instead of yielding wrong text. See
/// the module docs for per-event behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GridStamp {
    rows: u16,
    cols: u16,
    scrollback_len: usize,
    alternate_screen: bool,
}

impl GridStamp {
    fn capture(grid: &impl SelectionGrid) -> Self {
        Self {
            rows: grid.rows(),
            cols: grid.cols(),
            scrollback_len: grid.scrollback_len(),
            alternate_screen: grid.is_alternate_screen_active(),
        }
    }

    fn matches(&self, grid: &impl SelectionGrid) -> bool {
        self.rows == grid.rows()
            && self.cols == grid.cols()
            && self.scrollback_len == grid.scrollback_len()
            && self.alternate_screen == grid.is_alternate_screen_active()
    }
}

/// A normalized, inclusive grid range captured from a [`SelectionGrid`].
///
/// Endpoints are clamped into the grid, snapped off continuation cells, and
/// ordered so [`Selection::start`] precedes [`Selection::end`] in reading
/// order. Word and line granularities expand the endpoints at capture time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    mode: SelectionMode,
    start: GridPoint,
    end: GridPoint,
    stamp: GridStamp,
}

impl Selection {
    /// Capture a selection between two pointer positions.
    ///
    /// Out-of-bounds points clamp to the grid; points on a wide character's
    /// continuation round to the character's lead so extraction never splits
    /// the character. The grid's dimensions, scrollback length, and active
    /// screen are recorded in a stamp checked by [`Selection::extract`].
    pub fn new(
        grid: &impl SelectionGrid,
        mode: SelectionMode,
        anchor: GridPoint,
        focus: GridPoint,
    ) -> Self {
        let stamp = GridStamp::capture(grid);
        let mut anchor = clamp_point(grid, anchor);
        let mut focus = clamp_point(grid, focus);
        anchor.column = snap_off_continuation(grid, anchor);
        focus.column = snap_off_continuation(grid, focus);
        if (focus.line, focus.column) < (anchor.line, anchor.column) {
            std::mem::swap(&mut anchor, &mut focus);
        }
        let (start, end) = match mode {
            SelectionMode::Char => (anchor, focus),
            SelectionMode::Word => {
                let (word_start, _) = word_bounds(grid, anchor);
                let (_, word_end) = word_bounds(grid, focus);
                (
                    GridPoint::new(anchor.line, word_start),
                    GridPoint::new(focus.line, word_end),
                )
            }
            SelectionMode::Line => (
                GridPoint::new(anchor.line, 0),
                GridPoint::new(focus.line, last_lead_column(grid, focus.line)),
            ),
        };
        Self {
            mode,
            start,
            end,
            stamp,
        }
    }

    /// Select every cell of the grid: oldest scrollback row through the last
    /// visible row, at the current dimensions.
    pub fn entire_grid(grid: &impl SelectionGrid) -> Self {
        let last_line = grid.total_lines().saturating_sub(1);
        Self::new(
            grid,
            SelectionMode::Line,
            GridPoint::new(0, 0),
            GridPoint::new(last_line, usize::MAX),
        )
    }

    /// Granularity applied when this selection was captured.
    #[must_use]
    pub const fn mode(&self) -> SelectionMode {
        self.mode
    }

    /// Normalized first point in reading order, on a character boundary.
    #[must_use]
    pub const fn start(&self) -> GridPoint {
        self.start
    }

    /// Normalized inclusive last point in reading order, on a character
    /// boundary.
    #[must_use]
    pub const fn end(&self) -> GridPoint {
        self.end
    }

    /// Whether the grid still matches the stamp captured with this selection.
    ///
    /// `false` means coordinates no longer address the captured content and
    /// [`Selection::extract`] yields an empty string.
    #[must_use]
    pub fn is_valid(&self, grid: &impl SelectionGrid) -> bool {
        self.stamp.matches(grid)
    }

    /// Extract the selected text, or `""` when the selection has expired.
    ///
    /// Never splits a wide character: continuation cells are skipped and
    /// endpoints were rounded to character boundaries at capture time. Never
    /// panics: all coordinates stay within the captured stamp's grid.
    pub fn extract(&self, grid: &impl SelectionGrid) -> String {
        if !self.is_valid(grid) {
            return String::new();
        }
        let mut lines: Vec<String> = Vec::new();
        for line in self.start.line..=self.end.line {
            let Some(cells) = grid.row_cells(line) else {
                continue;
            };
            let from = if line == self.start.line {
                self.start.column
            } else {
                0
            };
            let to = if line == self.end.line {
                self.end.column
            } else {
                usize::MAX
            };
            lines.push(extract_row(cells, from, to));
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }
}

fn extract_row(cells: &[Cell], from: usize, to: usize) -> String {
    let mut line = String::new();
    for (column, cell) in cells.iter().enumerate() {
        if column < from || column > to {
            continue;
        }
        // Continuation cells render as nothing and never own text; skipping
        // them (instead of emitting their empty text) is what keeps wide
        // characters whole even if an endpoint were mis-snapped.
        if cell.is_continuation() {
            continue;
        }
        line.push_str(cell.text());
    }
    while line.ends_with(' ') {
        line.pop();
    }
    line
}

fn clamp_point(grid: &impl SelectionGrid, point: GridPoint) -> GridPoint {
    let last_line = grid.total_lines().saturating_sub(1);
    let line = point.line.min(last_line);
    let width = grid.row_cells(line).map(<[Cell]>::len).unwrap_or(1);
    let column = point.column.min(width.saturating_sub(1));
    GridPoint::new(line, column)
}

fn snap_off_continuation(grid: &impl SelectionGrid, point: GridPoint) -> usize {
    let Some(cells) = grid.row_cells(point.line) else {
        return point.column;
    };
    let mut column = point.column;
    // The grid invariant pairs every continuation directly with its lead, so
    // stepping back at most once suffices; the loop keeps this defensive.
    while column > 0 && cells.get(column).is_some_and(Cell::is_continuation) {
        column -= 1;
    }
    column
}

fn head_cells(cells: &[Cell]) -> Vec<(usize, &Cell)> {
    cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| !cell.is_continuation())
        .collect()
}

fn is_word_cell(cell: &Cell) -> bool {
    cell.text()
        .chars()
        .next()
        .is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
}

/// Bounds of the word containing `point`, as `(lead column of first word
/// cell, lead column of last word cell)`. A non-word cell selects itself.
fn word_bounds(grid: &impl SelectionGrid, point: GridPoint) -> (usize, usize) {
    let Some(cells) = grid.row_cells(point.line) else {
        return (point.column, point.column);
    };
    let heads = head_cells(cells);
    let Some(index) = heads
        .iter()
        .rposition(|(column, _)| *column <= point.column)
    else {
        return (point.column, point.column);
    };
    if !is_word_cell(heads[index].1) {
        return (heads[index].0, heads[index].0);
    }
    let mut low = index;
    let mut high = index;
    while low > 0 && is_word_cell(heads[low - 1].1) {
        low -= 1;
    }
    while high + 1 < heads.len() && is_word_cell(heads[high + 1].1) {
        high += 1;
    }
    (heads[low].0, heads[high].0)
}

fn last_lead_column(grid: &impl SelectionGrid, line: usize) -> usize {
    let Some(cells) = grid.row_cells(line) else {
        return 0;
    };
    let mut column = cells.len().saturating_sub(1);
    while column > 0 && cells[column].is_continuation() {
        column -= 1;
    }
    column
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: u16, cols: u16, bytes: &[u8]) -> TerminalState {
        let mut state = TerminalState::new(rows, cols).expect("valid test terminal");
        state.feed_bytes(bytes);
        state
    }

    #[test]
    fn char_selection_normalizes_reversed_endpoints() {
        let state = grid(1, 6, b"abc");
        let forward = Selection::new(
            &state,
            SelectionMode::Char,
            GridPoint::new(0, 0),
            GridPoint::new(0, 2),
        );
        let reversed = Selection::new(
            &state,
            SelectionMode::Char,
            GridPoint::new(0, 2),
            GridPoint::new(0, 0),
        );
        assert_eq!(forward, reversed);
        assert_eq!(forward.start(), GridPoint::new(0, 0));
        assert_eq!(forward.end(), GridPoint::new(0, 2));
        assert_eq!(forward.extract(&state), "abc");
    }

    #[test]
    fn out_of_bounds_points_clamp_instead_of_panicking() {
        let state = grid(2, 3, b"abc\r\ndef");
        let selection = Selection::new(
            &state,
            SelectionMode::Char,
            GridPoint::new(usize::MAX, usize::MAX),
            GridPoint::new(usize::MAX, usize::MAX),
        );
        assert_eq!(selection.start(), GridPoint::new(1, 2));
        assert_eq!(selection.extract(&state), "f");
    }
}
