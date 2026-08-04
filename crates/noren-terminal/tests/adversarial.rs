//! Adversarial hostile-input sweep for the Terminal Core state machine.
//!
//! These tests attack `TerminalState` as an untrusted-input boundary. Every
//! test here must execute. Inputs that produce correct behavior assert it;
//! inputs that only hunt for panics assert the public invariants (cursor in
//! bounds, screen bounded by the cell cap, scroll region well-formed).

use noren_terminal::{
    Cell, CellAttributes, CursorMove, MAX_SCREEN_CELLS, TerminalError, TerminalState,
};

/// Public invariants that must hold after *any* sequence of public calls.
fn assert_invariants(state: &TerminalState, context: &str) {
    let (rows, cols) = state.size();
    assert!(rows > 0 && cols > 0, "{context}: non-zero size");
    assert!(
        usize::from(rows) * usize::from(cols) <= MAX_SCREEN_CELLS,
        "{context}: cell cap"
    );
    assert_eq!(
        state.screen().cells().len(),
        usize::from(rows) * usize::from(cols),
        "{context}: cell count matches grid"
    );
    let cursor = state.cursor();
    assert!(cursor.row() < rows, "{context}: cursor row in bounds");
    assert!(cursor.column() < cols, "{context}: cursor column in bounds");
    let region = state.scroll_region();
    assert!(region.top() <= region.bottom(), "{context}: region ordered");
    assert!(region.bottom() < rows, "{context}: region within screen");
}

fn feed_bytewise(state: &mut TerminalState, bytes: &[u8]) {
    for byte in bytes {
        state.feed_bytes(std::slice::from_ref(byte));
    }
}

// ===== Enormous and degenerate CSI parameters =====

#[test]
fn enormous_csi_parameters_clamp_instead_of_panicking() {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");

    // 9-digit params saturate the u16 parameter accumulator to 65535, then the
    // cursor clamps to the grid. No overflow, no panic.
    state.feed_bytes(b"\x1b[999999999;999999999H");
    assert_eq!((state.cursor().row(), state.cursor().column()), (2, 4));
    assert_invariants(&state, "enormous CUP");

    state.feed_bytes(b"\x1b[999999999A\x1b[999999999B\x1b[999999999C\x1b[999999999D");
    assert_invariants(&state, "enormous cursor moves");

    state.feed_bytes(b"\x1b[999999999S\x1b[999999999T\x1b[999999999L\x1b[999999999M");
    assert_invariants(&state, "enormous scroll/line ops");

    state.feed_bytes(b"\x1b[999999999X\x1b[999999999@\x1b[999999999P");
    assert_invariants(&state, "enormous character edits");

    // SGR with enormous params: known codes between them still apply.
    state.feed_bytes(b"\x1b[999999999;1;4;999999999m");
    let pen = *state.attributes();
    assert!(pen.is_bold());
    assert!(pen.is_underlined());
}

#[test]
fn degenerate_parameter_lists_do_not_panic() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    // 30+ empty params -> parameter overflow -> whole CUP dropped.
    state.feed_bytes(b"\x1b[;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;H");
    assert_eq!((state.cursor().row(), state.cursor().column()), (0, 0));
    assert_invariants(&state, "many semicolons CUP");

    // SGR with >8 params is dropped entirely (overflow), pen unchanged.
    state.feed_bytes(b"\x1b[1;2;3;4;5;6;7;8;9m");
    assert_eq!(*state.attributes(), CellAttributes::default());

    // Lone/default params resolve to defaults.
    state.feed_bytes(b"\x1b[;H");
    assert_eq!((state.cursor().row(), state.cursor().column()), (0, 0));

    // A bare leading semicolon before the final still parses as two defaults.
    state.feed_bytes(b"\x1b[;r");
    assert_eq!(
        (state.scroll_region().top(), state.scroll_region().bottom()),
        (0, 1)
    );
    assert_invariants(&state, "default DECSTBM");
}

