use noren_terminal::{CursorMove, TerminalState};

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

fn rows(state: &TerminalState) -> Vec<String> {
    (0..state.screen().rows())
        .map(|row| {
            (0..state.screen().cols())
                .map(|column| {
                    state
                        .screen()
                        .cell(row, column)
                        .expect("cell is in bounds")
                        .text()
                })
                .collect()
        })
        .collect()
}

fn filled_screen() -> TerminalState {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    state.feed_bytes(b"ABCDE\x1b[2;1HFGHIJ\x1b[3;1HKLMNO");
    state
}

fn labeled_rows() -> TerminalState {
    let mut state = TerminalState::new(5, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\x1b[2;1HBBB\x1b[3;1HCCC\x1b[4;1HDDD\x1b[5;1HEEE");
    state
}

fn operate_on_line(column: u16, sequence: &[u8]) -> TerminalState {
    let mut state = TerminalState::new(1, 5).expect("valid terminal");
    state.feed_bytes(b"ABCDE");
    state.move_cursor(CursorMove::ToColumn(column));
    state.feed_bytes(sequence);
    state
}

#[test]
fn ed_modes_include_the_cursor_cell_and_preserve_the_cursor() {
    let cases: &[(&[u8], [&str; 3])] = &[
        (b"\x1b[J", ["ABCDE", "FG   ", "     "]),
        (b"\x1b[1J", ["     ", "   IJ", "KLMNO"]),
        (b"\x1b[2J", ["     ", "     ", "     "]),
    ];

    for (sequence, expected) in cases {
        let mut state = filled_screen();
        state.feed_bytes(b"\x1b[2;3H");

        state.feed_bytes(sequence);

        assert_eq!(rows(&state), *expected, "sequence {sequence:?}");
        assert_eq!(cursor(&state), (1, 2), "sequence {sequence:?}");
    }
}

#[test]
fn el_modes_only_change_the_active_row_and_preserve_the_cursor() {
    let cases: &[(&[u8], [&str; 3])] = &[
        (b"\x1b[0K", ["ABCDE", "FG   ", "KLMNO"]),
        (b"\x1b[1K", ["ABCDE", "   IJ", "KLMNO"]),
        (b"\x1b[2K", ["ABCDE", "     ", "KLMNO"]),
    ];

    for (sequence, expected) in cases {
        let mut state = filled_screen();
        state.feed_bytes(b"\x1b[2;3H");

        state.feed_bytes(sequence);

        assert_eq!(rows(&state), *expected, "sequence {sequence:?}");
        assert_eq!(cursor(&state), (1, 2), "sequence {sequence:?}");
    }
}

#[test]
fn character_operations_default_zero_shift_and_clamp_at_the_line_boundary() {
    let cases: &[(&[u8], u16, &str)] = &[
        (b"\x1b[X", 1, "A CDE"),
        (b"\x1b[0X", 1, "A CDE"),
        (b"\x1b[2X", 1, "A  DE"),
        (b"\x1b[99999X", 3, "ABC  "),
        (b"\x1b[@", 1, "A BCD"),
        (b"\x1b[0@", 1, "A BCD"),
        (b"\x1b[2@", 1, "A  BC"),
        (b"\x1b[99999@", 3, "ABC  "),
        (b"\x1b[P", 1, "ACDE "),
        (b"\x1b[0P", 1, "ACDE "),
        (b"\x1b[2P", 1, "ADE  "),
        (b"\x1b[99999P", 3, "ABC  "),
    ];

    for (sequence, column, expected) in cases {
        let state = operate_on_line(*column, sequence);

        assert_eq!(rows(&state), [*expected], "sequence {sequence:?}");
        assert_eq!(cursor(&state), (0, *column), "sequence {sequence:?}");
    }
}

#[test]
fn line_operations_are_clamped_from_the_cursor_to_the_bottom_margin() {
    let cases: &[(&[u8], [&str; 5])] = &[
        (b"\x1b[L", ["AAA", "BBB", "   ", "CCC", "EEE"]),
        (b"\x1b[0M", ["AAA", "BBB", "DDD", "   ", "EEE"]),
        (b"\x1b[99999L", ["AAA", "BBB", "   ", "   ", "EEE"]),
        (b"\x1b[99999M", ["AAA", "BBB", "   ", "   ", "EEE"]),
    ];

    for (sequence, expected) in cases {
        let mut state = labeled_rows();
        state.feed_bytes(b"\x1b[2;4r\x1b[3;2H");

        state.feed_bytes(sequence);

        assert_eq!(rows(&state), *expected, "sequence {sequence:?}");
        assert_eq!(cursor(&state), (2, 1), "sequence {sequence:?}");
    }
}

#[test]
fn line_operations_are_ignored_when_the_cursor_is_outside_the_scroll_region() {
    for (sequence, position, expected_cursor) in [
        (b"\x1b[L".as_slice(), b"\x1b[1;2H".as_slice(), (0, 1)),
        (b"\x1b[M".as_slice(), b"\x1b[5;2H".as_slice(), (4, 1)),
    ] {
        let mut state = labeled_rows();
        state.feed_bytes(b"\x1b[2;4r");
        state.feed_bytes(position);
        let before = state.snapshot();

        state.feed_bytes(sequence);

        assert_eq!(state.snapshot().screen(), before.screen());
        assert_eq!(cursor(&state), expected_cursor, "sequence {sequence:?}");
    }
}

#[test]
fn unsupported_modes_extra_parameters_and_parameter_overflow_are_ignored() {
    let mut state = filled_screen();
    state.feed_bytes(b"\x1b[2;3H");
    let before = state.snapshot();

    state.feed_bytes(
        b"\x1b[3J\x1b[1;2K\x1b[1;2X\x1b[1;2@\x1b[1;2P\x1b[1;2L\x1b[1;2M\
          \x1b[1;1;1;1;1;1;1;1;1X",
    );

    assert_eq!(state.snapshot(), before);
}
