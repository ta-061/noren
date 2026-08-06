//! Grid selection: granularities, wide-character boundary rounding,
//! scrollback spans, and the documented expiration policy for resize,
//! scroll, and alternate-screen switches.

use noren_terminal::{GridPoint, Selection, SelectionGrid, SelectionMode, TerminalState};

fn grid(rows: u16, cols: u16, bytes: &[u8]) -> TerminalState {
    let mut state = TerminalState::new(rows, cols).expect("valid test terminal");
    state.feed_bytes(bytes);
    state
}

#[test]
fn ascii_extraction_is_byte_exact() {
    let state = grid(3, 12, b"hello world\r\nsecond line");
    let row = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 10),
    );
    assert_eq!(row.extract(&state), "hello world");

    let partial = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 6),
        GridPoint::new(1, 5),
    );
    // First row keeps its tail, last row keeps its head, joined with LF.
    assert_eq!(partial.extract(&state), "world\nsecond");

    let single_cell = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(1, 3),
        GridPoint::new(1, 3),
    );
    assert_eq!(single_cell.extract(&state), "o");
}

#[test]
fn cjk_extraction_is_byte_exact() {
    let state = grid(2, 8, "日本語".as_bytes());
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 5),
    );
    assert_eq!(selection.extract(&state), "日本語");

    // Ending on the lead column of the second character excludes the third.
    let two = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 2),
    );
    assert_eq!(two.extract(&state), "日本");
}

#[test]
fn mixed_width_extraction_is_byte_exact() {
    let state = grid(2, 10, "a😀日b".as_bytes());
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 5),
    );
    assert_eq!(selection.extract(&state), "a😀日b");
}

#[test]
fn endpoints_on_a_continuation_round_to_the_whole_character() {
    // Columns: 你=0(+1 continuation), 好=2(+3 continuation), 世=4(+5 continuation).
    let state = grid(2, 8, "你好世".as_bytes());

    // End landing on 好's continuation still extracts the whole character.
    let end_on_continuation = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 3),
    );
    assert_eq!(end_on_continuation.end(), GridPoint::new(0, 2));
    let text = end_on_continuation.extract(&state);
    assert_eq!(text, "你好");
    assert!(text.contains('好'), "the whole character, never a half");

    // Start landing on 好's continuation rounds back onto its lead.
    let start_on_continuation = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 3),
        GridPoint::new(0, 5),
    );
    assert_eq!(start_on_continuation.start(), GridPoint::new(0, 2));
    assert_eq!(start_on_continuation.extract(&state), "好世");

    // Both endpoints on the continuation of one character select it whole.
    let collapsed = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 1),
        GridPoint::new(0, 1),
    );
    assert_eq!(collapsed.extract(&state), "你");
}

#[test]
fn word_selection_expands_to_word_boundaries() {
    let state = grid(2, 16, b"foo bar.baz_qux");

    let bar = Selection::new(
        &state,
        SelectionMode::Word,
        GridPoint::new(0, 5),
        GridPoint::new(0, 5),
    );
    assert_eq!(bar.start(), GridPoint::new(0, 4));
    assert_eq!(bar.end(), GridPoint::new(0, 6));
    assert_eq!(bar.extract(&state), "bar");

    // `_` joins a word; `.` and blanks separate words.
    let joined = Selection::new(
        &state,
        SelectionMode::Word,
        GridPoint::new(0, 9),
        GridPoint::new(0, 9),
    );
    assert_eq!(joined.extract(&state), "baz_qux");

    // A separator cell selects only itself.
    let dot = Selection::new(
        &state,
        SelectionMode::Word,
        GridPoint::new(0, 7),
        GridPoint::new(0, 7),
    );
    assert_eq!(dot.extract(&state), ".");

    // Dragging across two words covers both and the gap between them.
    let span = Selection::new(
        &state,
        SelectionMode::Word,
        GridPoint::new(0, 1),
        GridPoint::new(0, 9),
    );
    assert_eq!(span.extract(&state), "foo bar.baz_qux");
}

