use crate::{
    attributes::{AnsiColor, CellAttributes, Color},
    parser::{Action, EraseMode, Parser, PrivateMode, is_sub_parameter},
};
use std::collections::VecDeque;
use std::fmt;

/// Hard allocation bound for a visible Terminal Core screen.
pub const MAX_SCREEN_CELLS: usize = 1024 * 1024;

/// Maximum number of zero-width combining characters that may be attached to a
/// single cell's base character.
///
/// `attach_zero_width` previously appended every combining mark to the target
/// cell's owned `String` with no cap, so a hostile PTY stream of `a` followed
/// by arbitrarily many `U+0301` marks grew one cell linearly in the input
/// volume while every other documented ceiling — cell count (`rows * cols` ≤
/// [`MAX_SCREEN_CELLS`]), scrollback lines (≤ [`MAX_SCROLLBACK_LINES`]) — still
/// held. The inflated cells are also copied verbatim into scrollback and every
/// snapshot, multiplying the cost. Marks beyond this budget are now dropped
/// instead of appended (KBUG-01).
///
/// # Why this value
///
/// Real text rarely stacks more than three or four combining marks on one base:
/// fully pointed Hebrew carries up to three (dagesh + vowel point + cantillation),
/// decomposed Vietnamese two, and Devanagari/Thai conjuncts two to three. A
/// budget of **seven** leaves roughly 2× headroom over the heaviest real script
/// while keeping the per-cell text bound small and exact. Legitimate accented
/// and Indic text (base + two to three marks) renders unchanged.
///
/// # Resulting per-cell bound
///
/// A cell holds at most one base character plus [`MAX_COMBINING_MARKS_PER_CELL`]
/// marks, i.e. at most eight `char`s. The longest UTF-8 encoding of any `char`
/// is four bytes, so a cell's owned text is bounded by
/// `4 * (MAX_COMBINING_MARKS_PER_CELL + 1) == 32` bytes. Adding
/// `size_of::<Cell>() == 32` gives a hard **64 bytes per cell** in the inflated
/// worst case (single-character cells stay near 40 bytes). That bound propagates
/// for free to every retained scrollback row and every snapshot, which simply
/// borrow the already-capped cells.
pub const MAX_COMBINING_MARKS_PER_CELL: usize = 7;

/// Maximum number of lines retained in the primary screen's scrollback buffer.
///
/// Only lines that scroll off the top of the *primary* screen are retained; the
/// alternate screen never contributes. The bound is enforced by evicting the
/// oldest retained line when the cap is reached, so a hostile program emitting
/// unbounded output cannot grow history without limit.
///
/// # Memory ceiling
///
/// Each retained line owns `cols` cells (the column count at the moment it
/// scrolled off). `size_of::<Cell>() == 40` on this target (a 24-byte owned
/// `String` handle, a width byte, and a 13-byte `CellAttributes` made of three
/// 4-byte `Color` selections plus a flag byte). The owned text is bounded
/// separately by [`MAX_COMBINING_MARKS_PER_CELL`]: at most one base `char`
/// plus seven combining marks, i.e. at most `4 * 8 == 32` bytes of text, so a
/// cell holds **72 bytes worst case** (the 40-byte struct plus 32 bytes of
/// capped text; single-character cells stay near 48). The retained-line
/// ceiling is therefore `MAX_SCROLLBACK_LINES * cols * bytes_per_cell`:
///
/// - Typical 80-column, single-character text: `10_000 * 80 * 48 ≈ 38 MiB`.
/// - Worst-case 256-column, fully capped cells: `10_000 * 256 * 72 ≈ 184 MiB`.
///
/// The line count is the hard bound: a hostile program cannot grow history past
/// this many lines regardless of volume, and a stream of zero-width combining
/// marks cannot grow one cell past the per-cell cap. Per-row width is bounded by
/// the live grid, which is itself bounded by [`MAX_SCREEN_CELLS`]. Resize does
/// **not** reflow retained lines (see the terminal-core-foundation design note),
/// so each line keeps the width it had when it scrolled off.
pub const MAX_SCROLLBACK_LINES: usize = 10_000;

/// One basic terminal cell.
///
/// Text remains an owned string so later grapheme work does not require a
/// renderer-facing shape change. Printing honors display width: a two-column
/// character occupies a lead cell (`width == 2`, holding the character) and a
/// zero-width continuation cell that renders as nothing and is never an
/// independent character; zero-width combining characters are appended to the
/// preceding cell's text without changing its width.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    text: String,
    width: u8,
    attributes: CellAttributes,
}

impl Cell {
    /// Construct a cell from display text and a precomputed column width.
    #[must_use]
    pub fn new(text: impl Into<String>, width: u8) -> Self {
        Self {
            text: text.into(),
            width,
            attributes: CellAttributes::default(),
        }
    }

    /// A blank, single-column cell.
    #[must_use]
    pub fn blank() -> Self {
        Self::new(" ", 1)
    }

    /// Display text owned by the cell.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Display width in terminal columns.
    #[must_use]
    pub const fn width(&self) -> u8 {
        self.width
    }

    /// Renderer-independent visual attributes captured when this cell was written.
    #[must_use]
    pub const fn attributes(&self) -> &CellAttributes {
        &self.attributes
    }

    /// Whether this is the baseline blank cell.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.text == " "
    }

    /// Whether this is the second column placeholder of a wide character.
    ///
    /// A continuation cell renders as nothing and must always be preceded in
    /// its row by its width-two lead cell.
    #[must_use]
    pub fn is_continuation(&self) -> bool {
        self.text.is_empty() && self.width == 0
    }

    fn from_char(ch: char, width: u8, attributes: CellAttributes) -> Self {
        Self {
            text: ch.to_string(),
            width,
            attributes,
        }
    }

    fn continuation(attributes: CellAttributes) -> Self {
        Self {
            text: String::new(),
            width: 0,
            attributes,
        }
    }

    fn push_text(&mut self, ch: char) {
        if self.combining_marks() < MAX_COMBINING_MARKS_PER_CELL {
            self.text.push(ch);
        }
    }

    /// Number of zero-width combining marks attached to the base character
    /// (every `char` past the first). Bounded by
    /// [`MAX_COMBINING_MARKS_PER_CELL`]: `push_text` drops the excess.
    fn combining_marks(&self) -> usize {
        self.text.chars().count().saturating_sub(1)
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::blank()
    }
}

/// Zero-based cursor position within the visible screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    row: u16,
    column: u16,
}

impl Cursor {
    /// Zero-based row.
    #[must_use]
    pub const fn row(self) -> u16 {
        self.row
    }

    /// Zero-based column.
    #[must_use]
    pub const fn column(self) -> u16 {
        self.column
    }
}

/// Renderer-independent cursor operations used by the parser and future APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMove {
    Up(u16),
    Down(u16),
    Right(u16),
    Left(u16),
    NextLine(u16),
    PreviousLine(u16),
    To { row: u16, column: u16 },
    ToColumn(u16),
    ToRow(u16),
}

/// Inclusive vertical margins used by index and explicit scroll operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollRegion {
    top: u16,
    bottom: u16,
}

impl ScrollRegion {
    fn full_screen(rows: u16) -> Self {
        Self {
            top: 0,
            bottom: rows - 1,
        }
    }

    fn checked(rows: u16, top: u16, bottom: u16) -> Result<Self, TerminalError> {
        // xterm clamps both margins to the screen before the validity check,
        // so ESC[1;6r on a 5-row terminal yields (0, 4) instead of being
        // dropped. DECSTBM is ignored only when top >= bottom AFTER clamping.
        let last_row = rows - 1;
        let top = top.min(last_row);
        let bottom = bottom.min(last_row);
        if top >= bottom {
            return Err(TerminalError::InvalidScrollRegion);
        }
        Ok(Self { top, bottom })
    }

    /// Zero-based first row in the region.
    #[must_use]
    pub const fn top(self) -> u16 {
        self.top
    }

    /// Zero-based last row in the region.
    #[must_use]
    pub const fn bottom(self) -> u16 {
        self.bottom
    }

    /// Number of rows in the inclusive region.
    #[must_use]
    pub const fn height(self) -> u16 {
        self.bottom - self.top + 1
    }
}

/// Renderer-independent modes that affect screen selection or encoded input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalModes {
    alternate_screen: bool,
    application_cursor_key: bool,
    application_keypad: bool,
    bracketed_paste: bool,
    mouse_normal_tracking: bool,
    mouse_button_event_tracking: bool,
    mouse_any_event_tracking: bool,
    mouse_utf8_encoding: bool,
    mouse_sgr_encoding: bool,
    mouse_urxvt_encoding: bool,
}

impl TerminalModes {
    /// Whether DEC private mode 1049 currently selects the alternate screen.
    #[must_use]
    pub const fn is_alternate_screen_active(self) -> bool {
        self.alternate_screen
    }

    /// Whether DECCKM (DEC cursor key mode) is set to application mode.
    #[must_use]
    pub const fn is_application_cursor_key_mode(self) -> bool {
        self.application_cursor_key
    }

