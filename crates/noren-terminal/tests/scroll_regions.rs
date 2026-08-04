use noren_terminal::{CursorMove, TerminalError, TerminalState};

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

fn region(state: &TerminalState) -> (u16, u16) {
    (state.scroll_region().top(), state.scroll_region().bottom())
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

fn labeled_rows() -> TerminalState {
    let mut state = TerminalState::new(5, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\x1b[2;1HBBB\x1b[3;1HCCC\x1b[4;1HDDD\x1b[5;1HEEE");
    state
}

#[test]
fn decstbm_clamps_out_of_range_and_rejects_inverted_regions() {
    let mut state = labeled_rows();

    state.feed_bytes(b"\x1b[;4r");
    assert_eq!(region(&state), (0, 3));
    assert_eq!(cursor(&state), (0, 0));

    state.feed_bytes(b"\x1b[2r");
    assert_eq!(region(&state), (1, 4));
    assert_eq!(cursor(&state), (0, 0));

    // Genuine rejections: top >= bottom (even after clamping) leave the
    // current region and cursor untouched. ESC[3;3r, ESC[5;2r, and ESC[4;2r
    // all collapse to top >= bottom and are dropped, so the cursor stays.
    state.feed_bytes(b"\x1b[4;1H");
    assert_eq!(cursor(&state), (3, 0));
    state.feed_bytes(b"\x1b[3;3r\x1b[5;2r\x1b[4;2r");
    assert_eq!(region(&state), (1, 4));
    assert_eq!(cursor(&state), (3, 0));
    assert_eq!(rows(&state), ["AAA", "BBB", "CCC", "DDD", "EEE"]);

    // Out-of-range margins now clamp to the last row instead of rejecting.
    // ESC[1;6r on a 5-row terminal clamps bottom 5 -> 4, accepting (0, 4).
    state.feed_bytes(b"\x1b[1;6r");
    assert_eq!(region(&state), (0, 4));
    assert_eq!(cursor(&state), (0, 0));

    // ESC[2;99r clamps bottom 98 -> 4 and accepts (1, 4).
    state.feed_bytes(b"\x1b[2;99r");
    assert_eq!(region(&state), (1, 4));
    assert_eq!(cursor(&state), (0, 0));

    // The public set_scroll_region API rejects top >= bottom just like CSI.
    assert_eq!(
        state.set_scroll_region(3, 3),
        Err(TerminalError::InvalidScrollRegion)
    );
    assert_eq!(region(&state), (1, 4));

    // The public API clamps identically to the CSI path: out-of-range bottom
    // is accepted after clamping rather than erroring.
    assert_eq!(state.set_scroll_region(0, 99), Ok(()));
    assert_eq!(region(&state), (0, 4));

    state.feed_bytes(b"\x1b[r");
    assert_eq!(region(&state), (0, 4));
    assert_eq!(cursor(&state), (0, 0));

    state.feed_bytes(b"\x1b[2;4r\x1b[4;1H\x1b[S");
    assert_eq!(rows(&state), ["AAA", "CCC", "DDD", "   ", "EEE"]);
    assert_eq!(cursor(&state), (3, 0));
}

#[test]
fn line_feed_vertical_tab_form_feed_and_index_scroll_at_bottom_margin() {
    for control in [b"\n".as_slice(), b"\x0b", b"\x0c", b"\x1bD"] {
        let mut state = labeled_rows();
        state.feed_bytes(b"\x1b[2;4r\x1b[4;2H");
        state.feed_bytes(control);

        assert_eq!(rows(&state), ["AAA", "CCC", "DDD", "   ", "EEE"]);
        assert_eq!(cursor(&state), (3, 1));
        assert_eq!(region(&state), (1, 3));
    }
}

#[test]
fn reverse_index_scrolls_at_top_margin_without_moving_cursor() {
    let mut state = labeled_rows();
    state.feed_bytes(b"\x1b[2;4r\x1b[2;3H\x1bM");

    assert_eq!(rows(&state), ["AAA", "   ", "BBB", "CCC", "EEE"]);
    assert_eq!(cursor(&state), (1, 2));
    assert_eq!(region(&state), (1, 3));
}

#[test]
fn explicit_scroll_defaults_and_bounds_counts_to_region_height() {
    let mut up = labeled_rows();
    up.feed_bytes(b"\x1b[2;4r\x1b[3;2H\x1b[S");
    assert_eq!(rows(&up), ["AAA", "CCC", "DDD", "   ", "EEE"]);
    assert_eq!(cursor(&up), (2, 1));

    up.feed_bytes(b"\x1b[999S");
    assert_eq!(rows(&up), ["AAA", "   ", "   ", "   ", "EEE"]);
    assert_eq!(cursor(&up), (2, 1));

    let mut down = labeled_rows();
    down.feed_bytes(b"\x1b[2;4r\x1b[3;2H\x1b[T");
    assert_eq!(rows(&down), ["AAA", "   ", "BBB", "CCC", "EEE"]);
    assert_eq!(cursor(&down), (2, 1));

    down.feed_bytes(b"\x1b[999T");
    assert_eq!(rows(&down), ["AAA", "   ", "   ", "   ", "EEE"]);
    assert_eq!(cursor(&down), (2, 1));
}

#[test]
fn cnl_cpl_and_vpa_apply_defaults_reset_column_and_clamp() {
    let mut state = TerminalState::new(4, 5).expect("valid terminal");
    state.feed_bytes(b"\x1b[2;4H\x1b[E");
    assert_eq!(cursor(&state), (2, 0));

    state.feed_bytes(b"\x1b[99E");
    assert_eq!(cursor(&state), (3, 0));
    state.feed_bytes(b"\x1b[4G\x1b[F");
    assert_eq!(cursor(&state), (2, 0));
    state.feed_bytes(b"\x1b[99F");
    assert_eq!(cursor(&state), (0, 0));

    state.feed_bytes(b"\x1b[5G\x1b[d");
    assert_eq!(cursor(&state), (0, 4));
    state.feed_bytes(b"\x1b[99d");
    assert_eq!(cursor(&state), (3, 4));
}

#[test]
fn cursor_and_control_actions_cancel_delayed_wrap() {
    let cancellations: &[&[u8]] = &[
        b"\x08",
        b"\n",
        b"\x0b",
        b"\x0c",
        b"\x1bD",
        b"\x1bE",
        b"\x1bM",
        b"\x1b[A",
        b"\x1b[B",
        b"\x1b[C",
        b"\x1b[D",
        b"\x1b[E",
        b"\x1b[F",
        b"\x1b[G",
        b"\x1b[H",
        b"\x1b[d",
        b"\x1b[S",
        b"\x1b[T",
        b"\x1b[2;3r",
    ];

    for sequence in cancellations {
        let mut state = TerminalState::new(3, 3).expect("valid terminal");
        state.feed_bytes(b"abc");
        assert!(state.is_wrap_pending(), "precondition for {sequence:?}");

        state.feed_bytes(sequence);
        assert!(!state.is_wrap_pending(), "sequence {sequence:?}");
    }

    let mut via_api = TerminalState::new(2, 3).expect("valid terminal");
    via_api.feed_bytes(b"abc");
    via_api.move_cursor(CursorMove::ToRow(1));
    assert!(!via_api.is_wrap_pending());
}

#[test]
fn resize_preserves_overlap_clamps_cursor_and_resets_scroll_state() {
    let mut state = TerminalState::new(4, 4).expect("valid terminal");
    state.feed_bytes(b"ABCD\x1b[2;1HEF\x1b[3;1HGHI\x1b[4;1HJKLM");
    state.feed_bytes(b"\x1b[2;4r\x1b[4;4HZ");
    assert!(state.is_wrap_pending());

    state.resize(3, 2).expect("valid resize");
    assert_eq!(rows(&state), ["AB", "EF", "GH"]);
    assert_eq!(cursor(&state), (2, 1));
    assert_eq!(region(&state), (0, 2));
    assert!(!state.is_wrap_pending());

    state.resize(5, 5).expect("valid resize");
    assert_eq!(rows(&state), ["AB   ", "EF   ", "GH   ", "     ", "     "]);
    assert_eq!(cursor(&state), (2, 1));
    assert_eq!(region(&state), (0, 4));
}

#[test]
fn printable_ascii_and_ignored_controls_remain_stable() {
    let mut state = TerminalState::new(2, 5).expect("valid terminal");
    state.feed_bytes(b"A\0B\x07C\x7fD");

    assert_eq!(rows(&state), ["ABCD ", "     "]);
    assert_eq!(cursor(&state), (0, 4));
    assert!(!state.is_wrap_pending());
}

/// DECSTBM clamping must hold when the sequence is split across feed
/// boundaries, where this parser has historically been fragile. Feeding the
/// whole script one byte at a time must match feeding it as a single chunk.
#[test]
fn decstbm_clamp_survives_byte_at_a_time_feeding() {
    let script: &[&[u8]] = &[
        // Out-of-range bottom clamps to the last row.
        b"\x1b[1;6r",
        // Region valid only after clamping is accepted.
        b"\x1b[2;99r",
        // top >= bottom after clamping is rejected; prior region preserved.
        b"\x1b[4;2r",
    ];

    let mut chunked = labeled_rows();
    for bytes in script {
        chunked.feed_bytes(bytes);
    }

    let mut bytewise = labeled_rows();
    for bytes in script {
        for byte in *bytes {
            bytewise.feed_bytes(std::slice::from_ref(byte));
        }
    }

    assert_eq!(region(&chunked), (1, 4));
    assert_eq!(region(&bytewise), (1, 4));
    assert_eq!(cursor(&chunked), cursor(&bytewise));
    assert_eq!(chunked.snapshot(), bytewise.snapshot());
}
