//! Renderer-independent literal search over the visible screen **and** the
//! bounded scrollback.
//!
//! This primitive answers "where does this text appear on the grid?" without
//! depending on any renderer. It operates on a [`TerminalSnapshot`] via the
//! [`logical_row`](TerminalSnapshot::logical_row) accessor, so it never copies
//! history into a rectangle and never grows allocation with the past.
//!
//! # Scope and non-goals
//!
//! - **Literal substring only.** There is no regex, no glob, no shell pattern.
//! - **Per-row matching.** A match never spans a wrapped-line boundary: each
//!   logical row (scrollback or visible) is searched independently. Wrapped
//!   text occupies several independent rows, and scrollback rows do not reflow
//!   and each keeps the width it had when it scrolled off, so the grid is a
//!   ragged array rather than a rectangle. Treating rows independently is the
//!   predictable, well-defined choice.
//! - **Wide characters.** Matches report **display columns**, not byte or
//!   character counts, so renderer highlighting lines up with the grid. A
//!   two-column character contributes its lead column plus its continuation
//!   column; continuation cells are not independently matchable (their text is
//!   empty).
//! - **Case-insensitive matching is ASCII-only.** Lower-casing is byte-wise
//!   via `to_ascii_lowercase`; non-ASCII letters are compared byte-for-byte
//!   even in case-insensitive mode. Full Unicode case folding would require
//!   case-folding tables and is deliberately out of scope.
//! - **Empty needle.** An empty needle matches nothing: [`Search::count`]
//!   returns `0` and every iterator is empty. This is the defined behavior,
//!   not undefined.
//!
//! # Complexity
//!
//! Let `R` be the number of logical rows, `C` the width of the widest row, and
//! `M` the number of matches. A full scan runs in `O(sum of row widths + M)`
//! time with a naive substring scan over each row. **Auxiliary space is
//! `O(C + M)`**: one row scratch buffer of capacity `C` is reused across rows,
//! and collected results grow with `M`. No allocation is proportional to
//! history size — scanning a 10,000-row scrollback with zero matches allocates
//! only the fixed row scratch. [`Search::count`] does not allocate for matches
//! at all.

use crate::state::{Cell, TerminalSnapshot};

/// Case-sensitivity policy for [`Search`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaseSensitivity {
    /// Match bytes exactly.
    Sensitive,
    /// ASCII-only case folding: bytes are lower-cased with
    /// [`u8::to_ascii_lowercase`] on both sides before comparison. Non-ASCII
    /// bytes are compared verbatim.
    InsensitiveAscii,
}

/// One match on the logical grid.
///
/// `row` is a logical row index in the order exposed by
/// [`TerminalSnapshot::logical_row`]: scrollback rows first (oldest first),
/// then visible rows top-to-bottom. `start_col` and `end_col` are **display
/// columns** with `end_col` exclusive, so a renderer highlights cells in
/// `[start_col, end_col)`. Wide characters contribute both their lead and
/// continuation columns, so `end_col - start_col` is the match's display width
/// rather than its character count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchMatch {
    /// Logical row index (scrollback-first, then visible).
    pub row: u32,
    /// First display column, inclusive.
    pub start_col: u16,
    /// One past the last display column (exclusive).
    pub end_col: u16,
}

/// A cursor position used to anchor directional search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchPosition {
    /// Logical row index (scrollback-first, then visible).
    pub row: u32,
    /// Display column.
    pub col: u16,
}

impl SearchPosition {
    /// Construct a position from a logical row and display column.
    #[must_use]
    pub const fn new(row: u32, col: u16) -> Self {
        Self { row, col }
    }
}

/// Iteration direction for [`Search::iter_from`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchDirection {
    /// Forward: increasing row, then increasing column.
    Forward,
    /// Backward: decreasing row, then decreasing column.
    Backward,
}

/// A configured literal search over a snapshot.
///
/// Cheap to construct: the needle is lower-cased once for ASCII-insensitive
/// mode and otherwise borrowed verbatim. All scanning is deferred to the
/// query methods.
pub struct Search<'a> {
    snapshot: &'a TerminalSnapshot,
    needle_bytes: Vec<u8>,
    case: CaseSensitivity,
}