    /// Whether DECKPAM (DEC keypad application mode) is set to application mode.
    #[must_use]
    pub const fn is_application_keypad_mode(self) -> bool {
        self.application_keypad
    }

    /// Whether DEC private mode 2004 (bracketed paste) is enabled.
    ///
    /// When enabled, the application expects user-initiated paste text wrapped
    /// in `CSI 200 ~` / `CSI 201 ~` markers; when disabled, paste must be
    /// gated rather than silently sent unbracketed.
    #[must_use]
    pub const fn is_bracketed_paste_enabled(self) -> bool {
        self.bracketed_paste
    }

    /// Whether DEC private mode 1000 (normal mouse tracking) is enabled.
    #[must_use]
    pub const fn is_mouse_normal_tracking_enabled(self) -> bool {
        self.mouse_normal_tracking
    }

    /// Whether DEC private mode 1002 (button-event mouse tracking) is enabled.
    #[must_use]
    pub const fn is_mouse_button_event_tracking_enabled(self) -> bool {
        self.mouse_button_event_tracking
    }

    /// Whether DEC private mode 1003 (any-event mouse tracking) is enabled.
    #[must_use]
    pub const fn is_mouse_any_event_tracking_enabled(self) -> bool {
        self.mouse_any_event_tracking
    }

    /// Whether DEC private mode 1006 (SGR mouse encoding) is enabled.
    #[must_use]
    pub const fn is_mouse_sgr_encoding_enabled(self) -> bool {
        self.mouse_sgr_encoding
    }

    /// Whether DEC private mode 1005 (UTF-8 mouse encoding) is enabled.
    #[must_use]
    pub const fn is_mouse_utf8_encoding_enabled(self) -> bool {
        self.mouse_utf8_encoding
    }

    /// Whether DEC private mode 1015 (urxvt mouse encoding) is enabled.
    #[must_use]
    pub const fn is_mouse_urxvt_encoding_enabled(self) -> bool {
        self.mouse_urxvt_encoding
    }
}

/// Fixed-size visible screen buffer in row-major order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenBuffer {
    rows: u16,
    cols: u16,
    cells: Vec<Cell>,
}

impl ScreenBuffer {
    fn new(rows: u16, cols: u16) -> Result<Self, TerminalError> {
        let count = checked_cell_count(rows, cols)?;
        Ok(Self {
            rows,
            cols,
            cells: vec![Cell::blank(); count],
        })
    }

    /// Number of visible rows.
    #[must_use]
    pub const fn rows(&self) -> u16 {
        self.rows
    }

    /// Number of visible columns.
    #[must_use]
    pub const fn cols(&self) -> u16 {
        self.cols
    }

    /// All visible cells in row-major order.
    #[must_use]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Number of leading screen rows retained by the display model.
    ///
    /// Trailing rows made only of baseline blanks and wide-character
    /// continuations are omitted. A row containing an explicit background is
    /// retained even when every cell contains only a space, because renderers
    /// still draw that background. This is the row-selection rule used by
    /// [`TerminalSnapshot::display_lines`] and
    /// [`TerminalSnapshot::display_cells`].
    #[must_use]
    pub fn display_row_count(&self) -> usize {
        visible_display_row_count(self)
    }

    /// The cells of one zero-based row, as a contiguous slice.
    ///
    /// Narrow accessor shared by the screen and scrollback search so neither
    /// needs to recompute row offsets from the flat [`cells`](Self::cells)
    /// array. Returns an empty slice for an out-of-range row.
    #[must_use]
    pub fn row(&self, row: u16) -> &[Cell] {
        if row >= self.rows {
            return &[];
        }
        let start = usize::from(row) * usize::from(self.cols);
        let end = start + usize::from(self.cols);
        &self.cells[start..end]
    }

    /// Cell at a zero-based visible position.
    #[must_use]
    pub fn cell(&self, row: u16, column: u16) -> Option<&Cell> {
        self.index(row, column)
            .and_then(|index| self.cells.get(index))
    }

    fn put(&mut self, row: u16, column: u16, cell: Cell) {
        if let Some(index) = self.index(row, column) {
            self.cells[index] = cell;
        }
    }

    fn is_continuation(&self, row: u16, column: u16) -> bool {
        self.cell(row, column).is_some_and(Cell::is_continuation)
    }

    fn push_text(&mut self, row: u16, column: u16, ch: char) {
        if let Some(index) = self.index(row, column) {
            self.cells[index].push_text(ch);
        }
    }

    /// Enforce the wide-character invariant on one row: every continuation
    /// cell directly follows its width-two lead, and every lead is directly
    /// followed by its continuation. A half that lost its partner is blanked,
    /// i.e. clearing either half of a wide character clears both.
    fn repair_row(&mut self, row: u16) {
        let Some(start) = self.index(row, 0) else {
            return;
        };
        let end = start + usize::from(self.cols);
        let mut index = start;
        while index < end {
            if self.cells[index].is_continuation() {
                self.cells[index] = Cell::blank();
                index += 1;
            } else if self.cells[index].width() == 2 {
                let paired = index + 1 < end && self.cells[index + 1].is_continuation();
                if paired {
                    index += 2;
                } else {
                    self.cells[index] = Cell::blank();
                    index += 1;
                }
            } else {
                index += 1;
            }
        }
    }

    /// Whether every wide-character pair in the buffer is intact.
    pub(crate) fn wide_cells_intact(&self) -> bool {
        (0..self.rows).all(|row| {
            let start = usize::from(row) * usize::from(self.cols);
            let end = start + usize::from(self.cols);
            let mut index = start;
            while index < end {
                if self.cells[index].width() == 2 {
                    if index + 1 >= end || !self.cells[index + 1].is_continuation() {
                        return false;
                    }
                    index += 2;
                } else if self.cells[index].is_continuation() {
                    return false;
                } else {
                    index += 1;
                }
            }
            true
        })
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        let count = checked_cell_count(rows, cols)?;
        let mut next = vec![Cell::blank(); count];
        let retained_rows = self.rows.min(rows);
        let retained_cols = self.cols.min(cols);
        for row in 0..retained_rows {
            for column in 0..retained_cols {
                let old_index = usize::from(row) * usize::from(self.cols) + usize::from(column);
                let new_index = usize::from(row) * usize::from(cols) + usize::from(column);
                next[new_index] = self.cells[old_index].clone();
            }
        }
        self.rows = rows;
        self.cols = cols;
        self.cells = next;
        // Shrinking a row can truncate the continuation half of a wide
        // character; blank the orphaned lead rather than leaving a pair
        // split across the right edge.
        for row in 0..retained_rows {
            self.repair_row(row);
        }
        Ok(())
    }

    /// Scroll the region up by `count`, returning the rows that left the top of
    /// the *visible screen* in top-to-bottom order.
    ///
    /// Only rows that leave the physical top of the screen (region top == 0) are
    /// returned; scrolling within a non-screen-aligned margin returns an empty
    /// vector because those rows never left the grid. The caller still decides
    /// whether retained rows belong in scrollback (primary screen only).
    fn scroll_up(&mut self, region: ScrollRegion, count: u16) -> Vec<Vec<Cell>> {
        let columns = usize::from(self.cols);
        let rows = usize::from(count.min(region.height()));
        let start = usize::from(region.top) * columns;
        let end = (usize::from(region.bottom) + 1) * columns;
        let shift = rows * columns;
        let evicted = if region.top == 0 && rows > 0 {
            (0..rows)
                .map(|r| self.cells[start + r * columns..start + (r + 1) * columns].to_vec())
                .collect()
        } else {
            Vec::new()
        };
        self.cells[start..end].rotate_left(shift);
        self.cells[end - shift..end].fill(Cell::blank());
        evicted
    }

    fn scroll_down(&mut self, region: ScrollRegion, count: u16) {
        let columns = usize::from(self.cols);
        let rows = usize::from(count.min(region.height()));
        let start = usize::from(region.top) * columns;
        let end = (usize::from(region.bottom) + 1) * columns;
        let shift = rows * columns;
        self.cells[start..end].rotate_right(shift);
        self.cells[start..start + shift].fill(Cell::blank());
    }

    fn erase_in_display(&mut self, cursor: Cursor, mode: EraseMode) {
        let Some(cursor_index) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let repaired_rows = match mode {
            EraseMode::ToEnd => cursor.row..self.rows,
            EraseMode::ToBeginning => 0..cursor.row + 1,
            EraseMode::All => 0..self.rows,
        };
        match mode {
            EraseMode::ToEnd => self.cells[cursor_index..].fill(Cell::default()),
            EraseMode::ToBeginning => self.cells[..cursor_index + 1].fill(Cell::default()),
            EraseMode::All => self.cells.fill(Cell::default()),
        }
        // An erase boundary may cut a wide character; blank the orphaned half.
        for row in repaired_rows {
            self.repair_row(row);
        }
    }

