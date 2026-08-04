//! Issue #46: the legacy X10 mouse report `CSI M` corrupts the screen.
//!
//! These are state-level regressions through `TerminalState`: a `DECSET` that
//! enables a mouse tracking mode updates live mode state, the per-byte flag in
//! `feed_bytes` reads that state, and a following `CSI M` is then consumed as a
//! report introducer instead of executing as DeleteLines. The parser-unit
//! behaviour is covered in `parser::tests`; this file proves the wiring and the
//! observable screen/mode contract.

use noren_terminal::{MouseModes, TerminalState};

/// Read the six mouse-mode flags as a fixed-order array for indexed checks.
fn mouse_flags(m: MouseModes) -> [bool; 6] {
    [
        m.is_normal_click_tracking(),
        m.is_button_event_tracking(),
        m.is_any_event_tracking(),
        m.is_utf8_coordinate_encoding(),
        m.is_sgr_encoding(),
        m.is_urxvt_encoding(),
    ]
}

fn feed_bytewise(state: &mut TerminalState, bytes: &[u8]) {
    for byte in bytes {
        state.feed_bytes(std::slice::from_ref(byte));
    }
}

// ===== The issue #46 reproduction: screen unchanged under a tracking mode =====

#[test]
fn csi_m_report_leaves_the_screen_unchanged_under_each_tracking_mode() {
    for enable in [b"\x1b[?1000h".as_slice(), b"\x1b[?1002h", b"\x1b[?1003h"] {
        let mut state = TerminalState::new(3, 3).expect("valid terminal");
        state.feed_bytes(b"AAA\r\nBBB\r\nCCC");
        let before = state.snapshot().lines().to_vec();
        let cursor_before = state.cursor();

        state.feed_bytes(enable);
        // The exact coordinator reproduction: CSI M + three coordinate bytes.
        // (\x20 \x25 \x27 are space, %, ' — previously printed as text after a
        // DeleteLines(1) corrupted the last row.)
        state.feed_bytes(b"\x1b[M\x20\x25\x27");

        assert_eq!(
            state.snapshot().lines(),
            before.as_slice(),
            "screen must be untouched under {enable:?}"
        );
        assert_eq!(
            state.cursor(),
            cursor_before,
            "a report introduces no cursor movement"
        );
    }
}

#[test]
fn csi_m_report_data_bytes_are_never_printed_as_text() {
    // Position the cursor over existing text and feed a report whose payload
    // bytes would overwrite that text if they were printed. They must not be.
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"XXXX");
    state.feed_bytes(b"\x1b[?1000h");
    state.feed_bytes(b"\x1b[1;1H");
    state.feed_bytes(b"\x1b[M!\"#");

    assert_eq!(state.snapshot().lines(), ["XXXX"]);
}

// ===== No mouse mode: CSI M keeps its ECMA-48 DeleteLines meaning =====

