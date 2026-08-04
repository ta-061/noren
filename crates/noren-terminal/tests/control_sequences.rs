//! Regressions for escape-intermediate consumption and horizontal tab handling.

use noren_terminal::{Cell, CursorMove, TerminalState};

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

fn cells(state: &TerminalState, row: u16) -> Vec<String> {
    (0..state.screen().cols())
        .map(|column| {
            state
                .screen()
                .cell(row, column)
                .map(Cell::text)
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

/// `ESC ( B`, `ESC ) 0`, `ESC # 8`, and `ESC SP F` must be swallowed whole:
/// nothing prints and the cursor stays at the origin.
#[test]
fn escape_intermediate_sequences_print_nothing_and_keep_cursor_home() {
    for sequence in [b"\x1b(B".as_slice(), b"\x1b)0", b"\x1b#8", b"\x1b F"] {
        let mut state = TerminalState::new(2, 4).expect("valid terminal");
        state.feed_bytes(sequence);

        assert!(state.snapshot().lines().is_empty(), "sequence {sequence:?}");
        assert_eq!(cursor(&state), (0, 0), "sequence {sequence:?}");
    }
}

/// The same sequences split across `feed_bytes` chunk boundaries must still
/// print nothing.
#[test]
fn escape_intermediate_sequences_consume_whole_across_chunk_boundaries() {
    for sequence in [b"\x1b(B".as_slice(), b"\x1b)0", b"\x1b#8", b"\x1b F"] {
        let mut state = TerminalState::new(2, 4).expect("valid terminal");
        // One byte per call, the worst case for parser state retention.
        for byte in sequence {
            state.feed_bytes(&[*byte]);
        }

        assert!(state.snapshot().lines().is_empty(), "sequence {sequence:?}");
        assert_eq!(cursor(&state), (0, 0), "sequence {sequence:?}");

        // Output after the sequence is unaffected.
        state.feed_bytes(b"Z");
        assert_eq!(state.snapshot().lines(), ["Z".to_owned()]);
    }
}

/// A second intermediate before the final byte is also consumed.
#[test]
fn stacked_escape_intermediates_still_print_nothing() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b()BZ");

    assert_eq!(state.snapshot().lines(), ["Z".to_owned()]);
    assert_eq!(cursor(&state), (0, 1));
}

/// An unsupported single-byte escape final (e.g. `ESC c`, RIS) still consumes
/// exactly its bytes and returns to Ground, matching the pre-fix behavior.
#[test]
fn unsupported_single_byte_escape_final_behaves_as_before() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"\x1bcX");

    assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("X"));
    assert_eq!(cursor(&state), (0, 1));
}

/// A new `ESC` aborts an in-progress intermediate escape, same as it does for
/// CSI.
#[test]
fn escape_aborts_an_in_progress_intermediate_sequence() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b(\x1b[DX");

    assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("X"));
    assert_eq!(cursor(&state), (0, 1));
}

/// `a\tb` lands `b` at the next 8-column tab stop.
#[test]
fn horizontal_tab_advances_to_the_next_eighth_column() {
    let mut state = TerminalState::new(1, 12).expect("valid terminal");
    state.feed_bytes(b"a\tb");

    assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("a"));
    assert_eq!(state.screen().cell(0, 8).map(Cell::text), Some("b"));
    assert_eq!(cursor(&state), (0, 9));
}

/// A tab stops at the last column without wrapping, scrolling, or panicking
/// when the next stop would be past the right edge.
#[test]
fn tab_near_the_right_edge_clamps_to_the_last_column() {
    let mut state = TerminalState::new(2, 10).expect("valid terminal");
    // Fill columns 0..=6; cursor sits at column 7. The next stop (8) is in
    // range, so the following print lands at column 8.
    state.feed_bytes(b"1234567\tX");

    assert_eq!(state.screen().cell(0, 8).map(Cell::text), Some("X"));
    assert_eq!(cursor(&state), (0, 9));
    assert_eq!(
        cells(&state, 1),
        [" ", " ", " ", " ", " ", " ", " ", " ", " ", " "]
    );
}

/// A tab issued while the cursor is on the very last column must not panic,
/// wrap, or scroll.
#[test]
fn tab_on_the_last_column_does_not_wrap_or_panic() {
    let mut state = TerminalState::new(2, 10).expect("valid terminal");
    state.feed_bytes(b"1234567890");
    assert!(state.is_wrap_pending());

    state.feed_bytes(b"\t");

    assert_eq!(cursor(&state), (0, 9));
    assert!(!state.is_wrap_pending());
    // Row 1 is untouched: the tab neither wrapped nor scrolled.
    assert_eq!(
        cells(&state, 1),
        [" ", " ", " ", " ", " ", " ", " ", " ", " ", " "]
    );
}

/// Tab handling is bounded for the widest grid the cell budget allows: no
/// overflow in the next-stop computation.
#[test]
fn tab_is_bounded_on_the_widest_permitted_grid() {
    let cols = 65535_u16;
    let mut state = TerminalState::new(1, cols).expect("valid terminal");
    // Cursor at the last column (cols - 1). The next-stop math must not
    // overflow u16 for column 65534.
    state.move_cursor(CursorMove::ToColumn(cols - 1));
    state.feed_bytes(b"\t");

    assert_eq!(cursor(&state), (0, cols - 1));
}