    fn erase_in_line(&mut self, cursor: Cursor, mode: EraseMode) {
        let Some(cursor_index) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let row_start = usize::from(cursor.row) * usize::from(self.cols);
        let row_end = row_start + usize::from(self.cols);
        match mode {
            EraseMode::ToEnd => self.cells[cursor_index..row_end].fill(Cell::default()),
            EraseMode::ToBeginning => {
                self.cells[row_start..cursor_index + 1].fill(Cell::default());
            }
            EraseMode::All => self.cells[row_start..row_end].fill(Cell::default()),
        }
        self.repair_row(cursor.row);
    }

    fn erase_characters(&mut self, cursor: Cursor, count: u16) {
        let Some(start) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let row_end = (usize::from(cursor.row) + 1) * usize::from(self.cols);
        let count = usize::from(count).min(row_end - start);
        self.cells[start..start + count].fill(Cell::default());
        self.repair_row(cursor.row);
    }

    fn insert_characters(&mut self, cursor: Cursor, count: u16) {
        let Some(start) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let row_end = (usize::from(cursor.row) + 1) * usize::from(self.cols);
        let count = usize::from(count).min(row_end - start);
        self.cells[start..row_end].rotate_right(count);
        self.cells[start..start + count].fill(Cell::default());
        self.repair_row(cursor.row);
    }

    fn delete_characters(&mut self, cursor: Cursor, count: u16) {
        let Some(start) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let row_end = (usize::from(cursor.row) + 1) * usize::from(self.cols);
        let count = usize::from(count).min(row_end - start);
        self.cells[start..row_end].rotate_left(count);
        self.cells[row_end - count..row_end].fill(Cell::default());
        self.repair_row(cursor.row);
    }

    fn index(&self, row: u16, column: u16) -> Option<usize> {
        if row >= self.rows || column >= self.cols {
            return None;
        }
        Some(usize::from(row) * usize::from(self.cols) + usize::from(column))
    }
}

/// Errors at the bounded Terminal Core state boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalError {
    InvalidSize,
    ScreenTooLarge,
    InvalidScrollRegion,
}

impl fmt::Display for TerminalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSize => f.write_str("terminal rows and columns must be non-zero"),
            Self::ScreenTooLarge => f.write_str("terminal screen exceeds the cell limit"),
            Self::InvalidScrollRegion => {
                f.write_str("terminal scroll region must be ordered and within the screen")
            }
        }
    }
}

impl std::error::Error for TerminalError {}

struct ScreenState {
    screen: ScreenBuffer,
    cursor: Cursor,
    saved_cursor: Option<Cursor>,
    scroll_region: ScrollRegion,
    wrap_pending: bool,
}

impl ScreenState {
    fn new(rows: u16, cols: u16) -> Result<Self, TerminalError> {
        Ok(Self {
            screen: ScreenBuffer::new(rows, cols)?,
            cursor: Cursor::default(),
            saved_cursor: None,
            scroll_region: ScrollRegion::full_screen(rows),
            wrap_pending: false,
        })
    }

    fn blank_like(&self) -> Self {
        Self {
            screen: ScreenBuffer {
                rows: self.screen.rows,
                cols: self.screen.cols,
                cells: vec![Cell::blank(); self.screen.cells.len()],
            },
            cursor: Cursor::default(),
            saved_cursor: None,
            scroll_region: ScrollRegion::full_screen(self.screen.rows),
            wrap_pending: false,
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        self.screen.resize(rows, cols)?;
        self.cursor = snap_to_lead(&self.screen, clamp_cursor(self.cursor, rows, cols));
        self.saved_cursor = self
            .saved_cursor
            .map(|cursor| snap_to_lead(&self.screen, clamp_cursor(cursor, rows, cols)));
        self.scroll_region = ScrollRegion::full_screen(rows);
        self.wrap_pending = false;
        Ok(())
    }

    fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor);
    }

    fn restore_cursor(&mut self) {
        if let Some(cursor) = self.saved_cursor {
            let cursor = clamp_cursor(cursor, self.screen.rows, self.screen.cols);
            self.cursor = snap_to_lead(&self.screen, cursor);
        }
        self.wrap_pending = false;
    }
}

/// Move a cursor off continuation cells onto the lead of its wide character.
fn snap_to_lead(screen: &ScreenBuffer, mut cursor: Cursor) -> Cursor {
    while cursor.column > 0 && screen.is_continuation(cursor.row, cursor.column) {
        cursor.column -= 1;
    }
    cursor
}

/// Move a cursor forward past any continuation cell, resolving to the cell
/// after the wide character when there is room and otherwise to its lead.
fn snap_past_continuation(screen: &ScreenBuffer, mut cursor: Cursor) -> Cursor {
    let last_column = screen.cols.saturating_sub(1);
    while cursor.column < last_column && screen.is_continuation(cursor.row, cursor.column) {
        cursor.column += 1;
    }
    snap_to_lead(screen, cursor)
}

/// Noren-owned mutable terminal state.
pub struct TerminalState {
    active: ScreenState,
    primary_screen: Option<ScreenState>,
    modes: TerminalModes,
    // SGR is terminal-global in this bounded slice; cells retain the value
    // captured when written, independently of screen-buffer selection.
    pen: CellAttributes,
    parser: Parser,
    // Bounded history of lines that scrolled off the top of the primary screen.
    // The alternate screen never contributes; see `push_scrollback_rows`.
    scrollback: VecDeque<Vec<Cell>>,
}

impl TerminalState {
    /// Create a bounded non-zero visible terminal.
    pub fn new(rows: u16, cols: u16) -> Result<Self, TerminalError> {
        Ok(Self {
            active: ScreenState::new(rows, cols)?,
            primary_screen: None,
            modes: TerminalModes::default(),
            pen: CellAttributes::default(),
            parser: Parser::default(),
            scrollback: VecDeque::new(),
        })
    }

