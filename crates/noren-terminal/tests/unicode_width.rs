//! Display-width behavior for wide, zero-width, and mixed-width printing.

use noren_terminal::{Cell, CursorMove, MAX_COMBINING_MARKS_PER_CELL, TerminalState};

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

fn cell(state: &TerminalState, row: u16, column: u16) -> &Cell {
    state.screen().cell(row, column).expect("cell is in bounds")
}

fn assert_cursor_off_continuation(state: &TerminalState) {
    let (row, column) = cursor(state);
    assert!(
        !state
            .screen()
            .cell(row, column)
            .is_some_and(Cell::is_continuation),
        "cursor rests on a continuation cell at row {row}, column {column}"
    );
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

/// Coordinator reproduction: the cursor sits on the continuation column of
/// the row above a wide character, and LF carries that column downward onto
/// the continuation half.
fn reproduction_state() -> TerminalState {
    let mut state = TerminalState::new(4, 6).expect("valid terminal");
    state.feed_bytes("\x1b[3;3H日".as_bytes());
    assert_eq!(cursor(&state), (2, 4));
    state.feed_bytes(b"\x1b[2;4H");
    assert_eq!(cursor(&state), (1, 3));
    state
}

#[test]
fn line_feed_snaps_the_cursor_off_a_continuation_column() {
    let mut state = reproduction_state();
    state.feed_bytes(b"\n");
    assert_eq!(cursor(&state), (2, 2));
    assert_cursor_off_continuation(&state);
}

#[test]
fn index_nel_and_reverse_index_snap_the_cursor_off_continuations() {
    // IND (ESC D) keeps the column while moving down onto the continuation.
    let mut state = reproduction_state();
    state.feed_bytes(b"\x1bD");
    assert_eq!(cursor(&state), (2, 2));
    assert_cursor_off_continuation(&state);

    // NEL (ESC E) homes the column, which is never a continuation.
    let mut state = reproduction_state();
    state.feed_bytes(b"\x1bE");
    assert_eq!(cursor(&state), (2, 0));
    assert_cursor_off_continuation(&state);

    // RI (ESC M) keeps the column while moving up onto the continuation.
    let mut state = reproduction_state();
    state.feed_bytes(b"\x1b[4;4H");
    state.feed_bytes(b"\x1bM");
    assert_eq!(cursor(&state), (2, 2));
    assert_cursor_off_continuation(&state);
}

#[test]
fn reverse_index_at_the_top_margin_keeps_wide_rows_intact() {
    let mut state = TerminalState::new(3, 6).expect("valid terminal");
    state.feed_bytes("\x1b[2;3H日\x1b[1;4H".as_bytes());
    assert_eq!(cursor(&state), (0, 3));

    // RI at the top margin scrolls the region down instead of moving the
    // cursor; the blank row inserted under the cursor has no continuation,
    // and the wide row keeps both halves.
    state.feed_bytes(b"\x1bM");
    assert_eq!(cursor(&state), (0, 3));
    assert_cursor_off_continuation(&state);
    assert_eq!(cell(&state, 2, 2).text(), "日");
    assert_eq!(cell(&state, 2, 2).width(), 2);
    assert!(cell(&state, 2, 3).is_continuation());
}

#[test]
fn printing_after_a_snapped_line_feed_replaces_the_pair_at_the_lead() {
    let mut state = reproduction_state();
    state.feed_bytes(b"\nX");

    // The cursor snapped to the lead, so the printable replaces the wide
    // pair at its lead column instead of blanking the lead from the
    // continuation side and leaving a hole one column left of the print.
    assert_eq!(state.snapshot().lines(), ["", "", "  X"]);
    assert_eq!(cell(&state, 2, 2).text(), "X");
    assert_eq!(cell(&state, 2, 3).text(), " ");
    assert!(!cell(&state, 2, 3).is_continuation());
    assert_cursor_off_continuation(&state);
}

#[test]
fn wide_characters_survive_row_moves_and_later_prints() {
    let mut state = reproduction_state();
    // LF through the row carrying the continuation column, then print on the
    // following row: the wide character must remain intact behind the cursor.
    state.feed_bytes(b"\n\nX");

    assert_eq!(cursor(&state), (3, 3));
    assert_eq!(cell(&state, 2, 2).text(), "日");
    assert_eq!(cell(&state, 2, 2).width(), 2);
    assert!(cell(&state, 2, 3).is_continuation());
    assert_eq!(state.snapshot().lines(), ["", "", "  日", "  X"]);
}

#[test]
fn display_lines_preserve_wide_columns_while_lines_keep_their_meaning() {
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    state.feed_bytes("a日b".as_bytes());
    let snapshot = state.snapshot();

    // The existing accessor keeps its character-packed meaning.
    assert_eq!(snapshot.lines(), ["a日b"]);

    // The column-preserving view spans four display columns (a=1, 日=2, b=1)
    // with b at display column 3.
    let display = snapshot.display_lines();
    assert_eq!(display, ["a日 b"]);
    assert_eq!(display[0].chars().count(), 4);
    assert_eq!(display[0].chars().nth(3), Some('b'));

    // ASCII rows are byte-identical in both views.
    let mut ascii = TerminalState::new(1, 6).expect("valid terminal");
    ascii.feed_bytes(b"a b");
    let snapshot = ascii.snapshot();
    assert_eq!(snapshot.display_lines(), snapshot.lines());
}

// ===== KBUG-01 fix: combining-mark budget must not break legitimate text =====

/// The grapheme-cluster cap must leave realistic combining sequences — the
/// regression risk for this fix — exactly intact. Every script that stacks
/// marks in real text stays well under [`MAX_COMBINING_MARKS_PER_CELL`], so the
/// cell's text round-trips byte-for-byte.
#[test]
fn realistic_combining_sequences_render_exactly_within_the_cap() {
    // Latin: 'e' with macron and acute (e + U+0304 + U+0301).
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes("e\u{0304}\u{0301}".as_bytes());
    assert_eq!(cell(&state, 0, 0).text(), "e\u{0304}\u{0301}");
    assert_eq!(cell(&state, 0, 0).width(), 1);

    // Devanagari: consonant + nukta + candrabindu (क + U+093C + U+0901).
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes("\u{0915}\u{093C}\u{0901}".as_bytes());
    assert_eq!(cell(&state, 0, 0).text(), "\u{0915}\u{093C}\u{0901}");

    // Thai: consonant + below vowel + above tone mark (ก + U+0E38 + U+0E48).
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes("\u{0E01}\u{0E38}\u{0E48}".as_bytes());
    assert_eq!(cell(&state, 0, 0).text(), "\u{0E01}\u{0E38}\u{0E48}");

    // Fully pointed Hebrew: bet + dagesh + qamats + etnahta (3 marks).
    let pointed = "\u{05D1}\u{05BC}\u{05B8}\u{05A0}";
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes(pointed.as_bytes());
    assert_eq!(cell(&state, 0, 0).text(), pointed);

    // The renderer-facing line keeps every mark of the Hebrew cluster.
    assert_eq!(state.snapshot().lines(), [pointed]);
}

/// A hostile flood of combining marks is capped to exactly
/// [`MAX_COMBINING_MARKS_PER_CELL`] on both attach paths: the normal cursor
/// path (preceding cell one column left) and the wrap-pending path (cursor
/// resting on the armed right-edge cell). Kimi demonstrated the same flood
/// reaches both.
#[test]
fn a_combining_mark_flood_is_capped_on_both_attach_paths() {
    let flood = "\u{0301}".repeat(2_000);

    // Normal cursor path.
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"a");
    state.feed_bytes(flood.as_bytes());
    let normal_marks = cell(&state, 0, 0).text().chars().count().saturating_sub(1);
    assert_eq!(
        normal_marks, MAX_COMBINING_MARKS_PER_CELL,
        "normal cursor path did not stop at the cap"
    );

    // Wrap-pending path: filling the last column arms autowrap, so attach
    // targets that right-edge cell.
    let mut state = TerminalState::new(1, 2).expect("valid terminal");
    state.feed_bytes(b"ab");
    assert!(state.is_wrap_pending());
    state.feed_bytes(flood.as_bytes());
    let wrap_marks = cell(&state, 0, 1).text().chars().count().saturating_sub(1);
    assert_eq!(
        wrap_marks, MAX_COMBINING_MARKS_PER_CELL,
        "wrap-pending attach path did not stop at the cap"
    );
}