impl<'a> Search<'a> {
    /// Construct a literal search of `snapshot` for `needle`.
    ///
    /// An empty `needle` is allowed and matches nothing (see the module docs).
    #[must_use]
    pub fn new(snapshot: &'a TerminalSnapshot, needle: &str, case: CaseSensitivity) -> Self {
        let needle_bytes = match case {
            CaseSensitivity::Sensitive => needle.as_bytes().to_vec(),
            CaseSensitivity::InsensitiveAscii => needle
                .as_bytes()
                .iter()
                .map(|b| b.to_ascii_lowercase())
                .collect(),
        };
        Self {
            snapshot,
            needle_bytes,
            case,
        }
    }

    /// Whether the needle is empty (and therefore matches nothing).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.needle_bytes.is_empty()
    }

    /// Total number of matches across scrollback and the visible screen.
    ///
    /// Performs a full bounded scan without allocating any [`SearchMatch`].
    #[must_use]
    pub fn count(&self) -> usize {
        if self.needle_bytes.is_empty() {
            return 0;
        }
        let mut row_text = RowText::default();
        let mut bucket = Vec::new();
        let mut count = 0;
        let total = self.snapshot.logical_row_count();
        for row in 0..total {
            if let Some(cells) = self.snapshot.logical_row(row) {
                scan_row(
                    cells,
                    &self.needle_bytes,
                    self.case,
                    row,
                    &mut row_text,
                    &mut bucket,
                );
                count += bucket.len();
            }
        }
        count
    }

    /// Collect every match in forward grid order (oldest scrollback row first,
    /// left-to-right within each row).
    ///
    /// Allocates `O(matches)`. The returned vector has zero capacity when there
    /// are no matches.
    #[must_use]
    pub fn all(&self) -> Vec<SearchMatch> {
        let mut result = Vec::new();
        if self.needle_bytes.is_empty() {
            return result;
        }
        let mut row_text = RowText::default();
        let mut bucket = Vec::new();
        let total = self.snapshot.logical_row_count();
        for row in 0..total {
            if let Some(cells) = self.snapshot.logical_row(row) {
                scan_row(
                    cells,
                    &self.needle_bytes,
                    self.case,
                    row,
                    &mut row_text,
                    &mut bucket,
                );
                result.extend_from_slice(&bucket);
            }
        }
        result
    }

    /// First match in forward grid order, or `None` if there is no match.
    ///
    /// Stops at the earliest match without scanning the rest of the grid.
    #[must_use]
    pub fn first(&self) -> Option<SearchMatch> {
        if self.needle_bytes.is_empty() {
            return None;
        }
        let mut row_text = RowText::default();
        let mut bucket = Vec::new();
        let total = self.snapshot.logical_row_count();
        for row in 0..total {
            if let Some(cells) = self.snapshot.logical_row(row) {
                scan_row(
                    cells,
                    &self.needle_bytes,
                    self.case,
                    row,
                    &mut row_text,
                    &mut bucket,
                );
                if let Some(m) = bucket.first() {
                    return Some(*m);
                }
            }
        }
        None
    }

    /// Last match in forward grid order, or `None` if there is no match.
    #[must_use]
    pub fn last(&self) -> Option<SearchMatch> {
        if self.needle_bytes.is_empty() {
            return None;
        }
        let mut row_text = RowText::default();
        let mut bucket = Vec::new();
        let mut last: Option<SearchMatch> = None;
        let total = self.snapshot.logical_row_count();
        for row in 0..total {
            if let Some(cells) = self.snapshot.logical_row(row) {
                scan_row(
                    cells,
                    &self.needle_bytes,
                    self.case,
                    row,
                    &mut row_text,
                    &mut bucket,
                );
                if let Some(m) = bucket.last() {
                    last = Some(*m);
                }
            }
        }
        last
    }

    /// Iterate every match in forward grid order.
    ///
    /// Equivalent to [`Search::iter_from`] anchored at the top-left corner.
    pub fn iter(&self) -> SearchIter<'_> {
        self.iter_from(SearchPosition::new(0, 0), SearchDirection::Forward)
    }

    /// Iterate matches from a position, with wrap-around.
    ///
    /// Forward iteration yields matches at or after `pos` in grid order, then
    /// wraps to the top and yields matches before `pos`; each match is yielded
    /// exactly once. Backward iteration yields matches at or before `pos` in
    /// reverse grid order, then wraps to the bottom and yields matches after
    /// `pos`, again once each.
    ///
    /// The iterator is lazy: it scans one row at a time into a fixed scratch
    /// buffer, so the only allocation is the row scratch (`O(max_row_width)`,
    /// amortized) — never a copy of history.
    pub fn iter_from(&self, pos: SearchPosition, dir: SearchDirection) -> SearchIter<'_> {
        let total_rows = self.snapshot.logical_row_count();
        let done = self.needle_bytes.is_empty() || total_rows == 0 || pos.row >= total_rows;
        let start_row = if pos.row >= total_rows {
            total_rows.saturating_sub(1)
        } else {
            pos.row
        };
        SearchIter {
            search: self,
            dir,
            start_row,
            start_col: pos.col,
            total_rows,
            phase: 0,
            row: start_row,
            pending: Vec::new(),
            cursor: 0,
            row_text: RowText::default(),
            loaded: false,
            done,
        }
    }
}

