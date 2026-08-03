use noren_terminal::{Cell, CursorMove, MAX_SCREEN_CELLS, TerminalError, TerminalState};

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

#[test]
fn rejects_zero_and_over_limit_sizes_without_changing_state() {
    assert!(matches!(
        TerminalState::new(0, 1),
        Err(TerminalError::InvalidSize)
    ));
    assert!(matches!(
        TerminalState::new(1, 0),
        Err(TerminalError::InvalidSize)
    ));

    let over_limit_rows = u16::try_from(MAX_SCREEN_CELLS / 1024 + 1).expect("bounded row count");
    assert!(matches!(
        TerminalState::new(over_limit_rows, 1024),
        Err(TerminalError::ScreenTooLarge)
    ));

    let mut state = TerminalState::new(2, 2).expect("valid terminal");
    state.feed_bytes(b"A");
    assert_eq!(state.resize(0, 2), Err(TerminalError::InvalidSize));
    assert_eq!(state.size(), (2, 2));
    assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("A"));
}

#[test]
fn resize_preserves_overlap_and_clamps_cursor() {
    let mut state = TerminalState::new(3, 4).expect("valid terminal");
    state.feed_bytes(b"AB\x1b[2;3HZ");
    state.move_cursor(CursorMove::To { row: 2, column: 3 });

    state.resize(2, 3).expect("valid resize");
    assert_eq!(state.size(), (2, 3));
    assert_eq!(cursor(&state), (1, 2));
    assert_eq!(state.snapshot().lines(), ["AB", "  Z"]);

    state.resize(4, 5).expect("valid resize");
    assert_eq!(state.snapshot().lines(), ["AB", "  Z"]);
    assert_eq!(state.screen().cells().len(), 20);
}

#[test]
fn printable_ascii_lf_cr_and_backspace_remain_stable() {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    state.feed_bytes(b"AB\nC\rD\x08E");

    assert_eq!(state.snapshot().lines(), ["AB", "E C"]);
    assert_eq!(cursor(&state), (1, 1));
}

#[test]
fn cursor_commands_apply_defaults_absolute_positions_and_clamping() {
    let mut state = TerminalState::new(3, 4).expect("valid terminal");
    state.move_cursor(CursorMove::To { row: 1, column: 1 });

    state.feed_bytes(b"\x1b[A");
    assert_eq!(cursor(&state), (0, 1));
    state.feed_bytes(b"\x1b[D");
    assert_eq!(cursor(&state), (0, 0));
    state.feed_bytes(b"\x1b[B\x1b[C");
    assert_eq!(cursor(&state), (1, 1));
    state.feed_bytes(b"\x1b[99B\x1b[99C");
    assert_eq!(cursor(&state), (2, 3));
    state.feed_bytes(b"\x1b[99A\x1b[99D");
    assert_eq!(cursor(&state), (0, 0));
    state.feed_bytes(b"\x1b[2;3H");
    assert_eq!(cursor(&state), (1, 2));
    state.feed_bytes(b"\x1b[H");
    assert_eq!(cursor(&state), (0, 0));
}

#[test]
fn wrapping_at_the_bottom_scrolls_one_row_and_blanks_the_last() {
    let mut state = TerminalState::new(2, 3).expect("valid terminal");
    state.feed_bytes(b"abc");
    assert_eq!(state.snapshot().lines(), ["abc"]);
    assert_eq!(cursor(&state), (1, 0));

    state.feed_bytes(b"def");
    assert_eq!(state.snapshot().lines(), ["def"]);
    assert_eq!(cursor(&state), (1, 0));
    assert!(state.screen().cell(1, 0).is_some_and(Cell::is_blank));
}

#[test]
fn split_sequences_are_retained_and_unsupported_bytes_do_not_leak() {
    let mut state = TerminalState::new(3, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b");
    state.feed_bytes(b"[2;");
    state.feed_bytes(b"3H");
    assert_eq!(cursor(&state), (1, 2));

    state.feed_bytes(b"\x1b[31m\xff\xc3\xa9");
    state.feed_bytes(b"Q");
    assert_eq!(state.snapshot().lines(), ["", "  Q"]);
}

#[test]
fn snapshots_are_immutable_and_cells_are_row_major() {
    let mut state = TerminalState::new(2, 2).expect("valid terminal");
    state.feed_bytes(b"AB");
    let before = state.snapshot();

    state.feed_bytes(b"C");
    assert_eq!(before.lines(), ["AB"]);
    assert_eq!(state.snapshot().lines(), ["AB", "C"]);

    let texts: Vec<_> = state.screen().cells().iter().map(Cell::text).collect();
    assert_eq!(texts, ["A", "B", "C", " "]);
    assert!(state.screen().cell(2, 0).is_none());
    assert!(state.screen().cell(0, 2).is_none());
}
