use crate::parser::{Action, Parser};
use std::fmt;

/// Hard allocation bound for a visible Terminal Core screen.
pub const MAX_SCREEN_CELLS: usize = 1024 * 1024;

/// One basic terminal cell.
///
/// Text remains an owned string so later grapheme work does not require a
/// renderer-facing shape change. Terminal Core v1 writes one ASCII character
/// with width one into each cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    text: String,
    width: u8,
}

impl Cell {
    /// Construct a cell from display text and a precomputed column width.
    #[must_use]
    pub fn new(text: impl Into<String>, width: u8) -> Self {
        Self {
            text: text.into(),
            width,
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

    /// Whether this is the baseline blank cell.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.text == " "
    }

    fn from_ascii(byte: u8) -> Self {
        Self::new(char::from(byte).to_string(), 1)
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
        if top >= bottom || bottom >= rows {
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
        Ok(())
    }

    fn scroll_up(&mut self, region: ScrollRegion, count: u16) {
        let columns = usize::from(self.cols);
        let rows = usize::from(count.min(region.height()));
        let start = usize::from(region.top) * columns;
        let end = (usize::from(region.bottom) + 1) * columns;
        let shift = rows * columns;
        self.cells[start..end].rotate_left(shift);
        self.cells[end - shift..end].fill(Cell::blank());
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

/// Noren-owned mutable terminal state.
pub struct TerminalState {
    screen: ScreenBuffer,
    cursor: Cursor,
    scroll_region: ScrollRegion,
    wrap_pending: bool,
    parser: Parser,
}

impl TerminalState {
    /// Create a bounded non-zero visible terminal.
    pub fn new(rows: u16, cols: u16) -> Result<Self, TerminalError> {
        Ok(Self {
            screen: ScreenBuffer::new(rows, cols)?,
            cursor: Cursor::default(),
            scroll_region: ScrollRegion::full_screen(rows),
            wrap_pending: false,
            parser: Parser::default(),
        })
    }

    /// Apply PTY bytes in order.
    ///
    /// Non-ASCII bytes and unsupported control sequences are ignored in this
    /// foundation. They are never interpreted as authority or rendered as raw
    /// escape-sequence payload.
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            if let Some(action) = self.parser.advance(*byte) {
                self.apply(action);
            }
        }
    }

    /// Resize while preserving the overlapping top-left screen area.
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        self.screen.resize(rows, cols)?;
        self.cursor.row = self.cursor.row.min(rows - 1);
        self.cursor.column = self.cursor.column.min(cols - 1);
        self.scroll_region = ScrollRegion::full_screen(rows);
        self.wrap_pending = false;
        Ok(())
    }

    /// Current `(rows, cols)`.
    #[must_use]
    pub const fn size(&self) -> (u16, u16) {
        (self.screen.rows, self.screen.cols)
    }

    /// Current cursor position.
    #[must_use]
    pub const fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// Current visible screen.
    #[must_use]
    pub const fn screen(&self) -> &ScreenBuffer {
        &self.screen
    }

    /// Active inclusive vertical scrolling margins.
    #[must_use]
    pub const fn scroll_region(&self) -> ScrollRegion {
        self.scroll_region
    }

    /// Whether the next printable byte will wrap before it is written.
    #[must_use]
    pub const fn is_wrap_pending(&self) -> bool {
        self.wrap_pending
    }

    /// Set zero-based inclusive scrolling margins and move the cursor home.
    pub fn set_scroll_region(&mut self, top: u16, bottom: u16) -> Result<(), TerminalError> {
        self.scroll_region = ScrollRegion::checked(self.screen.rows, top, bottom)?;
        self.cursor = Cursor::default();
        self.wrap_pending = false;
        Ok(())
    }

    /// Apply a clamped renderer-independent cursor operation.
    pub fn move_cursor(&mut self, movement: CursorMove) {
        self.wrap_pending = false;
        let last_row = self.screen.rows - 1;
        let last_column = self.screen.cols - 1;
        match movement {
            CursorMove::Up(count) => self.cursor.row = self.cursor.row.saturating_sub(count),
            CursorMove::Down(count) => {
                self.cursor.row = self.cursor.row.saturating_add(count).min(last_row);
            }
            CursorMove::Right(count) => {
                self.cursor.column = self.cursor.column.saturating_add(count).min(last_column);
            }
            CursorMove::Left(count) => {
                self.cursor.column = self.cursor.column.saturating_sub(count);
            }
            CursorMove::NextLine(count) => {
                self.cursor.row = self.cursor.row.saturating_add(count).min(last_row);
                self.cursor.column = 0;
            }
            CursorMove::PreviousLine(count) => {
                self.cursor.row = self.cursor.row.saturating_sub(count);
                self.cursor.column = 0;
            }
            CursorMove::To { row, column } => {
                self.cursor.row = row.min(last_row);
                self.cursor.column = column.min(last_column);
            }
            CursorMove::ToColumn(column) => {
                self.cursor.column = column.min(last_column);
            }
            CursorMove::ToRow(row) => {
                self.cursor.row = row.min(last_row);
            }
        }
    }

    /// Clone a bounded immutable renderer/test view.
    #[must_use]
    pub fn snapshot(&self) -> TerminalSnapshot {
        TerminalSnapshot::from_state(self)
    }

    fn apply(&mut self, action: Action) {
        match action {
            Action::Print(byte) => self.print_ascii(byte),
            Action::LineFeed => self.line_feed(),
            Action::CarriageReturn => {
                self.cursor.column = 0;
                self.wrap_pending = false;
            }
            Action::Backspace => {
                self.cursor.column = self.cursor.column.saturating_sub(1);
                self.wrap_pending = false;
            }
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
        }
    }

    fn print_ascii(&mut self, byte: u8) {
        if self.wrap_pending {
            self.cursor.column = 0;
            self.index();
        }
        self.screen
            .put(self.cursor.row, self.cursor.column, Cell::from_ascii(byte));
        if self.cursor.column == self.screen.cols - 1 {
            self.wrap_pending = true;
        } else {
            self.cursor.column += 1;
        }
    }

    fn line_feed(&mut self) {
        self.wrap_pending = false;
        self.index();
    }

    fn index(&mut self) {
        self.wrap_pending = false;
        if self.cursor.row == self.scroll_region.bottom {
            self.screen.scroll_up(self.scroll_region, 1);
        } else if self.cursor.row < self.screen.rows - 1 {
            self.cursor.row += 1;
        }
    }

    fn next_line(&mut self) {
        self.cursor.column = 0;
        self.index();
    }

    fn reverse_index(&mut self) {
        self.wrap_pending = false;
        if self.cursor.row == self.scroll_region.top {
            self.screen.scroll_down(self.scroll_region, 1);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    fn scroll_up(&mut self, count: u16) {
        self.wrap_pending = false;
        self.screen.scroll_up(self.scroll_region, count);
    }

    fn scroll_down(&mut self, count: u16) {
        self.wrap_pending = false;
        self.screen.scroll_down(self.scroll_region, count);
    }

    fn reset_scroll_region(&mut self) {
        self.scroll_region = ScrollRegion::full_screen(self.screen.rows);
        self.cursor = Cursor::default();
        self.wrap_pending = false;
    }

    fn apply_scroll_region(&mut self, top: u16, bottom: Option<u16>) {
        if top == 0 && bottom.is_none() {
            self.reset_scroll_region();
        } else {
            let bottom = bottom.unwrap_or(self.screen.rows - 1);
            let _ = self.set_scroll_region(top, bottom);
        }
    }
}

impl fmt::Debug for TerminalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalState")
            .field("size", &self.size())
            .field("cursor", &self.cursor)
            .field("scroll_region", &self.scroll_region)
            .field("wrap_pending", &self.wrap_pending)
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
    lines: Vec<String>,
}

impl TerminalSnapshot {
    fn from_state(state: &TerminalState) -> Self {
        Self {
            screen: state.screen.clone(),
            cursor: state.cursor,
            scroll_region: state.scroll_region,
            wrap_pending: state.wrap_pending,
            lines: visible_lines(&state.screen),
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

fn visible_lines(screen: &ScreenBuffer) -> Vec<String> {
    let mut lines = Vec::with_capacity(usize::from(screen.rows));
    for row in 0..screen.rows {
        let mut line = String::new();
        for column in 0..screen.cols {
            if let Some(cell) = screen.cell(row, column) {
                line.push_str(cell.text());
            }
        }
        while line.ends_with(' ') {
            line.pop();
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
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
}
