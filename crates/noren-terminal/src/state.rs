use crate::{
    attributes::{AnsiColor, CellAttributes, Color},
    parser::{Action, EraseMode, Parser, PrivateMode},
};
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

    fn from_ascii(byte: u8, attributes: CellAttributes) -> Self {
        Self {
            text: char::from(byte).to_string(),
            width: 1,
            attributes,
        }
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

/// Renderer-independent modes that affect screen selection or encoded input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalModes {
    alternate_screen: bool,
    application_cursor_key: bool,
    application_keypad: bool,
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

    fn erase_in_display(&mut self, cursor: Cursor, mode: EraseMode) {
        let Some(cursor_index) = self.index(cursor.row, cursor.column) else {
            return;
        };
        match mode {
            EraseMode::ToEnd => self.cells[cursor_index..].fill(Cell::default()),
            EraseMode::ToBeginning => self.cells[..cursor_index + 1].fill(Cell::default()),
            EraseMode::All => self.cells.fill(Cell::default()),
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
    }

    fn erase_characters(&mut self, cursor: Cursor, count: u16) {
        let Some(start) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let row_end = (usize::from(cursor.row) + 1) * usize::from(self.cols);
        let count = usize::from(count).min(row_end - start);
        self.cells[start..start + count].fill(Cell::default());
    }

    fn insert_characters(&mut self, cursor: Cursor, count: u16) {
        let Some(start) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let row_end = (usize::from(cursor.row) + 1) * usize::from(self.cols);
        let count = usize::from(count).min(row_end - start);
        self.cells[start..row_end].rotate_right(count);
        self.cells[start..start + count].fill(Cell::default());
    }

    fn delete_characters(&mut self, cursor: Cursor, count: u16) {
        let Some(start) = self.index(cursor.row, cursor.column) else {
            return;
        };
        let row_end = (usize::from(cursor.row) + 1) * usize::from(self.cols);
        let count = usize::from(count).min(row_end - start);
        self.cells[start..row_end].rotate_left(count);
        self.cells[row_end - count..row_end].fill(Cell::default());
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
        self.cursor = clamp_cursor(self.cursor, rows, cols);
        self.saved_cursor = self
            .saved_cursor
            .map(|cursor| clamp_cursor(cursor, rows, cols));
        self.scroll_region = ScrollRegion::full_screen(rows);
        self.wrap_pending = false;
        Ok(())
    }

    fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor);
    }

    fn restore_cursor(&mut self) {
        if let Some(cursor) = self.saved_cursor {
            self.cursor = clamp_cursor(cursor, self.screen.rows, self.screen.cols);
        }
        self.wrap_pending = false;
    }
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
    pub fn move_cursor(&mut self, movement: CursorMove) {
        self.active.wrap_pending = false;
        let last_row = self.active.screen.rows - 1;
        let last_column = self.active.screen.cols - 1;
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
                self.active.cursor.column = 0;
                self.active.wrap_pending = false;
            }
            Action::Backspace => {
                self.active.cursor.column = self.active.cursor.column.saturating_sub(1);
                self.active.wrap_pending = false;
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
            Action::EraseInDisplay(mode) => self.erase_in_display(mode),
            Action::EraseInLine(mode) => self.erase_in_line(mode),
            Action::EraseCharacters(count) => self.erase_characters(count),
            Action::InsertCharacters(count) => self.insert_characters(count),
            Action::DeleteCharacters(count) => self.delete_characters(count),
            Action::InsertLines(count) => self.insert_lines(count),
            Action::DeleteLines(count) => self.delete_lines(count),
            Action::SelectGraphicRendition { params, len } => {
                self.select_graphic_rendition(&params[..len]);
            }
            Action::SaveCursor => self.save_cursor(),
            Action::RestoreCursor => self.restore_cursor(),
            Action::SetKeypadApplication(enabled) => {
                self.modes.application_keypad = enabled;
            }
            Action::SetPrivateMode { mode, enabled } => self.set_private_mode(mode, enabled),
        }
    }

    fn print_ascii(&mut self, byte: u8) {
        if self.active.wrap_pending {
            self.active.cursor.column = 0;
            self.index();
        }
        self.active.screen.put(
            self.active.cursor.row,
            self.active.cursor.column,
            Cell::from_ascii(byte, self.pen),
        );
        if self.active.cursor.column == self.active.screen.cols - 1 {
            self.active.wrap_pending = true;
        } else {
            self.active.cursor.column += 1;
        }
    }

    fn line_feed(&mut self) {
        self.active.wrap_pending = false;
        self.index();
    }

    fn index(&mut self) {
        self.active.wrap_pending = false;
        if self.active.cursor.row == self.active.scroll_region.bottom {
            self.active.screen.scroll_up(self.active.scroll_region, 1);
        } else if self.active.cursor.row < self.active.screen.rows - 1 {
            self.active.cursor.row += 1;
        }
    }

    fn next_line(&mut self) {
        self.active.cursor.column = 0;
        self.index();
    }

    fn reverse_index(&mut self) {
        self.active.wrap_pending = false;
        if self.active.cursor.row == self.active.scroll_region.top {
            self.active.screen.scroll_down(self.active.scroll_region, 1);
        } else if self.active.cursor.row > 0 {
            self.active.cursor.row -= 1;
        }
    }

    fn scroll_up(&mut self, count: u16) {
        self.active.wrap_pending = false;
        self.active
            .screen
            .scroll_up(self.active.scroll_region, count);
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
            self.active.screen.scroll_up(
                ScrollRegion {
                    top: cursor_row,
                    bottom: region.bottom,
                },
                count,
            );
        }
    }

    fn select_graphic_rendition(&mut self, params: &[u16]) {
        let mut index = 0;
        while index < params.len() {
            let param = params[index];
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
                // Indexed, direct, and underline colors are deferred. Consume
                // their semicolon-form arguments as one unsupported group so
                // channel values such as 1, 4, or 7 are never reinterpreted as
                // independent bold/underline/reverse controls.
                38 | 48 | 58 => {
                    index += extended_color_parameter_count(&params[index..]);
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

fn extended_color_parameter_count(params: &[u16]) -> usize {
    match params.get(1) {
        Some(5) => params.len().min(3),
        Some(2) => params.len().min(5),
        _ => 1,
    }
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