// ===== Sequences split across feed_bytes boundaries =====

#[test]
fn every_sequence_survives_byte_at_a_time_feeding() {
    let sequences: &[&[u8]] = &[
        b"\x1b[2;4H",
        b"\x1b[2;4r\x1b[2;1H\x1bD",
        b"\x1b[?1049h",
        b"\x1b[?1049l",
        b"\x1b[31;1;4m",
        b"\x1b[2J",
        b"\x1b[2K",
        b"\x1b[3;5H\x1b[2A",
        b"\x1b]0;title\x07",
        b"\x1b[?1h\x1b=",
        b"\x1b(B",
        b"a\tb",
        b"\x1b7\x1b8",
    ];

    for sequence in sequences {
        let mut whole = TerminalState::new(4, 10).expect("valid terminal");
        whole.feed_bytes(sequence);

        let mut split = TerminalState::new(4, 10).expect("valid terminal");
        feed_bytewise(&mut split, sequence);

        assert_eq!(
            whole.snapshot(),
            split.snapshot(),
            "byte-at-a-time diverged from whole-feed for {sequence:?}"
        );
    }
}

#[test]
fn split_osc_terminated_by_string_terminator_survives() {
    let mut whole = TerminalState::new(1, 4).expect("valid terminal");
    whole.feed_bytes(b"\x1b]0;ok\x1b\\Z");

    let mut split = TerminalState::new(1, 4).expect("valid terminal");
    feed_bytewise(&mut split, b"\x1b]0;ok\x1b\\Z");

    assert_eq!(whole.snapshot(), split.snapshot());
    // After the terminated OSC, the printable Z renders.
    assert_eq!(split.snapshot().lines(), ["Z".to_owned()]);
}

#[test]
fn mid_sequence_resize_does_not_corrupt_the_parser() {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    // Begin CSI 2;3H (1-based) but stop before the final byte.
    state.feed_bytes(b"\x1b[2;");
    state.resize(4, 6).expect("valid resize mid-sequence");
    // Complete the sequence across the resize boundary.
    state.feed_bytes(b"3HZ");
    // CSI 2;3H -> 0-based row 1, column 2; Z prints there and the cursor
    // advances to column 3.
    assert_eq!((state.cursor().row(), state.cursor().column()), (1, 3));
    assert_eq!(state.screen().cell(1, 2).map(Cell::text), Some("Z"));
    assert_invariants(&state, "mid-sequence resize");
}

// ===== Invalid UTF-8 =====

#[test]
fn invalid_utf8_bytes_are_dropped_without_panicking() {
    let invalid_inputs: &[&[u8]] = &[
        &[0x80],                   // lone continuation
        &[0xbf],                   // lone continuation
        &[0xc0, 0xaf],             // overlong 2-byte
        &[0xe0, 0x80, 0x80],       // overlong 3-byte
        &[0xf0, 0x80, 0x80, 0x80], // overlong 4-byte
        &[0xed, 0xa0, 0x80],       // surrogate half U+D800
        &[0xed, 0xbf, 0xbf],       // surrogate half U+DFFF
        &[0xfe],                   // invalid lead byte
        &[0xff],                   // invalid lead byte
        &[0xc3],                   // truncated 2-byte
        &[0xe0, 0xa0],             // truncated 3-byte
        &[0xf0, 0x90, 0x80],       // truncated 4-byte
    ];

    for input in invalid_inputs {
        let mut state = TerminalState::new(1, 4).expect("valid terminal");
        feed_bytewise(&mut state, input);
        assert!(state.snapshot().lines().is_empty(), "input {input:?}");
        assert_invariants(&state, "invalid utf8");

        // Valid ASCII still renders after garbage.
        state.feed_bytes(b"OK");
        assert_eq!(
            state.snapshot().lines(),
            ["OK".to_owned()],
            "after {input:?}"
        );
    }
}