/// Reusable scratch encoding of one logical row as searchable bytes plus the
/// byte-offset-to-column map needed to recover display columns.
#[derive(Default)]
struct RowText {
    /// Concatenated, possibly case-folded cell text.
    bytes: Vec<u8>,
    /// `cell_start[i]` is the byte offset where cell `i` begins; the trailing
    /// sentinel `cell_start[cells.len()]` equals `bytes.len()`.
    cell_start: Vec<u32>,
    /// Display width of each cell, parallel to the cells (no sentinel).
    cell_width: Vec<u8>,
}

impl RowText {
    fn build(&mut self, cells: &[Cell], case: CaseSensitivity) {
        self.bytes.clear();
        self.cell_start.clear();
        self.cell_width.clear();
        // Reserve once per row; capacity stabilises at the widest row.
        self.cell_start.reserve(cells.len() + 1);
        self.cell_width.reserve(cells.len());
        for cell in cells {
            self.cell_start
                .push(u32::try_from(self.bytes.len()).unwrap_or(u32::MAX));
            self.cell_width.push(cell.width());
            match case {
                CaseSensitivity::Sensitive => self.bytes.extend_from_slice(cell.text().as_bytes()),
                CaseSensitivity::InsensitiveAscii => {
                    for &b in cell.text().as_bytes() {
                        self.bytes.push(b.to_ascii_lowercase());
                    }
                }
            }
        }
        self.cell_start
            .push(u32::try_from(self.bytes.len()).unwrap_or(u32::MAX));
    }

    /// Index of the cell containing byte offset `b` (largest `i` with
    /// `cell_start[i] <= b`). Valid for `b < bytes.len()`.
    fn cell_at_byte(&self, b: usize) -> usize {
        let target = u32::try_from(b).unwrap_or(u32::MAX);
        // partition_point returns the first index whose cell_start > target.
        let pp = self.cell_start.partition_point(|&x| x <= target);
        // pp >= 1 because cell_start[0] == 0 <= target.
        pp - 1
    }

    /// Index of the last cell that contributed bytes before exclusive end `e`
    /// (largest `j` with `cell_start[j] < e`).
    fn cell_strictly_before(&self, e: usize) -> usize {
        let target = u32::try_from(e).unwrap_or(u32::MAX);
        let pp = self.cell_start.partition_point(|&x| x < target);
        // pp >= 1 because cell_start[0] == 0 < e for any non-empty match.
        pp - 1
    }
}