    /// Apply PTY bytes in order.
    ///
    /// Bytes are decoded as UTF-8; printable characters are placed by display
    /// width, and invalid or unsupported bytes and sequences are ignored.
    /// They are never interpreted as authority or rendered as raw
    /// escape-sequence payload.
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if let Some(action) = self.parser.advance(*byte) {
                self.apply(action);
            }
        }
    }

    /// Resize active and inactive screens while preserving their overlap.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        checked_cell_count(rows, cols)?;
        self.active.resize(rows, cols)?;
        if let Some(primary) = &mut self.primary_screen {
            primary.resize(rows, cols)?;
        }
        Ok(())
    }

    /// Current `(rows, cols)`.
    #[must_use]
    pub const fn size(&self) -> (u16, u16) {
        (self.active.screen.rows, self.active.screen.cols)
    }

    /// Current cursor position.
    #[must_use]
    pub const fn cursor(&self) -> Cursor {
        self.active.cursor
    }

    /// Current visible screen.
    #[must_use]
    pub const fn screen(&self) -> &ScreenBuffer {
        &self.active.screen
    }

    /// Active inclusive vertical scrolling margins.
    #[must_use]
    pub const fn scroll_region(&self) -> ScrollRegion {
        self.active.scroll_region
    }

    /// Whether the next printable byte will wrap before it is written.
    #[must_use]
    pub const fn is_wrap_pending(&self) -> bool {
        self.active.wrap_pending
    }

    /// Current renderer-independent terminal mode state.
    #[must_use]
    pub const fn modes(&self) -> TerminalModes {
        self.modes
    }

    /// Current renderer-independent attributes captured by newly printed cells.
    #[must_use]
    pub const fn attributes(&self) -> &CellAttributes {
        &self.pen
    }

    /// Number of primary-screen lines currently retained in scrollback.
    ///
    /// Always bounded by [`MAX_SCROLLBACK_LINES`]. Use
    /// [`TerminalSnapshot::scrollback`] for the full ordered cell view and
    /// [`TerminalSnapshot::scrollback_lines`] for renderer-ready text.
    #[must_use]
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Borrow one retained scrollback row by eviction order index (oldest
    /// first), without cloning the buffer.
    ///
    /// Each row keeps the cells (and width) it had when it scrolled off; see
    /// [`TerminalSnapshot::scrollback`] for the cloned ordered view.
    #[must_use]
    pub fn scrollback_row(&self, index: usize) -> Option<&[Cell]> {
        self.scrollback.get(index).map(Vec::as_slice)
    }

    /// Save the active screen's cursor position.
    pub fn save_cursor(&mut self) {
        self.active.save_cursor();
    }

    /// Restore the active screen's saved cursor position, if present.
    pub fn restore_cursor(&mut self) {
        self.active.restore_cursor();
    }

    /// Set zero-based inclusive scrolling margins and move the cursor home.
    pub fn set_scroll_region(&mut self, top: u16, bottom: u16) -> Result<(), TerminalError> {
        self.active.scroll_region = ScrollRegion::checked(self.active.screen.rows, top, bottom)?;
        self.active.cursor = Cursor::default();
        self.active.wrap_pending = false;
        Ok(())
    }

    /// Apply a clamped renderer-independent cursor operation.
    ///
    /// The result never rests on a continuation cell: relative forward motion
    /// resolves past the wide character, and all other motion resolves onto
    /// its lead cell.
    pub fn move_cursor(&mut self, movement: CursorMove) {
        self.active.wrap_pending = false;
        let last_row = self.active.screen.rows - 1;
        let last_column = self.active.screen.cols - 1;
        let snap_forward = matches!(movement, CursorMove::Right(_));
        match movement {
            CursorMove::Up(count) => {
                self.active.cursor.row = self.active.cursor.row.saturating_sub(count);
            }
            CursorMove::Down(count) => {
                self.active.cursor.row = self.active.cursor.row.saturating_add(count).min(last_row);
            }
            CursorMove::Right(count) => {
                self.active.cursor.column = self
                    .active
                    .cursor
                    .column
                    .saturating_add(count)
                    .min(last_column);
            }
            CursorMove::Left(count) => {
                self.active.cursor.column = self.active.cursor.column.saturating_sub(count);
            }
            CursorMove::NextLine(count) => {
                self.active.cursor.row = self.active.cursor.row.saturating_add(count).min(last_row);
                self.active.cursor.column = 0;
            }
            CursorMove::PreviousLine(count) => {
                self.active.cursor.row = self.active.cursor.row.saturating_sub(count);
                self.active.cursor.column = 0;
            }
            CursorMove::To { row, column } => {
                self.active.cursor.row = row.min(last_row);
                self.active.cursor.column = column.min(last_column);
            }
            CursorMove::ToColumn(column) => {
                self.active.cursor.column = column.min(last_column);
            }
            CursorMove::ToRow(row) => {
                self.active.cursor.row = row.min(last_row);
            }
        }
        if snap_forward {
            self.active.cursor = snap_past_continuation(&self.active.screen, self.active.cursor);
        } else {
            self.snap_cursor_to_lead();
        }
    }

    /// Re-snap the active cursor onto a lead cell.
    ///
    /// Every row-changing control path (LF, IND, NEL, RI) keeps the cursor
    /// column while the row or the row's content changes, so each of them can
    /// land the cursor on the continuation half of a wide character. Routing
    /// all of them through this shared helper keeps the cursor off
    /// continuations, so a path added later cannot forget the re-snap.
    fn snap_cursor_to_lead(&mut self) {
        self.active.cursor = snap_to_lead(&self.active.screen, self.active.cursor);
    }

    /// Clone a bounded immutable renderer/test view.
    #[must_use]
    pub fn snapshot(&self) -> TerminalSnapshot {
        TerminalSnapshot::from_state(self)
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Print(ch) => self.print_char(ch),
            Action::LineFeed => self.line_feed(),
            Action::CarriageReturn => {
                self.active.cursor.column = 0;
                self.active.wrap_pending = false;
            }
            Action::Backspace => {
                self.active.cursor.column = self.active.cursor.column.saturating_sub(1);
                self.snap_cursor_to_lead();
                self.active.wrap_pending = false;
            }
            Action::Tab => self.tab(),
            Action::Index => self.index(),
            Action::NextLine => self.next_line(),
            Action::ReverseIndex => self.reverse_index(),
            Action::MoveUp(count) => self.move_cursor(CursorMove::Up(count)),
            Action::MoveDown(count) => self.move_cursor(CursorMove::Down(count)),
            Action::MoveRight(count) => self.move_cursor(CursorMove::Right(count)),
            Action::MoveLeft(count) => self.move_cursor(CursorMove::Left(count)),
            Action::MoveNextLine(count) => self.move_cursor(CursorMove::NextLine(count)),
            Action::MovePreviousLine(count) => self.move_cursor(CursorMove::PreviousLine(count)),
            Action::MoveTo { row, col } => {
                self.move_cursor(CursorMove::To { row, column: col });
            }
            Action::MoveToColumn(column) => self.move_cursor(CursorMove::ToColumn(column)),
            Action::MoveToRow(row) => self.move_cursor(CursorMove::ToRow(row)),
            Action::SetScrollRegion { top, bottom } => {
                self.apply_scroll_region(top, bottom);
            }
            Action::ScrollUp(count) => self.scroll_up(count),
            Action::ScrollDown(count) => self.scroll_down(count),
            Action::EraseInDisplay(mode) => self.erase_in_display(mode),
            Action::EraseInLine(mode) => self.erase_in_line(mode),
            Action::EraseCharacters(count) => self.erase_characters(count),
            Action::InsertCharacters(count) => self.insert_characters(count),
            Action::DeleteCharacters(count) => self.delete_characters(count),
            Action::InsertLines(count) => self.insert_lines(count),
            Action::DeleteLines(count) => self.delete_lines(count),
            Action::SelectGraphicRendition {
                params,
                separators,
                len,
            } => self.select_graphic_rendition(&params[..len], &separators[..len]),
            Action::SaveCursor => self.save_cursor(),
            Action::RestoreCursor => self.restore_cursor(),
            Action::SetKeypadApplication(enabled) => {
                self.modes.application_keypad = enabled;
            }
            Action::SetPrivateMode { mode, enabled } => self.set_private_mode(mode, enabled),
            Action::SetPrivateModes {
                modes,
                enabled,
                len,
            } => {
                for mode in modes[..len].iter().flatten() {
                    self.set_private_mode(*mode, enabled);
                }
            }
        }
        debug_assert!(self.active.screen.wide_cells_intact());
        if let Some(primary) = &self.primary_screen {
            debug_assert!(primary.screen.wide_cells_intact());
        }
    }

    /// Print one decoded character honoring its display width.
    ///
    /// A two-column character occupies a lead cell plus a zero-width
    /// continuation cell; it wraps to the next line when the remaining
    /// columns cannot fit both. Zero-width combining characters attach to the
    /// preceding cell and never move the cursor. The cursor never lands on a
    /// continuation cell: after writing a wide character flush against the
    /// right edge it waits on the lead cell with autowrap pending, and a
    /// character wider than the whole grid is dropped.
    fn print_char(&mut self, ch: char) {
        let width = match crate::cell_width(ch) {
            0 => {
                self.attach_zero_width(ch);
                return;
            }
            width => u16::try_from(width).unwrap_or(1),
        };
        if width > self.active.screen.cols {
            return;
        }
        if self.active.wrap_pending {
            self.active.cursor.column = 0;
            self.index();
        }
        if self.active.cursor.column > self.active.screen.cols - width {
            self.active.cursor.column = 0;
            self.index();
        }
        let row = self.active.cursor.row;
        let column = self.active.cursor.column;
        self.active
            .screen
            .put(row, column, Cell::from_char(ch, width as u8, self.pen));
        if width == 2 {
            self.active
                .screen
                .put(row, column + 1, Cell::continuation(self.pen));
        }
        // Overwritten halves of pre-existing wide characters must not dangle.
        self.active.screen.repair_row(row);
        if column + width >= self.active.screen.cols {
            self.active.wrap_pending = true;
        } else {
            self.active.cursor.column = column + width;
        }
    }

    /// Append a zero-width (combining) character to the preceding cell.
    ///
    /// Combining characters are attached rather than dropped: they extend the
    /// preceding cell's text without changing its width, never advance the
    /// cursor, and do not clear pending autowrap. With no preceding cell the
    /// character is dropped.
    fn attach_zero_width(&mut self, ch: char) {
        let cursor = self.active.cursor;
        let column = if self.active.wrap_pending {
            Some(cursor.column)
        } else {
            cursor.column.checked_sub(1)
        };
        let Some(mut column) = column else {
            return;
        };
        if self.active.screen.is_continuation(cursor.row, column) {
            column = column.saturating_sub(1);
        }
        self.active.screen.push_text(cursor.row, column, ch);
    }

    fn line_feed(&mut self) {
        self.active.wrap_pending = false;
        self.index();
    }

    fn tab(&mut self) {
        self.active.wrap_pending = false;
        let last_column = self.active.screen.cols - 1;
        let next_stop = (usize::from(self.active.cursor.column) / 8 + 1) * 8;
        self.active.cursor.column =
            last_column.min(u16::try_from(next_stop).unwrap_or(last_column));
        self.active.cursor = snap_past_continuation(&self.active.screen, self.active.cursor);
    }

    /// Move the cursor down one row, scrolling at the bottom margin. Shared
    /// by LF, IND, and NEL, and ends in the shared lead re-snap.
    fn index(&mut self) {
        self.active.wrap_pending = false;
        if self.active.cursor.row == self.active.scroll_region.bottom {
            self.scroll_up_capturing(self.active.scroll_region, 1);
        } else if self.active.cursor.row < self.active.screen.rows - 1 {
            self.active.cursor.row += 1;
        }
        self.snap_cursor_to_lead();
    }

    fn next_line(&mut self) {
        self.active.cursor.column = 0;
        self.index();
    }

    /// Move the cursor up one row, scrolling at the top margin, and end in
    /// the shared lead re-snap.
    fn reverse_index(&mut self) {
        self.active.wrap_pending = false;
        if self.active.cursor.row == self.active.scroll_region.top {
            self.active.screen.scroll_down(self.active.scroll_region, 1);
        } else if self.active.cursor.row > 0 {
            self.active.cursor.row -= 1;
        }
        self.snap_cursor_to_lead();
    }

    fn scroll_up(&mut self, count: u16) {
        self.active.wrap_pending = false;
        self.scroll_up_capturing(self.active.scroll_region, count);
    }

    /// Scroll the active screen's region up and, when rows actually leave the
    /// visible primary screen, retain them in scrollback.
    ///
    /// Retention requires both: (1) the primary screen is active (the alternate
    /// screen never contributes, matching `less`/`vim` behavior), and (2) the
    /// region starts at row 0 so the evicted rows left the top of the screen
    /// rather than just the top of a non-screen-aligned margin.
    fn scroll_up_capturing(&mut self, region: ScrollRegion, count: u16) {
        let evicted = self.active.screen.scroll_up(region, count);
        if self.modes.alternate_screen || region.top != 0 || evicted.is_empty() {
            return;
        }
        for row in evicted {
            if self.scrollback.len() >= MAX_SCROLLBACK_LINES {
                self.scrollback.pop_front();
            }
            self.scrollback.push_back(row);
        }
    }

    fn scroll_down(&mut self, count: u16) {
        self.active.wrap_pending = false;
        self.active
            .screen
            .scroll_down(self.active.scroll_region, count);
    }

    fn erase_in_display(&mut self, mode: EraseMode) {
        self.active.wrap_pending = false;
        self.active
            .screen
            .erase_in_display(self.active.cursor, mode);
    }

    fn erase_in_line(&mut self, mode: EraseMode) {
        self.active.wrap_pending = false;
        self.active.screen.erase_in_line(self.active.cursor, mode);
    }

    fn erase_characters(&mut self, count: u16) {
        self.active.wrap_pending = false;
        self.active
            .screen
            .erase_characters(self.active.cursor, count);
    }

    fn insert_characters(&mut self, count: u16) {
        self.active.wrap_pending = false;
        self.active
            .screen
            .insert_characters(self.active.cursor, count);
    }

    fn delete_characters(&mut self, count: u16) {
        self.active.wrap_pending = false;
        self.active
            .screen
            .delete_characters(self.active.cursor, count);
    }

    fn insert_lines(&mut self, count: u16) {
        self.active.wrap_pending = false;
        let cursor_row = self.active.cursor.row;
        let region = self.active.scroll_region;
        if (region.top..=region.bottom).contains(&cursor_row) {
            self.active.screen.scroll_down(
                ScrollRegion {
                    top: cursor_row,
                    bottom: region.bottom,
                },
                count,
            );
        }
    }

    fn delete_lines(&mut self, count: u16) {
        self.active.wrap_pending = false;
        let cursor_row = self.active.cursor.row;
        let region = self.active.scroll_region;
        if (region.top..=region.bottom).contains(&cursor_row) {
            self.scroll_up_capturing(
                ScrollRegion {
                    top: cursor_row,
                    bottom: region.bottom,
                },
                count,
            );
        }
    }

    fn select_graphic_rendition(&mut self, params: &[u16], separators: &[u8]) {
        let mut index = 0;
        while index < params.len() {
            let param = params[index];
            // Extent of the current ECMA-48 parameter group: the leading value
            // plus any trailing colon sub-parameters (`:`-separated). A lone
            // `;`-separated value is a one-element group.
            let group_end = sgr_group_end(separators, index);
            // Outside the extended-color selectors (38/48/58), a multi-element
            // colon group is an unsupported compound attribute (e.g. `4:0` and
            // other modern underline styles). It must be skipped whole so its
            // sub-parameters never touch the pen — `4:0` must not turn underline
            // on and then let the trailing `0` reset every attribute.
            if group_end > index + 1 && !matches!(param, 38 | 48 | 58) {
                index = group_end;
                continue;
            }
            match param {
                0 => self.pen = CellAttributes::default(),
                1 => self.pen = self.pen.with_bold(true),
                22 => self.pen = self.pen.with_bold(false),
                4 => self.pen = self.pen.with_underline(true),
                24 => self.pen = self.pen.with_underline(false),
                7 => self.pen = self.pen.with_reverse(true),
                27 => self.pen = self.pen.with_reverse(false),
                30..=37 => {
                    self.pen = self
                        .pen
                        .with_foreground(Color::Ansi(AnsiColor::ALL[usize::from(param - 30)]));
                }
                39 => self.pen = self.pen.with_foreground(Color::Default),
                40..=47 => {
                    self.pen = self
                        .pen
                        .with_background(Color::Ansi(AnsiColor::ALL[usize::from(param - 40)]));
                }
                49 => self.pen = self.pen.with_background(Color::Default),
                90..=97 => {
                    self.pen = self
                        .pen
                        .with_foreground(Color::Ansi(AnsiColor::ALL[usize::from(param - 90 + 8)]));
                }
                100..=107 => {
                    self.pen = self
                        .pen
                        .with_background(Color::Ansi(AnsiColor::ALL[usize::from(param - 100 + 8)]));
                }
                58 => {
                    let (color, consumed) = parse_extended_color(params, separators, index);
                    if let Some(color) = color {
                        self.pen = self.pen.with_underline_color(color);
                    }
                    index += consumed;
                    continue;
                }
                59 => self.pen = self.pen.with_underline_color(Color::Default),
                38 => {
                    let (color, consumed) = parse_extended_color(params, separators, index);
                    if let Some(color) = color {
                        self.pen = self.pen.with_foreground(color);
                    }
                    index += consumed;
                    continue;
                }
                48 => {
                    let (color, consumed) = parse_extended_color(params, separators, index);
                    if let Some(color) = color {
                        self.pen = self.pen.with_background(color);
                    }
                    index += consumed;
                    continue;
                }
                _ => {}
            }
            index += 1;
        }
    }

    fn reset_scroll_region(&mut self) {
        self.active.scroll_region = ScrollRegion::full_screen(self.active.screen.rows);
        self.active.cursor = Cursor::default();
        self.active.wrap_pending = false;
    }

    fn apply_scroll_region(&mut self, top: u16, bottom: Option<u16>) {
        if top == 0 && bottom.is_none() {
            self.reset_scroll_region();
        } else {
            let bottom = bottom.unwrap_or(self.active.screen.rows - 1);
            let _ = self.set_scroll_region(top, bottom);
        }
    }

    fn set_private_mode(&mut self, mode: PrivateMode, enabled: bool) {
        match (mode, enabled) {
            (PrivateMode::AlternateScreen, true) => self.enter_alternate_screen(),
            (PrivateMode::AlternateScreen, false) => self.leave_alternate_screen(),
            (PrivateMode::ApplicationCursorKey, enabled) => {
                self.modes.application_cursor_key = enabled;
            }
            (PrivateMode::BracketedPaste, enabled) => {
                self.modes.bracketed_paste = enabled;
            }
            (PrivateMode::MouseTrackingNormal, enabled) => {
                self.modes.mouse_normal_tracking = enabled;
            }
            (PrivateMode::MouseTrackingButtonEvent, enabled) => {
                self.modes.mouse_button_event_tracking = enabled;
            }
            (PrivateMode::MouseTrackingAnyEvent, enabled) => {
                self.modes.mouse_any_event_tracking = enabled;
            }
            (PrivateMode::MouseEncodingUtf8, enabled) => {
                self.modes.mouse_utf8_encoding = enabled;
            }
            (PrivateMode::MouseEncodingSgr, enabled) => {
                self.modes.mouse_sgr_encoding = enabled;
            }
            (PrivateMode::MouseEncodingUrxvt, enabled) => {
                self.modes.mouse_urxvt_encoding = enabled;
            }
        }
    }

    fn enter_alternate_screen(&mut self) {
        if self.modes.alternate_screen {
            return;
        }
        self.active.save_cursor();
        let alternate = self.active.blank_like();
        let primary = std::mem::replace(&mut self.active, alternate);
        debug_assert!(self.primary_screen.is_none());
        self.primary_screen = Some(primary);
        self.modes.alternate_screen = true;
    }

    fn leave_alternate_screen(&mut self) {
        if !self.modes.alternate_screen {
            return;
        }
        if let Some(primary) = self.primary_screen.take() {
            self.active = primary;
            self.active.restore_cursor();
        }
        self.modes.alternate_screen = false;
    }
}

