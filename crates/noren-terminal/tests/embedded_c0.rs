//! DEC VT / xterm conformance for C0 controls embedded in a control sequence.

use noren_terminal::TerminalState;

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

/// `ESC [ <LF> 2 A` executes the embedded LF (cursor down one) and then runs
/// `CSI 2 A` (cursor up two) without aborting. From row 3 on a 5-row terminal
/// the cursor moves 3 -> 4 (LF) -> 2 (CUU 2). Pre-fix the LF was lost and the
/// cursor would have ended at row 1.
#[test]
fn embedded_lf_in_csi_moves_down_then_up() {
    let mut state = TerminalState::new(5, 5).expect("valid terminal");
    state.feed_bytes(b"\x1b[4;1H");
    assert_eq!(cursor(&state), (3, 0));

    state.feed_bytes(b"\x1b[\n2A");
    assert_eq!(cursor(&state), (2, 0));
}

/// The literal task sequence `ESC [ 1 <LF> 2 A` no longer drops the LF. The
/// digits concatenate across the embedded C0 (the execute action does not
/// commit the parameter), so this is `CSI 12 A` after the LF.
#[test]
fn embedded_lf_with_digits_on_both_sides_is_not_lost() {
    let mut state = TerminalState::new(5, 5).expect("valid terminal");
    state.feed_bytes(b"\x1b[2;1H");
    assert_eq!(cursor(&state), (1, 0));

    state.feed_bytes(b"\x1b[1\n2A");
    // LF moves 1 -> 2, then CSI 12 A clamps back to row 0. The point is that
    // the LF executed (pre-fix the cursor would have moved 1 -> 0 directly).
    assert_eq!(cursor(&state), (0, 0));

    // Discriminating check: an embedded LF alone inside a CSI moves the cursor
    // down, proving it executes rather than vanishing.
    let mut state = TerminalState::new(5, 5).expect("valid terminal");
    state.feed_bytes(b"\x1b[2;1H\x1b[\n");
    assert_eq!(cursor(&state), (2, 0));
}

/// A C0 inside a CSI does not abort the sequence: the final byte still
/// dispatches after the embedded control runs.
#[test]
fn embedded_c0_does_not_abort_the_csi() {
    let mut state = TerminalState::new(5, 5).expect("valid terminal");
    state.feed_bytes(b"\x1b[4;1H\x1b[\r2A");
    // CR runs (column -> 0, already 0), then CSI 2 A moves up two: 3 -> 1.
    assert_eq!(cursor(&state), (1, 0));
}

/// `ESC` inside a CSI still aborts it and starts a fresh escape.
#[test]
fn embedded_esc_still_aborts_the_csi() {
    let mut state = TerminalState::new(5, 5).expect("valid terminal");
    // ESC[2 is aborted by the second ESC; ESC[3;1H moves to row 2, then Z.
    state.feed_bytes(b"\x1b[2\x1b[3;1HZ");
    assert_eq!(cursor(&state), (2, 1));
    assert_eq!(state.screen().cell(2, 0).map(|c| c.text()), Some("Z"));
}

/// CAN (0x18) and SUB (0x1a) inside a CSI behave as before: no action, no
/// abort. The surrounding sequence still completes.
#[test]
fn embedded_can_and_sub_do_not_abort_or_act() {
    for control in [0x18_u8, 0x1a] {
        let mut state = TerminalState::new(5, 5).expect("valid terminal");
        state.feed_bytes(b"\x1b[4;1H");
        let before = cursor(&state);
        // The control itself does not move the cursor.
        state.feed_bytes(&[0x1b, b'[', b'2', control]);
        assert_eq!(
            cursor(&state),
            before,
            "control {control:#04x} moved cursor"
        );
        // The CSI still completes: the final A moves the cursor up two.
        state.feed_bytes(b"A");
        assert_eq!(cursor(&state), (1, 0), "control {control:#04x} aborted");
    }
}

/// Split-sequence robustness: feeding the bytes one at a time must produce the
/// same result as a single chunk. This is where the parser has been fragile.
#[test]
fn embedded_c0_survives_byte_at_a_time_feeding() {
    let sequence: &[&[u8]] = &[
        b"\x1b[4;1H",
        b"\x1b[\n2A",
        b"\x1b[2\x1b[3;1H",
        b"\x1b[2\x18A",
        b"\x1b[2\x1aA",
    ];

    let mut chunked = TerminalState::new(5, 5).expect("valid terminal");
    for bytes in sequence {
        chunked.feed_bytes(bytes);
    }

    let mut bytewise = TerminalState::new(5, 5).expect("valid terminal");
    for bytes in sequence {
        for byte in *bytes {
            bytewise.feed_bytes(std::slice::from_ref(byte));
        }
    }

    assert_eq!(chunked.cursor(), bytewise.cursor());
    assert_eq!(chunked.snapshot(), bytewise.snapshot());
}