/// Scan one row's cells for every occurrence of `needle`, appending into
/// `bucket` (which is cleared first). Overlapping occurrences are reported
/// (e.g. `aa` in `aaa` matches at columns 0 and 1).
fn scan_row(
    cells: &[Cell],
    needle: &[u8],
    case: CaseSensitivity,
    row: u32,
    row_text: &mut RowText,
    bucket: &mut Vec<SearchMatch>,
) {
    bucket.clear();
    let nb = needle.len();
    if nb == 0 {
        return;
    }
    row_text.build(cells, case);
    let len = row_text.bytes.len();
    if len < nb {
        return;
    }
    let mut start = 0;
    while start + nb <= len {
        if &row_text.bytes[start..start + nb] == needle {
            let start_cell = row_text.cell_at_byte(start);
            let end_cell = row_text.cell_strictly_before(start + nb);
            // Cell index doubles as display column (wide chars occupy one
            // index per column slot, continuations included).
            let start_col = u16::try_from(start_cell).unwrap_or(u16::MAX);
            let end_col = u16::try_from(end_cell)
                .unwrap_or(u16::MAX)
                .saturating_add(u16::from(row_text.cell_width[end_cell]));
            bucket.push(SearchMatch {
                row,
                start_col,
                end_col,
            });
        }
        // Advance one byte so overlapping candidates are all reported. A
        // valid-UTF-8 needle simply cannot match at a mid-codepoint offset, so
        // this never produces spurious matches.
        start += 1;
    }
}

/// Lazy cyclic iterator produced by [`Search::iter_from`].
pub struct SearchIter<'a> {
    search: &'a Search<'a>,
    dir: SearchDirection,
    start_row: u32,
    start_col: u16,
    total_rows: u32,
    phase: u8,
    row: u32,
    pending: Vec<SearchMatch>,
    cursor: usize,
    row_text: RowText,
    loaded: bool,
    done: bool,
}

/// Filter predicate selecting which matches a `(direction, phase)` visit keeps
/// from a freshly scanned row. Free-standing so the iterator can call it from a
/// closure without borrowing `self` while it also mutates the pending buffer.
fn keeps_match(
    dir: SearchDirection,
    phase: u8,
    start_row: u32,
    start_col: u16,
    m: &SearchMatch,
) -> bool {
    if m.row != start_row {
        return true;
    }
    match (dir, phase) {
        (SearchDirection::Forward, 0) => m.start_col >= start_col,
        (SearchDirection::Forward, 1) => m.start_col < start_col,
        (SearchDirection::Backward, 0) => m.start_col <= start_col,
        (SearchDirection::Backward, 1) => m.start_col > start_col,
        _ => true,
    }
}

impl SearchIter<'_> {
    fn load_current_row(&mut self) {
        self.pending.clear();
        self.cursor = 0;
        let row = self.row;
        let dir = self.dir;
        let phase = self.phase;
        let start_row = self.start_row;
        let start_col = self.start_col;
        if let Some(cells) = self.search.snapshot.logical_row(row) {
            let mut bucket = Vec::new();
            scan_row(
                cells,
                &self.search.needle_bytes,
                self.search.case,
                row,
                &mut self.row_text,
                &mut bucket,
            );
            self.pending.extend(
                bucket
                    .into_iter()
                    .filter(|m| keeps_match(dir, phase, start_row, start_col, m)),
            );
        }
    }

    fn take_pending(&mut self) -> Option<SearchMatch> {
        if self.cursor >= self.pending.len() {
            return None;
        }
        let item = match self.dir {
            // Forward: yield in stored (ascending column) order.
            SearchDirection::Forward => self.pending[self.cursor],
            // Backward: yield in descending column order.
            SearchDirection::Backward => self.pending[self.pending.len() - 1 - self.cursor],
        };
        self.cursor += 1;
        Some(item)
    }

    fn advance_row(&mut self) {
        match self.dir {
            SearchDirection::Forward => match self.phase {
                0 => {
                    if self.row + 1 >= self.total_rows {
                        self.phase = 1;
                        self.row = 0;
                    } else {
                        self.row += 1;
                    }
                }
                _ => {
                    if self.row == self.start_row {
                        self.done = true;
                    } else {
                        self.row += 1;
                    }
                }
            },
            SearchDirection::Backward => match self.phase {
                0 => {
                    if self.row == 0 {
                        self.phase = 1;
                        self.row = self.total_rows.saturating_sub(1);
                    } else {
                        self.row -= 1;
                    }
                }
                _ => {
                    if self.row == self.start_row {
                        self.done = true;
                    } else if self.row > 0 {
                        self.row -= 1;
                    } else {
                        self.done = true;
                    }
                }
            },
        }
    }
}