#[test]
fn word_selection_treats_cjk_cells_as_word_characters() {
    let state = grid(2, 10, "日本語 abc".as_bytes());
    let word = Selection::new(
        &state,
        SelectionMode::Word,
        GridPoint::new(0, 3),
        GridPoint::new(0, 3),
    );
    assert_eq!(word.extract(&state), "日本語");

    // Word expansion never crosses a row edge.
    let across = grid(2, 8, b"abc\r\ndef");
    let bounded = Selection::new(
        &across,
        SelectionMode::Word,
        GridPoint::new(0, 1),
        GridPoint::new(0, 1),
    );
    assert_eq!(bounded.extract(&across), "abc");
}

#[test]
fn line_selection_takes_whole_rows_and_trims_trailing_blanks() {
    // Row 0 carries trailing blanks; row 1 is short.
    let state = grid(3, 8, b"AAA   \r\nBB");
    let both = Selection::new(
        &state,
        SelectionMode::Line,
        GridPoint::new(0, 5),
        GridPoint::new(1, 0),
    );
    assert_eq!(both.start(), GridPoint::new(0, 0));
    assert_eq!(both.end(), GridPoint::new(1, 7));
    assert_eq!(both.extract(&state), "AAA\nBB");

    let single = Selection::new(
        &state,
        SelectionMode::Line,
        GridPoint::new(0, 3),
        GridPoint::new(0, 3),
    );
    assert_eq!(single.extract(&state), "AAA");
}

#[test]
fn selections_spanning_scrollback_and_visible_rows_are_contiguous() {
    // Two-row grid; three CRLFs evict AAAA then BBBB into scrollback.
    let state = grid(2, 4, b"AAAA\r\nBBBB\r\nCCCC\r\n");
    assert_eq!(state.scrollback_len(), 2);

    let span = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(3, 3),
    );
    assert_eq!(span.extract(&state), "AAAA\nBBBB\nCCCC");

    // A span starting inside scrollback and ending mid-visible works too.
    let tail = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(1, 2),
        GridPoint::new(2, 1),
    );
    assert_eq!(tail.extract(&state), "BB\nCC");
}

#[test]
fn scrollback_rows_keep_their_own_widths_after_resize() {
    let mut state = grid(2, 4, b"AAAA\r\nBBBB\r\nCCCC\r\n");
    state.resize(2, 8).expect("valid resize");

    // Scrollback rows keep width 4; visible rows are width 8. A line-wise
    // selection of the oldest scrollback row yields exactly its content.
    let selection = Selection::new(
        &state,
        SelectionMode::Line,
        GridPoint::new(0, 0),
        GridPoint::new(0, 0),
    );
    assert_eq!(selection.extract(&state), "AAAA");
}

#[test]
fn resize_expires_a_selection() {
    let mut state = grid(3, 8, b"hello");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 4),
    );
    assert_eq!(selection.extract(&state), "hello");

    state.resize(4, 10).expect("valid resize");
    assert!(!selection.is_valid(&state));
    // Expired selections yield empty text, never stale or wrong text.
    assert_eq!(selection.extract(&state), "");
}

#[test]
fn scrolling_that_retains_lines_expires_a_selection() {
    let mut state = grid(2, 4, b"AAAA\r\nBBBB");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(1, 0),
        GridPoint::new(1, 3),
    );
    assert_eq!(selection.extract(&state), "BBBB");

    // Evicting a top row changes the scrollback length, shifting every
    // absolute line index; the selection must not silently re-address.
    state.feed_bytes(b"\r\nCCCC");
    assert_eq!(state.scrollback_len(), 1);
    assert!(!selection.is_valid(&state));
    assert_eq!(selection.extract(&state), "");
}

#[test]
fn alternate_screen_makes_capture_time_selections_unavailable() {
    let mut state = grid(3, 8, b"primary");
    let selection = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(0, 6),
    );
    assert_eq!(selection.extract(&state), "primary");

    // While the alternate screen is selected, extraction refuses.
    state.feed_bytes(b"\x1b[?1049h");
    assert!(state.modes().is_alternate_screen_active());
    assert!(!selection.is_valid(&state));
    assert_eq!(selection.extract(&state), "");

    // Leaving restores the primary content untouched, so the stamp matches
    // again (the app separately invalidates on the output that switched).
    state.feed_bytes(b"\x1b[?1049l");
    assert!(selection.is_valid(&state));
    assert_eq!(selection.extract(&state), "primary");
}