/// End index (exclusive) of the ECMA-48 parameter group that begins at `start`.
///
/// A group is the leading value plus every trailing colon sub-parameter
/// (separator `SEPARATOR_SUB`). A lone `;`-separated value is a one-element
/// group, so this returns `start + 1` for it. Used by the SGR walker to treat a
/// colon-bearing attribute as a single parameter and skip unsupported compound
/// groups (e.g. `4:0`) whole instead of walking their sub-parameters.
fn sgr_group_end(separators: &[u8], start: usize) -> usize {
    let mut end = start + 1;
    while end < separators.len() && is_sub_parameter(&separators[end]) {
        end += 1;
    }
    end
}

/// Parse an extended SGR color (`38`/`48`/`58`) starting at `start`.
///
/// Returns the parsed color (if the sequence is well-formed and in range) and
/// the number of parameter slots consumed, *including* the selector. The
/// consume count is chosen so a truncated or out-of-range sequence can never
/// leak its channel values back into the caller as independent bold/underline/
/// reverse codes: every channel slot the selector claimed is skipped over
/// whether or not a color was produced.
///
/// Both ECMA-48 forms are handled:
///
/// - semicolon form: `38;5;N` and `38;2;R;G;B` (xterm);
/// - colon sub-parameter form: `38:5:N` and `38:2::R:G:B` (ITU-T T.416). The
///   RGB colon form carries an empty colour-space slot after the `2`; a
///   non-empty slot (`38:2:Pi:R:G:B`) is accepted and ignored. The whole run
///   of sub-parameters belongs to the selector's parameter, so all of it is
///   consumed.
fn parse_extended_color(params: &[u16], separators: &[u8], start: usize) -> (Option<Color>, usize) {
    let Some(mode_index) = start.checked_add(1).filter(|&i| i < params.len()) else {
        return (None, 1);
    };
    let remaining = params.len() - start;
    let colon_form = separators.get(mode_index).is_some_and(is_sub_parameter);
    if colon_form {
        // The whole run of `:`-separated sub-parameters belongs to the
        // selector's parameter, so all of it is consumed regardless of whether
        // a color is produced.
        let run_end = params.len().min(
            (mode_index..)
                .take_while(|&i| separators.get(i).is_some_and(is_sub_parameter))
                .last()
                .map_or(mode_index, |i| i + 1),
        );
        let run = &params[mode_index..run_end];
        let consumed = 1 + run.len();
        let body = run.get(1..).unwrap_or(&[]);
        let color = parse_extended_color_body(*run.first().unwrap_or(&0), body);
        (color, consumed)
    } else {
        let mode = params[mode_index];
        match mode {
            5 => {
                let consumed = remaining.min(3);
                let color = params
                    .get(start + 2)
                    .copied()
                    .filter(|value| *value <= u16::from(u8::MAX))
                    .map(|value| Color::Indexed(value as u8));
                (color, consumed)
            }
            2 => {
                let consumed = remaining.min(5);
                let body = params.get(start + 2..start + 5).unwrap_or(&[]);
                let color = parse_extended_color_body(mode, body);
                (color, consumed)
            }
            _ => (None, remaining.min(2)),
        }
    }
}