#[test]
fn high_bytes_interleaved_with_ascii_are_individually_dropped() {
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    // 0xff between every ASCII byte: none of the high bytes may print or panic.
    state.feed_bytes(&[b'A', 0xff, b'B', 0xc0, 0xaf, b'C']);
    assert_eq!(state.snapshot().lines(), ["ABC".to_owned()]);
    assert_invariants(&state, "interleaved high bytes");
}

// ===== Degenerate scroll regions =====

#[test]
fn inverted_and_degenerate_scroll_regions_are_rejected_and_preserve_state() {
    let mut state = TerminalState::new(5, 3).expect("valid terminal");
    let (top0, bottom0) = (state.scroll_region().top(), state.scroll_region().bottom());

    // inverted (top > bottom)
    state.feed_bytes(b"\x1b[4;2r");
    assert_eq!(
        (state.scroll_region().top(), state.scroll_region().bottom()),
        (top0, bottom0)
    );
    // single-row (top == bottom)
    state.feed_bytes(b"\x1b[3;3r");
    assert_eq!(
        (state.scroll_region().top(), state.scroll_region().bottom()),
        (top0, bottom0)
    );
    // bottom past the last screen line
    state.feed_bytes(b"\x1b[1;99r");
    assert_eq!(
        (state.scroll_region().top(), state.scroll_region().bottom()),
        (top0, bottom0)
    );

    // The public API must agree and return Err without mutating.
    assert_eq!(
        state.set_scroll_region(3, 1),
        Err(TerminalError::InvalidScrollRegion)
    );
    assert_eq!(
        (state.scroll_region().top(), state.scroll_region().bottom()),
        (top0, bottom0)
    );

    assert_invariants(&state, "degenerate regions rejected");
}