#[test]
fn csi_m_deletes_a_line_with_no_mouse_mode_enabled() {
    let mut state = TerminalState::new(3, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\r\nBBB\r\nCCC");
    // Cursor at row 0; bare CSI M is DeleteLines(1): row 0 drops, rows shift up.
    state.feed_bytes(b"\x1b[1;1H\x1b[M");
    assert_eq!(state.snapshot().lines(), ["BBB", "CCC"]);
}

#[test]
fn csi_m_data_bytes_print_as_text_with_no_mouse_mode_enabled() {
    // The corruption the fix prevents: without a mouse mode the three bytes
    // after CSI M are ordinary printable text (CSI M itself is DeleteLines).
    let mut state = TerminalState::new(2, 6).expect("valid terminal");
    state.feed_bytes(b"\x1b[1;1H\x1b[M %'");
    // CSI M deletes row 0 (blanking via scroll-up of the single-line region's
    // top), then " %'" prints at the start of row 0.
    assert_eq!(state.snapshot().lines(), [" %'"]);
}

// ===== Payload control bytes / ESC never desync the parser =====

#[test]
fn report_payload_with_esc_and_controls_does_not_desync() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[?1000hAB");
    let before = state.snapshot().lines().to_vec();

    // Payload bytes ESC, LF, BEL are raw binary coordinates: none may start an
    // escape, scroll, or ring — they are swallowed and the count stays exact.
    state.feed_bytes(b"\x1b[M\x1b\n\x07");
    assert_eq!(state.snapshot().lines(), before.as_slice());

    // The parser resynchronises on the byte after Cy: text prints in place.
    state.feed_bytes(b"CD");
    assert_eq!(state.snapshot().lines(), ["ABCD"]);
}

// ===== A report split one byte at a time matches a single feed =====

#[test]
fn report_split_one_byte_at_a_time_matches_a_single_feed() {
    let stream = b"\x1b[?1000h\x1b[M\x20\x25\x27X";

    let mut whole = TerminalState::new(2, 4).expect("valid terminal");
    whole.feed_bytes(stream);

    let mut split = TerminalState::new(2, 4).expect("valid terminal");
    feed_bytewise(&mut split, stream);

    assert_eq!(whole.snapshot(), split.snapshot());
    // The three report bytes were consumed; X is the first printable after.
    assert_eq!(split.snapshot().lines(), ["X"]);
}

// ===== Mouse mode enable/disable round-trips in the snapshot =====

#[test]
fn each_mouse_mode_round_trips_in_the_snapshot() {
    // (DEC private mode number, index into mouse_flags()).
    let cases = [
        (1000, 0),
        (1002, 1),
        (1003, 2),
        (1005, 3),
        (1006, 4),
        (1015, 5),
    ];

    for (num, idx) in cases {
        let mut state = TerminalState::new(1, 2).expect("valid terminal");

        // Default: every mouse flag is off.
        assert!(
            mouse_flags(state.snapshot().modes().mouse())
                .iter()
                .all(|flag| !flag),
            "mode {num}: all flags start off"
        );

        // Enable: only the targeted flag flips on; siblings stay off.
        state.feed_bytes(format!("\x1b[?{num}h").as_bytes());
        let flags = mouse_flags(state.snapshot().modes().mouse());
        for (i, &flag) in flags.iter().enumerate() {
            assert_eq!(flag, i == idx, "mode {num}: enable flag[{i}]");
        }

        // Disable: back to all off.
        state.feed_bytes(format!("\x1b[?{num}l").as_bytes());
        assert!(
            mouse_flags(state.snapshot().modes().mouse())
                .iter()
                .all(|flag| !flag),
            "mode {num}: disable clears the flag"
        );
    }
}

#[test]
fn only_tracking_modes_enable_the_csi_m_disambiguation() {
    // The encoding-only modes (1005/1006/1015) never produce a report on their
    // own, so they must not flip the tracking flag that reclassifies CSI M.
    let mut state = TerminalState::new(1, 2).expect("valid terminal");
    for enc in [1005, 1006, 1015] {
        state.feed_bytes(format!("\x1b[?{enc}h").as_bytes());
        assert!(
            !state.modes().is_mouse_tracking_enabled(),
            "encoding mode {enc} must not enable tracking"
        );
        state.feed_bytes(format!("\x1b[?{enc}l").as_bytes());
    }

    // Each tracking mode alone is sufficient to arm the CSI M disambiguation.
    for track in [1000, 1002, 1003] {
        state.feed_bytes(format!("\x1b[?{track}h").as_bytes());
        assert!(
            state.modes().is_mouse_tracking_enabled(),
            "tracking mode {track} arms disambiguation"
        );
        state.feed_bytes(format!("\x1b[?{track}l").as_bytes());
        assert!(
            !state.modes().is_mouse_tracking_enabled(),
            "disabling {track} disarms disambiguation"
        );
    }
}

// ===== SGR-form reports stay harmless (issue #41 regression) =====

#[test]
fn sgr_form_mouse_reports_are_harmless_under_tracking() {
    // Issue #41 fixed the `<` private marker so a SGR-mouse-shaped CSI is
    // poisoned and never executes. Re-assert it under a tracking mode so a
    // future change to the marker handling cannot regress #46 alongside it.
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"AB\r\nCD");
    let before = state.snapshot().lines().to_vec();

    state.feed_bytes(b"\x1b[?1006h\x1b[?1000h");
    // Press (`...M`) and release (`...m`) SGR reports.
    state.feed_bytes(b"\x1b[<0;5;7M\x1b[<0;5;7m");

    assert_eq!(state.snapshot().lines(), before.as_slice());
}
