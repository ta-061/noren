//! Renderer-independent terminal state and bounded VT/ANSI parsing foundation.
//!
//! [`TerminalState`] owns the screen, cursor, dimensions, and parser state.
//! PTY bytes enter through [`TerminalEngine::feed_bytes`]; renderers receive an
//! immutable [`TerminalSnapshot`] and never depend on PTY or parser types.
//!
//! This foundation supports a bounded ASCII/CSI subset, basic SGR attributes,
//! scrolling regions, cursor save/restore, and DEC private mode 1049 screen
//! switching. It is not a VT100/xterm compatibility claim.

mod attributes;
mod parser;
mod state;

pub use attributes::{AnsiColor, CellAttributes, Color};
pub use state::{
    Cell, Cursor, CursorMove, MAX_SCREEN_CELLS, MAX_SCROLLBACK_LINES, ScreenBuffer, ScrollRegion,
    TerminalError, TerminalModes, TerminalSnapshot, TerminalState,
};

use unicode_width::UnicodeWidthChar;

/// Terminal state contract: bytes and dimensions in, immutable snapshot out.
///
/// The trait carries no window, GPU, PTY, or third-party parser types, keeping
/// the core replaceable and directly testable.
pub trait TerminalEngine {
    /// Feed non-authoritative PTY output bytes into terminal state.
    fn feed_bytes(&mut self, bytes: &[u8]);

    /// Resize the visible grid while preserving the overlapping top-left area.
    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError>;

    /// Current grid size as `(rows, cols)`.
    fn size(&self) -> (u16, u16);

    /// Take a bounded immutable snapshot for a renderer or test oracle.
    fn snapshot(&self) -> TerminalSnapshot;
}

impl TerminalEngine for TerminalState {
    fn feed_bytes(&mut self, bytes: &[u8]) {
        Self::feed_bytes(self, bytes);
    }

    fn resize(&mut self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        Self::resize(self, rows, cols)
    }

    fn size(&self) -> (u16, u16) {
        Self::size(self)
    }

    fn snapshot(&self) -> TerminalSnapshot {
        Self::snapshot(self)
    }
}

/// Current Unicode column-width seed for future non-ASCII cell handling.
///
/// Terminal Core v1 writes printable ASCII only. This policy remains public so
/// later grapheme/ambiguous-width work can evolve behind the same boundary.
#[must_use]
pub fn cell_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_is_usable_through_the_renderer_independent_trait() {
        let mut engine: Box<dyn TerminalEngine> =
            Box::new(TerminalState::new(2, 4).expect("valid terminal"));
        engine.feed_bytes(b"ok");
        engine.resize(4, 8).expect("valid resize");

        assert_eq!(engine.size(), (4, 8));
        let snapshot = engine.snapshot();
        assert_eq!((snapshot.rows(), snapshot.cols()), (4, 8));
        assert_eq!(snapshot.lines(), ["ok".to_owned()]);
    }

    #[test]
    fn cell_width_keeps_the_future_unicode_policy_separate() {
        assert_eq!(cell_width('A'), 1);
        assert_eq!(cell_width(' '), 1);
        assert_eq!(cell_width('\u{5168}'), 2);
        assert_eq!(cell_width('\u{0}'), 0);
    }
}