#[test]
fn hard_scroll_inside_a_valid_region_stays_bounded() {
    let mut state = TerminalState::new(5, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\x1b[2;1HBBB\x1b[3;1HCCC\x1b[4;1HDDD\x1b[5;1HEEE");
    state.feed_bytes(b"\x1b[2;4r\x1b[3;2H");

    // Saturating scroll counts clamp to the region height (3) and must not panic.
    state.feed_bytes(b"\x1b[65535S");
    assert_invariants(&state, "hard scroll up");
    state.feed_bytes(b"\x1b[65535T");
    assert_invariants(&state, "hard scroll down");
    state.feed_bytes(b"\x1b[65535L");
    assert_invariants(&state, "hard insert lines");
    state.feed_bytes(b"\x1b[65535M");
    assert_invariants(&state, "hard delete lines");

    assert_eq!(
        (state.scroll_region().top(), state.scroll_region().bottom()),
        (1, 3)
    );
}

// ===== Alternate-screen thrash with resize and DECSC/DECRC =====

#[test]
fn alternate_screen_thrash_with_resize_and_cursor_save_restore() {
    let mut state = TerminalState::new(4, 8).expect("valid terminal");
    state.feed_bytes(b"PRIMARY\x1b[3;3H");

    for iteration in 0..30_u16 {
        state.feed_bytes(b"\x1b[?1049h");
        assert!(state.modes().is_alternate_screen_active());
        state.feed_bytes(b"\x1b[2;2H\x1b7"); // DECSC on the alternate screen
        state
            .resize(2 + (iteration % 3), 8 + (iteration % 4))
            .expect("valid resize during alternate");
        state.feed_bytes(b"\x1b8"); // DECRC restores the alternate-screen cursor
        state.feed_bytes(b"\x1b[?1049l");
        assert!(!state.modes().is_alternate_screen_active());
        assert_invariants(&state, &format!("thrash iteration {iteration}"));
    }

    // After the storm the public modes are consistent and the primary screen's
    // top row survived (cols never dropped below 8).
    assert!(!state.modes().is_alternate_screen_active());
    assert_eq!(
        state.snapshot().lines().first().map(String::as_str),
        Some("PRIMARY")
    );
}

#[test]
fn decsc_with_no_prior_save_is_a_no_op_not_a_panic() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"AB\x1b[2;3H");
    // DECRC with nothing saved must leave the cursor where it is.
    state.feed_bytes(b"\x1b8");
    assert_eq!((state.cursor().row(), state.cursor().column()), (1, 2));
    assert_invariants(&state, "DECRC with no save");
}

// ===== Resize extremes and the cell cap =====

#[test]
fn resize_to_one_by_one_then_print_and_overflow() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.resize(1, 1).expect("resize to 1x1");
    assert_eq!(state.size(), (1, 1));

    // Three prints into a 1x1 grid: each print after the first scrolls.
    state.feed_bytes(b"ABC");
    assert_invariants(&state, "1x1 overflow");
    assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("C"));
}

#[test]
fn the_cell_cap_holds_at_the_boundary_and_rejects_overflow() {
    let side = 1024_u16;
    assert_eq!(usize::from(side) * usize::from(side), MAX_SCREEN_CELLS);

    // Exactly at the cap is permitted.
    let mut state = TerminalState::new(side, side).expect("1024x1024 == cap");
    assert_invariants(&state, "at cap");

    // One column over the cap is rejected; state is unchanged.
    assert_eq!(
        state.resize(side, side + 1),
        Err(TerminalError::ScreenTooLarge)
    );
    assert_eq!(state.size(), (side, side));

    // Zero dimension rejected; state unchanged.
    assert_eq!(state.resize(0, 10), Err(TerminalError::InvalidSize));
    assert_eq!(state.resize(10, 0), Err(TerminalError::InvalidSize));
    assert_eq!(state.size(), (side, side));
    assert_invariants(&state, "after rejected resizes");
}

#[test]
fn rapid_resize_storm_keeps_state_consistent() {
    let mut state = TerminalState::new(3, 3).expect("valid terminal");
    state.feed_bytes(b"ABC");
    for size in 1..=50_u16 {
        let rows = (size % 5) + 1;
        let cols = (size % 6) + 1;
        state.resize(rows, cols).expect("valid storm resize");
        assert_invariants(&state, "storm resize");
    }
}

// ===== Long unterminated escape / OSC must not accumulate =====

#[test]
fn long_unterminated_escape_does_not_accumulate_or_break_state() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    // ESC + many intermediate bytes (no final): stays in EscapeIntermediate,
    // stores nothing, grows nothing.
    let mut hostile = vec![0x1b];
    hostile.extend(std::iter::repeat_n(b'(', 50_000));
    state.feed_bytes(&hostile);
    assert_invariants(&state, "long unterminated escape");
    assert!(state.snapshot().lines().is_empty());

    // A final byte terminates the sequence and is consumed (not printed).
    state.feed_bytes(b"B");
    assert!(state.snapshot().lines().is_empty());

    // Subsequent valid output renders normally.
    state.feed_bytes(b"OK");
    assert_eq!(state.snapshot().lines(), ["OK".to_owned()]);
}

#[test]
fn long_unterminated_osc_does_not_accumulate_or_break_state() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    let mut hostile = vec![0x1b, b']'];
    hostile.extend(std::iter::repeat_n(b'x', 50_000));
    state.feed_bytes(&hostile);
    assert_invariants(&state, "long unterminated OSC");
    assert!(state.snapshot().lines().is_empty());

    // Properly terminate, then print.
    state.feed_bytes(b"\x07OK");
    assert_eq!(state.snapshot().lines(), ["OK".to_owned()]);
}

#[test]
fn long_run_of_naked_escapes_does_not_desync() {
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    let mut hostile = vec![0x1b; 50_000];
    hostile.extend(b"[2CZ"); // after the ESC storm, a real CSI then a print
    state.feed_bytes(&hostile);
    // Cursor moved right 2 from col 0 -> col 2; Z prints at col 2.
    assert_eq!(state.screen().cell(0, 2).map(Cell::text), Some("Z"));
    assert_invariants(&state, "naked escape storm");
}

