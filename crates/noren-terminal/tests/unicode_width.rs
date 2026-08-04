//! Display-width behavior for wide, zero-width, and mixed-width printing.

use noren_terminal::{Cell, CursorMove, TerminalState};

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

fn cell(state: &TerminalState, row: u16, column: u16) -> &Cell {
    state.screen().cell(row, column).expect("cell is in bounds")
}

#[test]
fn ascii_line_is_unchanged_and_every_cell_keeps_width_one() {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    state.feed_bytes(b"ab\rZ\nQ\x08R");

    assert_eq!(state.snapshot().lines(), ["Zb", " R"]);
    assert_eq!(cursor(&state), (1, 2));
    assert!(
        state
            .screen()
            .cells()
            .iter()
            .all(|cell| cell.width() == 1 && !cell.is_continuation())
    );
}

#[test]
fn cjk_characters_occupy_two_cells_with_a_continuation() {
    let mut state = TerminalState::new(2, 8).expect("valid terminal");
    state.feed_bytes("你好".as_bytes());

    assert_eq!(state.snapshot().lines(), ["你好"]);
    assert_eq!(cell(&state, 0, 0).text(), "你");
    assert_eq!(cell(&state, 0, 0).width(), 2);
    assert!(cell(&state, 0, 1).is_continuation());
    assert_eq!(cell(&state, 0, 1).text(), "");
    assert_eq!(cell(&state, 0, 1).width(), 0);
    assert_eq!(cell(&state, 0, 2).text(), "好");
    assert_eq!(cell(&state, 0, 2).width(), 2);
    assert!(cell(&state, 0, 3).is_continuation());
    assert_eq!(cursor(&state), (0, 4));
}

#[test]
fn wide_character_flush_against_the_right_edge_defers_wrap_on_its_lead() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"ab");
    state.feed_bytes("你".as_bytes());

    assert_eq!(state.snapshot().lines(), ["ab你"]);
    // The cursor rests on the lead cell, never on the continuation.
    assert_eq!(cursor(&state), (0, 2));
    assert!(state.is_wrap_pending());

    state.feed_bytes(b"X");
    assert_eq!(state.snapshot().lines(), ["ab你", "X"]);
    assert_eq!(cursor(&state), (1, 1));
}

#[test]
fn wide_character_that_does_not_fit_wraps_whole_instead_of_splitting() {
    let mut state = TerminalState::new(2, 5).expect("valid terminal");
    state.feed_bytes(b"abcd");
    assert_eq!(cursor(&state), (0, 4));

    state.feed_bytes("你".as_bytes());

    assert_eq!(state.snapshot().lines(), ["abcd", "你"]);
    assert_eq!(cursor(&state), (1, 2));
    assert!(!state.is_wrap_pending());
    assert!(cell(&state, 1, 0).text() == "你" && cell(&state, 1, 1).is_continuation());
}

#[test]
fn overwriting_half_of_a_wide_character_clears_both_halves() {
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("你A".as_bytes());

    // Positioning onto the continuation column snaps to the lead cell, and
    // overwriting the lead clears the continuation as well.
    state.move_cursor(CursorMove::ToColumn(1));
    assert_eq!(cursor(&state), (0, 0));
    state.feed_bytes(b"X");

    assert_eq!(state.snapshot().lines(), ["X A"]);
    assert!(!cell(&state, 0, 0).is_continuation());
    assert!(!cell(&state, 0, 1).is_continuation());
    assert_eq!(cell(&state, 0, 1).text(), " ");
}

#[test]
fn erasing_half_of_a_wide_character_clears_both_halves() {
    // ECH covering only the lead leaves no dangling continuation.
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("你A".as_bytes());
    state.move_cursor(CursorMove::ToColumn(0));
    state.feed_bytes(b"\x1b[1X");
    // The erased lead and its orphaned continuation both blank in place;
    // ECH never shifts the tail left.
    assert_eq!(state.snapshot().lines(), ["  A"]);
    assert!(
        state
            .screen()
            .cells()
            .iter()
            .all(|cell| !cell.is_continuation())
    );

    // EL to the end starting on the lead clears the whole pair.
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("a你b".as_bytes());
    state.move_cursor(CursorMove::ToColumn(1));
    state.feed_bytes(b"\x1b[0K");
    assert_eq!(state.snapshot().lines(), ["a"]);

    // EL to the beginning ending on the lead clears the orphaned
    // continuation as well.
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("a你b".as_bytes());
    state.move_cursor(CursorMove::ToColumn(1));
    state.feed_bytes(b"\x1b[1K");
    assert_eq!(state.snapshot().lines(), ["   b"]);
    assert!(
        state
            .screen()
            .cells()
            .iter()
            .all(|cell| !cell.is_continuation())
    );
}