#[test]
fn empty_selections_yield_empty_text_without_panicking() {
    let blank = grid(3, 4, b"");
    let selection = Selection::new(
        &blank,
        SelectionMode::Char,
        GridPoint::new(1, 1),
        GridPoint::new(2, 2),
    );
    assert_eq!(selection.extract(&blank), "");

    // Line-wise over blank-only rows is equally empty.
    let lines = Selection::new(
        &blank,
        SelectionMode::Line,
        GridPoint::new(0, 0),
        GridPoint::new(2, 3),
    );
    assert_eq!(lines.extract(&blank), "");
}

#[test]
fn whole_grid_selection_works_at_every_boundary() {
    let state = grid(3, 4, b"ABCD\r\nEFGH\r\nIJKL");
    let everything = Selection::new(
        &state,
        SelectionMode::Char,
        GridPoint::new(0, 0),
        GridPoint::new(2, 3),
    );
    assert_eq!(everything.extract(&state), "ABCD\nEFGH\nIJKL");

    // The four corners individually.
    for (point, expected) in [
        (GridPoint::new(0, 0), "A"),
        (GridPoint::new(0, 3), "D"),
        (GridPoint::new(2, 0), "I"),
        (GridPoint::new(2, 3), "L"),
    ] {
        let corner = Selection::new(&state, SelectionMode::Char, point, point);
        assert_eq!(corner.extract(&state), expected, "corner {point:?}");
    }

    // Trailing blank rows inside the range do not leave dangling newlines.
    let with_tail = grid(4, 4, b"ABCD");
    let full = Selection::new(
        &with_tail,
        SelectionMode::Line,
        GridPoint::new(0, 0),
        GridPoint::new(3, 3),
    );
    assert_eq!(full.extract(&with_tail), "ABCD");
}

#[test]
fn selection_works_through_a_snapshot_grid_as_well() {
    let state = grid(2, 4, b"AAAA\r\nBBBB\r\nCCCC\r\n");
    let snapshot = state.snapshot();
    assert_eq!(snapshot.scrollback_len(), 2);

    let span = Selection::new(
        &snapshot,
        SelectionMode::Line,
        GridPoint::new(0, 0),
        GridPoint::new(2, 0),
    );
    assert_eq!(span.extract(&snapshot), "AAAA\nBBBB\nCCCC");
    assert!(!span.extract(&state).is_empty());

    // A resize after capture expires the snapshot-based selection too.
    let mut resized = state;
    resized.resize(3, 6).expect("valid resize");
    let resized_snapshot = resized.snapshot();
    assert!(!span.is_valid(&resized_snapshot));
    assert_eq!(span.extract(&resized_snapshot), "");
}

#[test]
fn entire_grid_covers_scrollback_and_visible_rows_at_boundaries() {
    let state = grid(2, 4, b"AAAA\r\nBBBB\r\nCCCC\r\n");
    let everything = Selection::entire_grid(&state);
    assert_eq!(everything.start(), GridPoint::new(0, 0));
    assert_eq!(everything.extract(&state), "AAAA\nBBBB\nCCCC");

    // A resized grid is covered by a freshly captured entire-grid selection.
    let grown = grid(3, 4, b"ABCD\r\nEFGH\r\nIJKL");
    assert_eq!(
        Selection::entire_grid(&grown).extract(&grown),
        "ABCD\nEFGH\nIJKL"
    );
}

#[test]
fn wide_character_word_and_line_modes_keep_characters_whole() {
    let state = grid(1, 8, "a你好b".as_bytes());

    let word = Selection::new(
        &state,
        SelectionMode::Word,
        GridPoint::new(0, 2),
        GridPoint::new(0, 2),
    );
    assert_eq!(word.extract(&state), "a你好b");

    let line = Selection::new(
        &state,
        SelectionMode::Line,
        GridPoint::new(0, 0),
        GridPoint::new(0, 7),
    );
    assert_eq!(line.extract(&state), "a你好b");
    // No endpoint rests on a continuation column after normalization.
    for point in [line.start(), line.end(), word.start(), word.end()] {
        let cells = state.row_cells(point.line()).expect("row exists");
        assert!(!cells[point.column()].is_continuation());
    }
}