// ===== Tab handling at and past the last column (freshest code) =====

#[test]
fn tab_in_a_one_column_grid_clamps_without_panicking() {
    let mut state = TerminalState::new(1, 1).expect("valid terminal");
    state.feed_bytes(b"\t");
    assert_eq!((state.cursor().row(), state.cursor().column()), (0, 0));
    assert!(!state.is_wrap_pending());
    assert_invariants(&state, "tab in 1x1");
}

#[test]
fn tab_at_and_past_the_right_edge_clamps_and_keeps_wrapping_sound() {
    let mut state = TerminalState::new(2, 10).expect("valid terminal");
    // Repeated tabs clamp to the last column without advancing past it.
    state.feed_bytes(b"\t\t\tX");
    assert_eq!(state.screen().cell(0, 9).map(Cell::text), Some("X"));
    assert!(state.is_wrap_pending());
    assert_invariants(&state, "tab at right edge");

    // A tab while wrap is pending clears it and stays at the last column.
    state.feed_bytes(b"\t");
    assert_eq!(state.cursor().column(), 9);
    assert!(!state.is_wrap_pending());
}

#[test]
fn tab_on_the_widest_permitted_grid_does_not_overflow() {
    // Column 65534 -> next stop 65536 overflows u16; the implementation must
    // compute in usize and clamp rather than panic.
    let mut state = TerminalState::new(1, 65535).expect("widest grid");
    state.move_cursor(CursorMove::ToColumn(65534));
    state.feed_bytes(b"\t");
    assert_eq!(state.cursor().column(), 65534);
    assert_invariants(&state, "tab at widest grid last column");
}

#[test]
fn tab_then_print_then_scroll_round_trip_is_sound() {
    let mut state = TerminalState::new(2, 8).expect("valid terminal");
    state.feed_bytes(b"a\tb"); // a at col 0, tab -> col 8 clamp 7, b at col 7
    assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("a"));
    assert_eq!(state.screen().cell(0, 7).map(Cell::text), Some("b"));
    assert!(state.is_wrap_pending());
    // One more printable wraps and scrolls within the full-screen region.
    state.feed_bytes(b"c");
    assert_eq!(state.screen().cell(1, 0).map(Cell::text), Some("c"));
    assert_invariants(&state, "tab wrap round trip");
}

// ===== A bound on observable memory: the screen never exceeds the grid =====

#[test]
fn high_volume_printable_input_never_grows_the_grid() {
    // A flood of printable bytes into a tiny grid must keep the cell count
    // exactly at rows*cols: printing overwrites cells and scrolling rotates
    // the buffer in place, so nothing accumulates.
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    let max_cells = usize::from(state.size().0) * usize::from(state.size().1);

    let flood: Vec<u8> = (0..200_000_u32).map(|i| b'a' + ((i % 26) as u8)).collect();
    state.feed_bytes(&flood);

    assert_eq!(state.screen().cells().len(), max_cells);
    assert_invariants(&state, "printable flood");
}

#[test]
fn hostile_output_never_grows_the_screen_beyond_the_grid() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    let max_cells = usize::from(state.size().0) * usize::from(state.size().1);

    // Mix of everything nasty, repeated.
    for _ in 0..100 {
        state.feed_bytes(b"\x1b[999999999;999999999H\x1b]0;pad\x07\x1b(Baaaa");
        state.feed_bytes(b"\t\t\t\x1b[65535S\x1b[65535T\x1b[2J");
        state.feed_bytes(&[0xff, 0xc0, 0xaf, 0xed, 0xa0, 0x80]);
    }
    assert_eq!(state.screen().cells().len(), max_cells);
    assert_invariants(&state, "hostile output mix");
}