/// Resolve the body following an extended-color mode (`5` indexed, `2` direct).
///
/// `body` excludes the mode slot itself. For the colon RGB form the first body
/// slot is the colour-space id (often empty) when four slots are present, and
/// is ignored; the next three are red/green/blue. Indexed values above 255 are
/// out of range and yield no color; direct channels are clamped to `u8`.
fn parse_extended_color_body(mode: u16, body: &[u16]) -> Option<Color> {
    match mode {
        5 => body
            .first()
            .copied()
            .filter(|value| *value <= u16::from(u8::MAX))
            .map(|value| Color::Indexed(value as u8)),
        2 => {
            let channels = if body.len() >= 4 {
                body.get(1..4)
            } else if body.len() == 3 {
                body.get(0..3)
            } else {
                None
            }?;
            Some(Color::Rgb(
                clamp_channel(channels[0]),
                clamp_channel(channels[1]),
                clamp_channel(channels[2]),
            ))
        }
        _ => None,
    }
}

/// Clamp a direct-color channel to its eight-bit range, matching xterm.
fn clamp_channel(value: u16) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

impl fmt::Debug for TerminalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalState")
            .field("size", &self.size())
            .field("cursor", &self.active.cursor)
            .field("scroll_region", &self.active.scroll_region)
            .field("wrap_pending", &self.active.wrap_pending)
            .field("modes", &self.modes)
            .finish_non_exhaustive()
    }
}

/// Immutable snapshot passed to renderers and deterministic test oracles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    screen: ScreenBuffer,
    cursor: Cursor,
    scroll_region: ScrollRegion,
    wrap_pending: bool,
    modes: TerminalModes,
    lines: Vec<String>,
    display_lines: Vec<String>,
    scrollback: Vec<Vec<Cell>>,
}

impl TerminalSnapshot {
    fn from_state(state: &TerminalState) -> Self {
        Self {
            screen: state.active.screen.clone(),
            cursor: state.active.cursor,
            scroll_region: state.active.scroll_region,
            wrap_pending: state.active.wrap_pending,
            modes: state.modes,
            lines: visible_lines(&state.active.screen),
            display_lines: visible_display_lines(&state.active.screen),
            scrollback: state.scrollback.iter().cloned().collect(),
        }
    }

    /// Number of visible rows.
    #[must_use]
    pub const fn rows(&self) -> u16 {
        self.screen.rows
    }

    /// Number of visible columns.
    #[must_use]
    pub const fn cols(&self) -> u16 {
        self.screen.cols
    }

    /// Cursor captured with the screen.
    #[must_use]
    pub const fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Active scrolling margins captured with the screen.
    #[must_use]
    pub const fn scroll_region(&self) -> ScrollRegion {
        self.scroll_region
    }

    /// Whether a printable byte would wrap before being written.
    #[must_use]
    pub const fn is_wrap_pending(&self) -> bool {
        self.wrap_pending
    }

    /// Terminal modes captured with the visible screen.
    #[must_use]
    pub const fn modes(&self) -> TerminalModes {
        self.modes
    }

    /// Captured visible screen.
    #[must_use]
    pub const fn screen(&self) -> &ScreenBuffer {
        &self.screen
    }

    /// Renderer-compatible text rows with trailing blank rows/columns removed.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Column-preserving rendering of the visible screen, parallel to
    /// [`lines`](Self::lines).
    ///
    /// Continuation cells of wide characters keep one placeholder column so a
    /// consumer that enumerates characters positions every glyph at its
    /// display column. Row selection, trailing-blank trimming, and ASCII rows
    /// match [`lines`](Self::lines) exactly; only the wide-character
    /// continuation columns differ.
    #[must_use]
    pub fn display_lines(&self) -> &[String] {
        &self.display_lines
    }

    /// Display-positioned cell rows of the visible screen, parallel to
    /// [`display_lines`](Self::display_lines).
    ///
    /// Rows are contiguous slices of exactly [`cols`](Self::cols) cells,
    /// selected exactly like `display_lines` (trailing all-blank rows are
    /// dropped, so both accessors always agree on the row count and on which
    /// screen row each entry represents). Enumerating a row yields one cell
    /// per display column: a width-two character's continuation cell keeps
    /// its own column with empty text ([`Cell::is_continuation`]), encoding
    /// the same column rule that `display_lines` encodes with one placeholder
    /// character — so both accessors agree where every following glyph lands.
    /// Within a row, trailing blank cells are retained (only whole rows are
    /// dropped) so per-cell attributes stay addressable to the end of the
    /// row.
    #[must_use]
    pub fn display_cells(&self) -> impl ExactSizeIterator<Item = &[Cell]> {
        let cols = usize::from(self.screen.cols);
        self.screen
            .cells
            .chunks_exact(cols)
            .take(self.screen.display_row_count())
    }

    /// Retained scrollback rows in eviction order (oldest first, newest last).
    ///
    /// Each row is the full cell content of a primary-screen line that scrolled
    /// off the top of the visible grid, captured at the width it had when it
    /// left. The alternate screen never contributes. The slice length is bounded
    /// by [`MAX_SCROLLBACK_LINES`].
    #[must_use]
    pub fn scrollback(&self) -> &[Vec<Cell>] {
        &self.scrollback
    }

    /// Renderer-ready text rendering of [`scrollback`](Self::scrollback) with
    /// trailing blanks trimmed per line, parallel to [`lines`](Self::lines).
    #[must_use]
    pub fn scrollback_lines(&self) -> Vec<String> {
        self.scrollback
            .iter()
            .map(|row| cells_to_line(row))
            .collect()
    }

    /// Total logical rows spanning scrollback followed by the visible screen.
    ///
    /// Scrollback rows are indexed oldest-first (the order returned by
    /// [`scrollback`](Self::scrollback)); the visible rows follow in
    /// top-to-bottom order. The value fits a `u32` because both contributors
    /// are bounded ([`MAX_SCROLLBACK_LINES`] plus a `u16` visible row count).
    #[must_use]
    pub fn logical_row_count(&self) -> u32 {
        u32::try_from(self.scrollback.len() + usize::from(self.screen.rows)).unwrap_or(u32::MAX)
    }

    /// Borrow one logical row as a cell slice, scrollback-first then visible.
    ///
    /// Indexing matches [`logical_row_count`](Self::logical_row_count): rows
    /// `0..scrollback_len()` borrow scrollback rows in oldest-first order, and
    /// rows `scrollback_len()..` borrow visible rows in top-to-bottom order.
    /// Returns `None` for out-of-range indices. Used by
    /// [`Search`](crate::search::Search) so the renderer-independent search
    /// never copies history into a rectangle.
    #[must_use]
    pub fn logical_row(&self, index: u32) -> Option<&[Cell]> {
        let i = usize::try_from(index).ok()?;
        let sb = self.scrollback.len();
        if i < sb {
            Some(self.scrollback[i].as_slice())
        } else {
            let v = i - sb;
            let cols = usize::from(self.screen.cols);
            if v < usize::from(self.screen.rows) {
                Some(&self.screen.cells[v * cols..(v + 1) * cols])
            } else {
                None
            }
        }
    }
}