impl Iterator for SearchIter<'_> {
    type Item = SearchMatch;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        loop {
            if !self.loaded {
                self.load_current_row();
                self.loaded = true;
            }
            if let Some(m) = self.take_pending() {
                return Some(m);
            }
            self.advance_row();
            if self.done {
                return None;
            }
            self.loaded = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TerminalState;

    /// Build a terminal, feed it bytes, then return a snapshot.
    fn snapshot_after(rows: u16, cols: u16, bytes: &[u8]) -> TerminalSnapshot {
        let mut state = TerminalState::new(rows, cols).expect("valid terminal");
        state.feed_bytes(bytes);
        state.snapshot()
    }

    #[test]
    fn matches_in_visible_rows_are_reported_in_order() {
        let snap = snapshot_after(3, 12, b"alpha\r\nbeta alpha\r\ngamma");
        let search = Search::new(&snap, "alpha", CaseSensitivity::Sensitive);
        let matches = search.all();
        assert_eq!(matches.len(), 2);
        // Scrollback is empty; visible rows start at logical row 0.
        assert_eq!(
            matches[0],
            SearchMatch {
                row: 0,
                start_col: 0,
                end_col: 5
            }
        );
        assert_eq!(
            matches[1],
            SearchMatch {
                row: 1,
                start_col: 5,
                end_col: 10
            }
        );
        assert_eq!(search.count(), 2);
    }

    #[test]
    fn matches_in_scrollback_use_scrollback_first_row_indices() {
        // Two-row terminal; fill, scroll, then add a final matching row.
        let mut state = TerminalState::new(2, 8).expect("valid terminal");
        state.feed_bytes(b"foo bar\r\nfoo baz\r\nfoo end");
        let snap = state.snapshot();
        assert!(snap.scrollback().len() == 1, "expected one scrollback row");

        let search = Search::new(&snap, "foo", CaseSensitivity::Sensitive);
        let matches = search.all();
        // Scrollback row first (logical row 0), then visible rows.
        let rows: Vec<u32> = matches.iter().map(|m| m.row).collect();
        assert_eq!(rows, vec![0, 1, 2]);
        assert_eq!(matches[0].start_col, 0);
        assert_eq!(matches[0].end_col, 3);
        assert_eq!(search.count(), 3);
    }

    #[test]
    fn matches_spanning_scrollback_and_visible_keep_grid_order() {
        // Two-row grid; four lines means two scroll off and two stay visible.
        let mut state = TerminalState::new(2, 6).expect("valid terminal");
        state.feed_bytes(b"abc123\r\nXYZ123\r\nabc123\r\ndef123");
        let snap = state.snapshot();
        assert_eq!(snap.scrollback().len(), 2);

        let search = Search::new(&snap, "123", CaseSensitivity::Sensitive);
        let matches = search.all();
        let rows: Vec<u32> = matches.iter().map(|m| m.row).collect();
        assert_eq!(rows, vec![0, 1, 2, 3]);
        for m in &matches {
            assert_eq!(m.start_col, 3);
            assert_eq!(m.end_col, 6);
        }
    }

    #[test]
    fn case_insensitive_matches_ascii_only() {
        let snap = snapshot_after(1, 12, b"HeLLo hello");
        let sensitive = Search::new(&snap, "hello", CaseSensitivity::Sensitive);
        let insensitive = Search::new(&snap, "hello", CaseSensitivity::InsensitiveAscii);
        assert_eq!(sensitive.count(), 1);
        assert_eq!(insensitive.count(), 2);

        // ASCII-folding is symmetric on the needle side.
        let upper = Search::new(&snap, "HELLO", CaseSensitivity::InsensitiveAscii);
        assert_eq!(upper.count(), 2);

        // Non-ASCII letters are NOT folded: Cyrillic "Ш" keeps its bytes.
        let cyrillic = snapshot_after(1, 6, "ШШ".as_bytes());
        let folded = Search::new(&cyrillic, "ш", CaseSensitivity::InsensitiveAscii);
        assert_eq!(
            folded.count(),
            0,
            "Unicode case folding is out of scope; only ASCII is folded"
        );
    }

    #[test]
    fn wide_characters_report_display_columns_not_byte_counts() {
        // Grid: a | 日(lead) | 日(cont) | b -> "a日b" occupies columns 0..4.
        let snap = snapshot_after(1, 8, "a日b".as_bytes());
        let search = Search::new(&snap, "a日", CaseSensitivity::Sensitive);
        let matches = search.all();
        assert_eq!(matches.len(), 1);
        // Display columns 0..3 cover 'a' (1) plus the wide '日' (2).
        assert_eq!(
            matches[0],
            SearchMatch {
                row: 0,
                start_col: 0,
                end_col: 3
            }
        );

        let mid = Search::new(&snap, "日", CaseSensitivity::Sensitive);
        let m = mid.first().expect("wide char matches");
        assert_eq!(m.start_col, 1);
        assert_eq!(m.end_col, 3);

        let tail = Search::new(&snap, "日b", CaseSensitivity::Sensitive);
        let m = tail.first().expect("spans wide + narrow");
        assert_eq!(m.start_col, 1);
        assert_eq!(m.end_col, 4);

        // A continuation cell is never an independent match start.
        let cont = Search::new(&snap, "\u{0}", CaseSensitivity::Sensitive);
        assert_eq!(cont.count(), 0);
    }

    #[test]
    fn forward_backward_and_wrap_around_iteration() {
        // Three matches across two visible rows.
        let snap = snapshot_after(2, 12, b"x foo y foo\r\nfoo z");
        let search = Search::new(&snap, "foo", CaseSensitivity::Sensitive);

        // Forward from top-left: rows/cols in order, no wrap needed.
        let fwd: Vec<SearchMatch> = search
            .iter_from(SearchPosition::new(0, 0), SearchDirection::Forward)
            .collect();
        assert_eq!(
            fwd,
            vec![
                SearchMatch {
                    row: 0,
                    start_col: 2,
                    end_col: 5
                },
                SearchMatch {
                    row: 0,
                    start_col: 8,
                    end_col: 11
                },
                SearchMatch {
                    row: 1,
                    start_col: 0,
                    end_col: 3
                },
            ]
        );

        // Backward from end-of-grid: reverse order, no wrap.
        let back: Vec<SearchMatch> = search
            .iter_from(SearchPosition::new(1, 12), SearchDirection::Backward)
            .collect();
        assert_eq!(
            back,
            vec![
                SearchMatch {
                    row: 1,
                    start_col: 0,
                    end_col: 3
                },
                SearchMatch {
                    row: 0,
                    start_col: 8,
                    end_col: 11
                },
                SearchMatch {
                    row: 0,
                    start_col: 2,
                    end_col: 5
                },
            ]
        );

        // Forward from the middle match must wrap: middle, last, then first.
        let wrap_fwd: Vec<SearchMatch> = search
            .iter_from(SearchPosition::new(0, 8), SearchDirection::Forward)
            .collect();
        assert_eq!(
            wrap_fwd,
            vec![
                SearchMatch {
                    row: 0,
                    start_col: 8,
                    end_col: 11
                },
                SearchMatch {
                    row: 1,
                    start_col: 0,
                    end_col: 3
                },
                SearchMatch {
                    row: 0,
                    start_col: 2,
                    end_col: 5
                },
            ]
        );

        // Backward from the middle match must wrap the other way: the match
        // at the cursor first, then earlier matches, then the wrap to later.
        let wrap_back: Vec<SearchMatch> = search
            .iter_from(SearchPosition::new(0, 8), SearchDirection::Backward)
            .collect();
        assert_eq!(
            wrap_back,
            vec![
                SearchMatch {
                    row: 0,
                    start_col: 8,
                    end_col: 11
                },
                SearchMatch {
                    row: 0,
                    start_col: 2,
                    end_col: 5
                },
                SearchMatch {
                    row: 1,
                    start_col: 0,
                    end_col: 3
                },
            ]
        );

        // Each iterator visits every match exactly once.
        assert_eq!(wrap_fwd.len(), 3);
        assert_eq!(wrap_back.len(), 3);
    }

    #[test]
    fn no_match_single_match_and_overlapping_candidates() {
        // No match.
        let snap = snapshot_after(1, 8, b"abcdefg");
        let none = Search::new(&snap, "zzz", CaseSensitivity::Sensitive);
        assert_eq!(none.count(), 0);
        assert!(none.all().is_empty());
        assert_eq!(none.all().capacity(), 0, "no allocation without matches");
        assert!(none.first().is_none());
        assert!(none.last().is_none());

        // Single match.
        let one = Search::new(&snap, "cde", CaseSensitivity::Sensitive);
        assert_eq!(one.count(), 1);
        assert_eq!(
            one.first(),
            Some(SearchMatch {
                row: 0,
                start_col: 2,
                end_col: 5
            })
        );
        assert_eq!(one.last(), one.first());

        // Overlapping: "aa" in "aaa" matches at columns 0 and 1.
        let overlap_snap = snapshot_after(1, 8, b"aaa");
        let overlap = Search::new(&overlap_snap, "aa", CaseSensitivity::Sensitive);
        let matches = overlap.all();
        assert_eq!(
            matches,
            vec![
                SearchMatch {
                    row: 0,
                    start_col: 0,
                    end_col: 2
                },
                SearchMatch {
                    row: 0,
                    start_col: 1,
                    end_col: 3
                },
            ]
        );
    }

    #[test]
    fn empty_needle_matches_nothing_by_definition() {
        let snap = snapshot_after(2, 8, b"hello\r\nworld");
        let search = Search::new(&snap, "", CaseSensitivity::Sensitive);
        assert!(search.is_empty());
        assert_eq!(search.count(), 0);
        assert!(search.all().is_empty());
        assert!(search.first().is_none());
        assert!(search.last().is_none());
        let iter_count = search.iter().count();
        assert_eq!(iter_count, 0);
    }

    #[test]
    fn ten_thousand_row_search_completes_without_history_allocation() {
        // Scroll many distinctive rows into history. The scrollback cap is
        // 10,000 by design. `\r\n` returns the cursor to column 0 before each
        // line feed so each line is exactly one row; the final line has no
        // terminator so it remains on the visible screen.
        let mut state = TerminalState::new(1, 10).expect("valid terminal");
        for i in 0..12_000u32 {
            let line = format!("row{i:05}\r\n");
            state.feed_bytes(line.as_bytes());
        }
        // One final line without a newline stays visible.
        state.feed_bytes(b"row99999");
        let snap = state.snapshot();
        assert_eq!(
            snap.scrollback().len(),
            10_000,
            "scrollback is bounded at MAX_SCROLLBACK_LINES"
        );

        let search = Search::new(&snap, "needle", CaseSensitivity::Sensitive);
        // Completing over 10,000 rows is the gate; capacity 0 demonstrates no
        // history-proportional allocation on the no-match path.
        let matches = search.all();
        assert!(matches.is_empty());
        assert_eq!(matches.capacity(), 0);
        assert_eq!(search.count(), 0);

        // A needle that matches every row still allocates only per match.
        let every = Search::new(&snap, "row", CaseSensitivity::Sensitive);
        let expected = snap.logical_row_count() as usize;
        assert_eq!(every.count(), expected);
        let collected = every.all();
        assert_eq!(collected.len(), expected);
        assert!(collected.capacity() >= expected);
    }

    #[test]
    fn logical_row_accessor_resolves_scrollback_then_visible() {
        // Two-row grid; feed four short lines so two scroll off and two stay.
        let mut state = TerminalState::new(2, 4).expect("valid terminal");
        state.feed_bytes(b"AA\r\nBB\r\nCC\r\nDD");
        let snap = state.snapshot();
        assert_eq!(snap.scrollback().len(), 2);
        assert_eq!(snap.logical_row_count(), 4);

        // Scrollback rows first (oldest first).
        assert_eq!(
            snap.logical_row(0).and_then(|c| c.first().map(Cell::text)),
            Some("A")
        );
        assert_eq!(
            snap.logical_row(1).and_then(|c| c.first().map(Cell::text)),
            Some("B")
        );
        // Then visible rows top-to-bottom.
        assert_eq!(
            snap.logical_row(2).and_then(|c| c.first().map(Cell::text)),
            Some("C")
        );
        assert_eq!(
            snap.logical_row(3).and_then(|c| c.first().map(Cell::text)),
            Some("D")
        );
        assert!(snap.logical_row(4).is_none());
    }
}
