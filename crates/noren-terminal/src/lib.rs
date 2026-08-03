//! Noren-owned terminal state, cell-width, and bounded snapshot contracts.
//!
//! This crate owns the [`TerminalEngine`] contract from the
//! [minimum architecture](https://github.com/ta-061/noren/blob/main/docs/architecture/minimal-local-pty-poc.md):
//! bytes and dimensions in; bounded snapshot out. Replies, damage, and a
//! stricter byte budget are wired by the drain loop in a later step.
//!
//! The PoC trials `avt` 0.18.0 behind [`AvtEngine`]. Passing this baseline
//! makes no terminal-compatibility claim: the adapter is a replaceable
//! candidate, full VT/xterm semantics remain deferred, and the byte-to-text
//! boundary here uses a lossy placeholder until the streaming UTF-8 buffer
//! lands. `unicode-width` 0.2.2 is the direct cell-width policy seed; `avt`
//! itself uses an older `unicode-width` internally and the two coexist.

use unicode_width::UnicodeWidthChar;

/// One grid cell in a [`TerminalSnapshot`].
///
/// `width` follows the [`cell_width`] policy so the renderer can reserve the
/// correct number of columns for wide and combining glyphs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    text: String,
    width: u8,
}

impl Cell {
    /// Build a cell from its display text and precomputed column width.
    #[must_use]
    pub fn new(text: impl Into<String>, width: u8) -> Self {
        Self {
            text: text.into(),
            width,
        }
    }

    /// The cell's display text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cell's column width.
    #[must_use]
    pub fn width(&self) -> u8 {
        self.width
    }
}

/// A bounded, immutable view of the terminal grid.
///
/// `lines` is bounded by the current grid plus retained scrollback; a stricter
/// per-turn byte budget is applied by the main-loop drain layer, not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSnapshot {
    rows: u16,
    cols: u16,
    lines: Vec<String>,
}

impl TerminalSnapshot {
    /// Number of rows in the grid.
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Number of columns in the grid.
    #[must_use]
    pub fn cols(&self) -> u16 {
        self.cols
    }

    /// The snapshot lines, oldest first.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }
}

/// Terminal state contract: bytes and dimensions in, bounded snapshot out.
///
/// The trait carries no third-party types so a candidate library can be
/// replaced behind it without leaking its API across crate boundaries.
pub trait TerminalEngine {
    /// Feed PTY output bytes to the terminal. Bytes are non-authoritative.
    fn feed_bytes(&mut self, bytes: &[u8]);

    /// Resize the grid. Noren convention is rows-first; implementations map to
    /// their candidate's ordering.
    fn resize(&mut self, rows: u16, cols: u16);

    /// Current grid size as `(rows, cols)`.
    fn size(&self) -> (u16, u16);

    /// Take a bounded immutable snapshot of the grid.
    fn snapshot(&self) -> TerminalSnapshot;
}

/// Cell-width policy seed.
///
/// Returns the Unicode column width for a single char, or `0` for control
/// characters. Grapheme clustering and the explicit ambiguous-width policy are
/// deferred; this is the minimal seed that exercises `unicode-width` 0.2.2.
#[must_use]
pub fn cell_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

/// `avt` 0.18.0 candidate adapter behind [`TerminalEngine`].
///
/// Provisional and replaceable: the byte boundary here is a lossy placeholder
/// (a streaming UTF-8 buffer lands later) and no compatibility is claimed.
pub struct AvtEngine {
    vt: avt::Vt,
}

impl AvtEngine {
    /// Create an engine sized to `rows` x `cols`.
    #[must_use]
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            vt: avt::Vt::new(usize::from(cols), usize::from(rows)),
        }
    }
}

impl TerminalEngine for AvtEngine {
    fn feed_bytes(&mut self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        // avt applies all operations eagerly inside feed_str; the returned
        // change record is not needed for the baseline snapshot path.
        let _ = self.vt.feed_str(&text);
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        let _ = self.vt.resize(usize::from(cols), usize::from(rows));
    }

    fn size(&self) -> (u16, u16) {
        to_rows_cols(self.vt.size())
    }

    fn snapshot(&self) -> TerminalSnapshot {
        let (rows, cols) = self.size();
        TerminalSnapshot {
            rows,
            cols,
            lines: trim_trailing_empty(self.vt.text()),
        }
    }
}

/// Drop trailing empty grid rows from a snapshot's line buffer.
///
/// Grid dimensions (`rows`/`cols`) keep reporting the full allocated grid; the
/// line buffer reports only rows that carry content, so a bounded snapshot never
/// pays the byte budget for unused trailing rows.
fn trim_trailing_empty(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

fn to_rows_cols(avt_size: (usize, usize)) -> (u16, u16) {
    let (cols, rows) = avt_size;
    (
        u16::try_from(rows).unwrap_or(u16::MAX),
        u16::try_from(cols).unwrap_or(u16::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_carries_text_and_width() {
        let cell = Cell::new("A", 1);
        assert_eq!(cell.text(), "A");
        assert_eq!(cell.width(), 1);
    }

    #[test]
    fn cell_width_reports_expected_columns() {
        assert_eq!(cell_width('A'), 1);
        assert_eq!(cell_width(' '), 1);
        // CJK ideograph is East Asian Wide (width 2).
        assert_eq!(cell_width('\u{5168}'), 2);
        // Control characters report no width.
        assert_eq!(cell_width('\u{0}'), 0);
    }

    #[test]
    fn avt_engine_records_text_and_dimensions() {
        let mut engine = AvtEngine::new(3, 5);
        engine.feed_bytes(b"hi");
        let snapshot = engine.snapshot();
        assert_eq!((snapshot.rows(), snapshot.cols()), (3, 5));
        assert_eq!(snapshot.lines(), ["hi".to_owned()]);
    }

    #[test]
    fn avt_engine_resize_updates_size() {
        let mut engine = AvtEngine::new(3, 5);
        engine.resize(10, 20);
        assert_eq!(engine.size(), (10, 20));
        let snapshot = engine.snapshot();
        assert_eq!((snapshot.rows(), snapshot.cols()), (10, 20));
    }

    #[test]
    fn engine_is_usable_through_the_trait() {
        let mut engine: Box<dyn TerminalEngine> = Box::new(AvtEngine::new(2, 4));
        engine.feed_bytes(b"ok");
        engine.resize(4, 8);
        assert_eq!(engine.size(), (4, 8));
        let snapshot = engine.snapshot();
        assert_eq!((snapshot.rows(), snapshot.cols()), (4, 8));
        assert!(
            snapshot
                .lines()
                .first()
                .is_some_and(|line| line.contains("ok"))
        );
    }
}