fn checked_cell_count(rows: u16, cols: u16) -> Result<usize, TerminalError> {
    if rows == 0 || cols == 0 {
        return Err(TerminalError::InvalidSize);
    }
    let count = usize::from(rows) * usize::from(cols);
    if count > MAX_SCREEN_CELLS {
        Err(TerminalError::ScreenTooLarge)
    } else {
        Ok(count)
    }
}

fn clamp_cursor(cursor: Cursor, rows: u16, cols: u16) -> Cursor {
    Cursor {
        row: cursor.row.min(rows - 1),
        column: cursor.column.min(cols - 1),
    }
}

fn visible_lines(screen: &ScreenBuffer) -> Vec<String> {
    let mut lines = Vec::with_capacity(usize::from(screen.rows));
    for row in 0..screen.rows {
        let start = usize::from(row) * usize::from(screen.cols);
        let end = start + usize::from(screen.cols);
        lines.push(cells_to_line(&screen.cells[start..end]));
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// Render a cell row to text with trailing blanks removed. Shared by the
/// visible-screen snapshot and scrollback so the trimming policy is identical.
fn cells_to_line(cells: &[Cell]) -> String {
    let mut line = String::with_capacity(cells.len());
    for cell in cells {
        line.push_str(cell.text());
    }
    while line.ends_with(' ') {
        line.pop();
    }
    line
}

fn visible_display_lines(screen: &ScreenBuffer) -> Vec<String> {
    let cols = usize::from(screen.cols);
    screen
        .cells
        .chunks_exact(cols)
        .take(screen.display_row_count())
        .map(cells_to_display_line)
        .collect()
}

/// Render a cell row to text preserving display columns: a continuation cell
/// contributes one placeholder column so character positions equal display
/// columns. Trailing blanks are trimmed exactly like [`cells_to_line`].
fn cells_to_display_line(cells: &[Cell]) -> String {
    let mut line = String::with_capacity(cells.len());
    for cell in cells {
        if cell.is_continuation() {
            line.push(' ');
        } else {
            line.push_str(cell.text());
        }
    }
    while line.ends_with(' ') {
        line.pop();
    }
    line
}

/// Whether a cell row renders as a blank display line.
///
/// This is exactly the condition under which
/// [`visible_display_lines`] drops a trailing row: every cell contributes
/// only spaces (blank cells) or placeholders (continuation cells), and no cell
/// carries an explicit background. Keeping the predicate here — next to the
/// line builder it mirrors — is what lets [`TerminalSnapshot::display_cells`]
/// select the same rows as [`TerminalSnapshot::display_lines`] while retaining
/// background-only rows for rendering.
fn row_is_display_blank(cells: &[Cell]) -> bool {
    cells.iter().all(|cell| {
        (cell.is_blank() || cell.is_continuation()) && cell.attributes().background().is_default()
    })
}

/// Shared row count behind every display-facing screen view.
fn visible_display_row_count(screen: &ScreenBuffer) -> usize {
    let cols = usize::from(screen.cols);
    screen
        .cells
        .chunks_exact(cols)
        .rposition(|row| !row_is_display_blank(row))
        .map_or(0, |row| row + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ascii_and_controls_update_owned_state() {
        let mut state = TerminalState::new(3, 5).expect("valid terminal");
        state.feed_bytes(b"ab\rZ\nQ\x08R");

        assert_eq!(state.snapshot().lines(), ["Zb", " R"]);
        assert_eq!(state.cursor(), Cursor { row: 1, column: 2 });
        assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("Z"));
    }

    #[test]
    fn csi_cursor_foundation_is_clamped_and_renderer_independent() {
        let mut state = TerminalState::new(3, 5).expect("valid terminal");
        state.feed_bytes(b"abc\x1b[2DX\x1b[3;4HY");

        assert_eq!(state.screen().cell(0, 1).map(Cell::text), Some("X"));
        assert_eq!(state.screen().cell(2, 3).map(Cell::text), Some("Y"));
        assert_eq!(state.cursor(), Cursor { row: 2, column: 4 });
    }

    #[test]
    fn index_and_reverse_index_scroll_only_inside_the_active_region() {
        let mut state = TerminalState::new(5, 2).expect("valid terminal");
        state.feed_bytes(b"A\x1b[2;1HB\x1b[3;1HC\x1b[4;1HD\x1b[5;1HE");
        state.feed_bytes(b"\x1b[2;4r\x1b[4;1H\x1bD");

        assert_eq!(state.snapshot().lines(), ["A", "C", "D", "", "E"]);
        assert_eq!(state.cursor(), Cursor { row: 3, column: 0 });

        state.feed_bytes(b"\x1b[2;1H\x1bM");
        assert_eq!(state.snapshot().lines(), ["A", "", "C", "D", "E"]);
        assert_eq!(state.cursor(), Cursor { row: 1, column: 0 });
    }

    #[test]
    fn carriage_return_cancels_delayed_wrap_without_scrolling() {
        let mut state = TerminalState::new(2, 3).expect("valid terminal");
        state.feed_bytes(b"abc");
        assert!(state.is_wrap_pending());

        state.feed_bytes(b"\rZ");
        assert_eq!(state.snapshot().lines(), ["Zbc"]);
        assert_eq!(state.cursor(), Cursor { row: 0, column: 1 });
        assert!(!state.is_wrap_pending());
    }

    fn row_widths(state: &TerminalState, row: u16) -> Vec<u8> {
        (0..state.screen().cols())
            .map(|column| state.screen().cell(row, column).map_or(0, Cell::width))
            .collect()
    }

    #[test]
    fn ascii_lines_are_unchanged_by_the_width_model() {
        let mut state = TerminalState::new(2, 8).expect("valid terminal");
        state.feed_bytes(b"hello");

        assert_eq!(state.snapshot().lines(), ["hello"]);
        assert_eq!(row_widths(&state, 0), vec![1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(state.cursor(), Cursor { row: 0, column: 5 });
        assert!(!state.is_wrap_pending());
    }

    #[test]
    fn cjk_characters_occupy_two_cells_each() {
        let mut state = TerminalState::new(2, 8).expect("valid terminal");
        state.feed_bytes("日本語".as_bytes());

        let screen = state.screen();
        assert_eq!(
            screen.cell(0, 0).map(|c| (c.text(), c.width())),
            Some(("日", 2))
        );
        assert!(screen.cell(0, 1).is_some_and(Cell::is_continuation));
        assert_eq!(
            screen.cell(0, 2).map(|c| (c.text(), c.width())),
            Some(("本", 2))
        );
        assert!(screen.cell(0, 3).is_some_and(Cell::is_continuation));
        assert_eq!(
            screen.cell(0, 4).map(|c| (c.text(), c.width())),
            Some(("語", 2))
        );
        assert!(screen.cell(0, 5).is_some_and(Cell::is_continuation));
        assert_eq!(state.snapshot().lines(), ["日本語"]);
        assert_eq!(state.cursor(), Cursor { row: 0, column: 6 });
    }

    #[test]
    fn wide_character_at_the_last_column_wraps_instead_of_splitting() {
        let mut state = TerminalState::new(3, 4).expect("valid terminal");
        state.feed_bytes("abc日".as_bytes());

        assert_eq!(state.snapshot().lines(), ["abc", "日"]);
        // The lead never lands in the final column without its continuation.
        assert_eq!(state.screen().cell(0, 3).map(Cell::text), Some(" "));
        assert_eq!(
            state.screen().cell(1, 0).map(|c| (c.text(), c.width())),
            Some(("日", 2))
        );
        assert!(state.screen().cell(1, 1).is_some_and(Cell::is_continuation));
        assert_eq!(state.cursor(), Cursor { row: 1, column: 2 });
    }

    #[test]
    fn wide_character_fitting_at_the_right_edge_keeps_cursor_on_its_lead() {
        let mut state = TerminalState::new(2, 5).expect("valid terminal");
        state.feed_bytes("abc日".as_bytes());

        assert_eq!(state.snapshot().lines(), ["abc日"]);
        assert!(state.is_wrap_pending());
        // Pending wrap parks the cursor on the lead, never the continuation.
        assert_eq!(state.cursor(), Cursor { row: 0, column: 3 });
        assert!(!state.screen().cell(0, 3).is_some_and(Cell::is_continuation));
    }

    #[test]
    fn overwriting_or_erasing_half_of_a_wide_character_leaves_no_continuation() {
        // Overwrite the lead of a wide character with a narrow one.
        let mut state = TerminalState::new(2, 6).expect("valid terminal");
        state.feed_bytes("日a".as_bytes());
        state.feed_bytes(b"\x1b[1;1HX");
        assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("X"));
        assert_eq!(state.screen().cell(0, 1).map(Cell::text), Some(" "));
        assert!(!state.screen().cell(0, 1).is_some_and(Cell::is_continuation));
        assert_eq!(state.snapshot().lines(), ["X a"]);

        // ECH over only the lead clears the orphaned continuation too.
        let mut state = TerminalState::new(2, 6).expect("valid terminal");
        state.feed_bytes("a日b".as_bytes());
        state.feed_bytes(b"\x1b[1;2H\x1b[X");
        assert_eq!(state.snapshot().lines(), ["a  b"]);

        // EL to the beginning up to the lead clears the continuation after it.
        let mut state = TerminalState::new(2, 6).expect("valid terminal");
        state.feed_bytes("a日b".as_bytes());
        state.feed_bytes(b"\x1b[1;2H\x1b[1K");
        assert_eq!(state.snapshot().lines(), ["   b"]);

        // DCH deleting the lead shifts the row and clears the continuation.
        let mut state = TerminalState::new(2, 6).expect("valid terminal");
        state.feed_bytes("a日b".as_bytes());
        state.feed_bytes(b"\x1b[1;2H\x1b[P");
        assert_eq!(state.snapshot().lines(), ["a b"]);
        assert!(state.screen().wide_cells_intact());
    }

    #[test]
    fn combining_mark_attaches_to_the_preceding_cell_and_does_not_advance() {
        let mut state = TerminalState::new(2, 8).expect("valid terminal");
        state.feed_bytes("e\u{0301}".as_bytes());

        assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("e\u{0301}"));
        assert_eq!(state.screen().cell(0, 0).map(Cell::width), Some(1));
        assert_eq!(state.cursor(), Cursor { row: 0, column: 1 });
        assert_eq!(state.snapshot().lines(), ["e\u{0301}"]);
    }

    #[test]
    fn combining_mark_without_a_preceding_cell_is_dropped() {
        let mut state = TerminalState::new(2, 8).expect("valid terminal");
        state.feed_bytes("\u{0301}x".as_bytes());

        assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("x"));
        assert_eq!(state.cursor(), Cursor { row: 0, column: 1 });
    }

    #[test]
    fn cursor_motion_never_rests_on_a_continuation_cell() {
        let mut state = TerminalState::new(3, 8).expect("valid terminal");
        state.feed_bytes("日a".as_bytes());
        assert_eq!(state.cursor(), Cursor { row: 0, column: 3 });

        // Left onto the continuation snaps onto the lead, Right skips the pair.
        state.move_cursor(CursorMove::Left(2));
        assert_eq!(state.cursor(), Cursor { row: 0, column: 0 });
        state.move_cursor(CursorMove::Right(1));
        assert_eq!(state.cursor(), Cursor { row: 0, column: 2 });

        // Absolute addressing onto the continuation column snaps to the lead.
        state.move_cursor(CursorMove::To { row: 0, column: 1 });
        assert_eq!(state.cursor(), Cursor { row: 0, column: 0 });

        // Backspace from just past the pair lands on the lead.
        state.move_cursor(CursorMove::To { row: 0, column: 2 });
        state.feed_bytes(b"\x08");
        assert_eq!(state.cursor(), Cursor { row: 0, column: 0 });

        // Vertical/absolute motion into a row whose target column is a
        // continuation resolves onto that row's lead.
        state.feed_bytes(b"\r\n \xe6\x97\xa5");
        state.move_cursor(CursorMove::To { row: 1, column: 2 });
        assert_eq!(state.cursor(), Cursor { row: 1, column: 1 });
        assert!(
            !state
                .screen()
                .cell(state.cursor().row(), state.cursor().column())
                .is_some_and(Cell::is_continuation)
        );
    }

    #[test]
    fn mixed_ascii_cjk_and_emoji_render_at_the_expected_columns() {
        let mut state = TerminalState::new(2, 10).expect("valid terminal");
        state.feed_bytes("a日😀b".as_bytes());

        let texts: Vec<&str> = (0..6)
            .map(|column| state.screen().cell(0, column).map_or("", Cell::text))
            .collect();
        assert_eq!(texts, ["a", "日", "", "\u{1F600}", "", "b"]);
        assert_eq!(row_widths(&state, 0), vec![1, 2, 0, 2, 0, 1, 1, 1, 1, 1]);
        assert_eq!(state.snapshot().lines(), ["a日😀b"]);
        assert_eq!(state.cursor(), Cursor { row: 0, column: 6 });
    }

    #[test]
    fn display_cells_match_display_lines_column_positions() {
        let mut state = TerminalState::new(2, 6).expect("valid terminal");
        state.feed_bytes("a日b".as_bytes());
        let snapshot = state.snapshot();

        // One cell per display column: the continuation keeps column 2, so
        // the cell index equals the placeholder character index of
        // display_lines and `b` sits at display column 3 in both.
        let cells: &[Cell] = snapshot.display_cells().next().expect("row");
        assert_eq!(
            cells.iter().map(|cell| cell.text()).collect::<Vec<_>>(),
            ["a", "日", "", "b", " ", " "]
        );
        assert!(!cells[0].is_continuation());
        assert!(!cells[1].is_continuation());
        assert!(cells[2].is_continuation());
        assert_eq!(snapshot.display_lines()[0].chars().count(), 4);
        assert_eq!(
            snapshot.display_lines()[0].chars().nth(3),
            Some('b'),
            "the cell index and the display-line character index agree"
        );
        // Row selection matches display_lines: identical counts at every
        // trailing-blank height.
        assert_eq!(
            snapshot.display_cells().len(),
            snapshot.display_lines().len()
        );
        let mut more = TerminalState::new(3, 6).expect("valid terminal");
        more.feed_bytes("x\r\n\r\n".as_bytes());
        assert_eq!(
            more.snapshot().display_cells().len(),
            more.snapshot().display_lines().len()
        );
    }

    #[test]
    fn display_cells_keep_attributes_through_continuations_and_trailing_blanks() {
        let mut state = TerminalState::new(3, 6).expect("valid terminal");
        // A printed blank keeps the pen's background; display_lines would
        // trim it, so cell access is the only way to see it.
        state.feed_bytes("\x1b[42ma日 \r\nX".as_bytes());
        let snapshot = state.snapshot();

        let row: Vec<CellAttributes> = snapshot
            .display_cells()
            .next()
            .expect("row")
            .iter()
            .map(|cell| *cell.attributes())
            .collect();
        // The green background covers the lead, its continuation, and the
        // printed blank, but not the untouched blanks ahead of the cursor.
        let green_background =
            CellAttributes::default().with_background(Color::Ansi(AnsiColor::Green));
        assert_eq!(row[0], green_background);
        assert_eq!(row[1], green_background);
        assert_eq!(row[2], green_background);
        assert_eq!(row[3], green_background);
        assert_eq!(row[4], CellAttributes::default());
        assert_eq!(row[5], CellAttributes::default());
        assert_eq!(snapshot.display_lines(), ["a日", "X"]);
        // Trailing blank cells within a row stay addressable, and row
        // selection still matches display_lines.
        assert_eq!(
            snapshot.display_cells().len(),
            snapshot.display_lines().len()
        );
    }

    #[test]
    fn display_rows_retain_a_background_only_space() {
        let mut state = TerminalState::new(1, 1).expect("valid terminal");
        state.feed_bytes(b"\x1b[48;2;73;18;146m ");
        let snapshot = state.snapshot();

        assert_eq!(state.screen().display_row_count(), 1);
        assert_eq!(snapshot.screen().display_row_count(), 1);
        assert_eq!(snapshot.display_lines(), [""]);
        assert_eq!(snapshot.display_cells().len(), 1);
        assert_eq!(snapshot.display_cells().next().unwrap()[0].text(), " ");
        assert!(
            !snapshot.display_cells().next().unwrap()[0]
                .attributes()
                .background()
                .is_default()
        );
    }

    #[test]
    fn utf8_split_across_feeds_still_decodes_and_invalid_bytes_are_dropped() {
        let mut state = TerminalState::new(2, 8).expect("valid terminal");
        state.feed_bytes(&[0xe6]);
        state.feed_bytes(&[0x97, 0xa5]);
        assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("日"));

        state.feed_bytes(&[0xff]);
        assert_eq!(state.cursor(), Cursor { row: 0, column: 2 });
    }

    #[test]
    fn resize_truncating_a_wide_character_blanks_the_orphaned_lead() {
        let mut state = TerminalState::new(2, 4).expect("valid terminal");
        state.feed_bytes("日".as_bytes());
        state.resize(2, 1).expect("valid resize");

        assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some(" "));
        assert_eq!(state.snapshot().lines(), [] as [String; 0]);
        assert!(state.screen().wide_cells_intact());
    }

    #[test]
    fn a_character_wider_than_the_grid_is_dropped() {
        let mut state = TerminalState::new(2, 1).expect("valid terminal");
        state.feed_bytes("日".as_bytes());

        assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some(" "));
        assert_eq!(state.cursor(), Cursor { row: 0, column: 0 });
        assert!(!state.is_wrap_pending());
    }
}