#[test]
fn deleting_and_inserting_characters_leave_no_dangling_continuation() {
    // DCH 1 on the lead shifts the row; the orphaned continuation that
    // lands in column zero must be blanked.
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes("你AB".as_bytes());
    state.move_cursor(CursorMove::ToColumn(0));
    state.feed_bytes(b"\x1b[1P");
    // Cell-based delete: the continuation shifted into column zero is
    // blanked, leaving one blank ahead of the shifted tail.
    assert_eq!(state.snapshot().lines(), [" AB"]);
    assert!(
        state
            .screen()
            .cells()
            .iter()
            .all(|cell| !cell.is_continuation())
    );

    // ICH before the lead shifts the intact pair right without splitting it.
    let mut state = TerminalState::new(1, 5).expect("valid terminal");
    state.feed_bytes("A你".as_bytes());
    state.move_cursor(CursorMove::ToColumn(2));
    assert_eq!(cursor(&state), (0, 1));
    state.feed_bytes(b"\x1b[1@");
    assert_eq!(state.snapshot().lines(), ["A 你"]);
    assert_eq!(cell(&state, 0, 2).text(), "你");
    assert!(cell(&state, 0, 3).is_continuation());
}

#[test]
fn combining_marks_attach_to_the_preceding_cell_without_moving_the_cursor() {
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("e\u{0301}o".as_bytes());

    assert_eq!(cell(&state, 0, 0).text(), "e\u{0301}");
    assert_eq!(cell(&state, 0, 0).width(), 1);
    assert_eq!(cell(&state, 0, 1).text(), "o");
    assert_eq!(state.snapshot().lines(), ["e\u{0301}o"]);
    assert_eq!(cursor(&state), (0, 2));

    // A mark after a wide character skips the continuation cell and attaches
    // to the lead.
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("你\u{0301}".as_bytes());
    assert_eq!(cell(&state, 0, 0).text(), "你\u{0301}");
    assert_eq!(cell(&state, 0, 0).width(), 2);
    assert_eq!(cursor(&state), (0, 2));

    // A mark while autowrap is pending attaches to the edge character and
    // keeps the wrap pending.
    let mut state = TerminalState::new(2, 3).expect("valid terminal");
    state.feed_bytes("abc\u{0301}".as_bytes());
    assert_eq!(cursor(&state), (0, 2));
    assert!(state.is_wrap_pending());
    state.feed_bytes(b"d");
    assert_eq!(state.snapshot().lines(), ["abc\u{0301}", "d"]);
}

#[test]
fn combining_mark_with_no_preceding_cell_is_dropped() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes("\u{0301}A".as_bytes());

    assert_eq!(state.snapshot().lines(), ["A"]);
    assert_eq!(cell(&state, 0, 0).text(), "A");
    assert_eq!(cursor(&state), (0, 1));
}

#[test]
fn cursor_never_reports_a_continuation_column() {
    let mut state = TerminalState::new(2, 6).expect("valid terminal");
    state.feed_bytes("你好".as_bytes());
    assert_eq!(cursor(&state), (0, 4));

    // Absolute positioning onto a continuation column snaps to the lead.
    state.move_cursor(CursorMove::To { row: 0, column: 3 });
    assert_eq!(cursor(&state), (0, 2));
    state.feed_bytes(b"\x1b[1;4H");
    assert_eq!(cursor(&state), (0, 2));

    // Relative motion resolves onto character boundaries: forward motion
    // skips the wide character, backward motion lands on its lead.
    state.move_cursor(CursorMove::Right(1));
    assert_eq!(cursor(&state), (0, 4));
    state.feed_bytes(b"\x08");
    assert_eq!(cursor(&state), (0, 2));
    state.move_cursor(CursorMove::ToColumn(4));
    state.move_cursor(CursorMove::Left(1));
    assert_eq!(cursor(&state), (0, 2));

    // A downward move onto another row's continuation snaps to its lead.
    state.feed_bytes("\x1b[2;5H你".as_bytes());
    state.move_cursor(CursorMove::To { row: 0, column: 5 });
    state.move_cursor(CursorMove::Down(1));
    assert_eq!(cursor(&state), (1, 4));
}

#[test]
fn mixed_ascii_cjk_and_emoji_land_on_the_expected_columns() {
    let mut state = TerminalState::new(1, 10).expect("valid terminal");
    state.feed_bytes("a😀日b".as_bytes());

    assert_eq!(state.snapshot().lines(), ["a😀日b"]);
    assert_eq!(cell(&state, 0, 0).text(), "a");
    assert_eq!(cell(&state, 0, 0).width(), 1);
    assert_eq!(cell(&state, 0, 1).text(), "😀");
    assert_eq!(cell(&state, 0, 1).width(), 2);
    assert!(cell(&state, 0, 2).is_continuation());
    assert_eq!(cell(&state, 0, 3).text(), "日");
    assert_eq!(cell(&state, 0, 3).width(), 2);
    assert!(cell(&state, 0, 4).is_continuation());
    assert_eq!(cell(&state, 0, 5).text(), "b");
    assert_eq!(cursor(&state), (0, 6));
}

#[test]
fn multibyte_sequences_decode_across_feed_boundaries() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    for byte in "你".as_bytes() {
        state.feed_bytes(std::slice::from_ref(byte));
    }

    assert_eq!(state.snapshot().lines(), ["你"]);
    assert_eq!(cursor(&state), (0, 2));
}

#[test]
fn resize_cutting_a_continuation_blanks_the_orphaned_lead() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes("ab你".as_bytes());

    state.resize(1, 3).expect("valid resize");

    assert_eq!(state.snapshot().lines(), ["ab"]);
    assert!(
        state
            .screen()
            .cells()
            .iter()
            .all(|cell| !cell.is_continuation())
    );
    assert_eq!(cell(&state, 0, 2).text(), " ");
}