/// Once a cell is capped the bound propagates for free to retained scrollback
/// rows and to every snapshot, since both simply observe the already-capped
/// cells. The documented per-cell text ceiling is `4 * (cap + 1)` bytes.
#[test]
fn a_capped_cell_stays_capped_through_scrollback_and_snapshots() {
    let bytes_per_cell = 4 * (MAX_COMBINING_MARKS_PER_CELL + 1);
    let flood = "\u{0301}".repeat(5_000);

    // One-row grid: the capped row is the one that will scroll off.
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"a");
    state.feed_bytes(flood.as_bytes());
    let live_text = cell(&state, 0, 0).text().to_owned();
    assert!(
        live_text.len() <= bytes_per_cell,
        "live cell text {} exceeds the {}-byte per-cell ceiling",
        live_text.len(),
        bytes_per_cell
    );

    // Scroll the capped row off into scrollback.
    state.feed_bytes(b"\r\nb");
    assert_eq!(state.scrollback_len(), 1);

    // The retained scrollback cell (via the snapshot) is the same capped cell.
    let snapshot = state.snapshot();
    let retained = &snapshot.scrollback()[0][0];
    assert_eq!(
        retained.text(),
        live_text,
        "scrollback inherited a different cell"
    );
    assert!(
        retained.text().len() <= bytes_per_cell,
        "scrollback cell exceeds the per-cell ceiling"
    );

    // The renderer-facing rendering of that retained row is bounded too.
    let scrollback_lines = snapshot.scrollback_lines();
    let row_text = scrollback_lines[0].as_str();
    assert!(
        row_text.len() <= bytes_per_cell,
        "scrollback line {row_text:?} exceeds the per-cell ceiling"
    );
}
