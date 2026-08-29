//! DEC private mouse tracking (9/1000/1002/1003) and encoding (1005/1006/1015)
//! mode tracking in terminal state.
//!
//! The terminal holds the mode state only; generating mouse reports stays in
//! `noren-app`'s input encoder. These tests exercise the public API
//! (`feed_bytes` + `modes()`) the way the app will read it, including a
//! DECSET split across two `feed_bytes` calls — the case where a hand-rolled
//! byte scanner would differ from the incremental parser.

use noren_terminal::TerminalState;

#[test]
fn mouse_modes_default_to_disabled() {
    let state = TerminalState::new(2, 4).expect("valid terminal");
    let modes = state.modes();
    assert!(!modes.is_mouse_x10_tracking_enabled());
    assert!(!modes.is_mouse_normal_tracking_enabled());
    assert!(!modes.is_mouse_button_event_tracking_enabled());
    assert!(!modes.is_mouse_any_event_tracking_enabled());
    assert!(!modes.is_mouse_utf8_encoding_enabled());
    assert!(!modes.is_mouse_sgr_encoding_enabled());
    assert!(!modes.is_mouse_urxvt_encoding_enabled());
}

#[test]
fn x10_mouse_tracking_mode_9_toggles_with_decset_and_decrst() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?9h");
    assert!(state.modes().is_mouse_x10_tracking_enabled());
    assert!(!state.modes().is_mouse_normal_tracking_enabled());

    state.feed_bytes(b"\x1b[?9l");
    assert!(!state.modes().is_mouse_x10_tracking_enabled());
}

#[test]
fn mouse_tracking_modes_toggle_with_decset_and_decrst() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?1000h");
    assert!(state.modes().is_mouse_normal_tracking_enabled());
    assert!(!state.modes().is_mouse_button_event_tracking_enabled());

    state.feed_bytes(b"\x1b[?1002h");
    assert!(state.modes().is_mouse_button_event_tracking_enabled());

    state.feed_bytes(b"\x1b[?1003h");
    assert!(state.modes().is_mouse_any_event_tracking_enabled());

    state.feed_bytes(b"\x1b[?1000l");
    assert!(!state.modes().is_mouse_normal_tracking_enabled());
    assert!(state.modes().is_mouse_button_event_tracking_enabled());

    state.feed_bytes(b"\x1b[?1002l\x1b[?1003l");
    assert!(!state.modes().is_mouse_button_event_tracking_enabled());
    assert!(!state.modes().is_mouse_any_event_tracking_enabled());
}

#[test]
fn mouse_encoding_modes_toggle_with_decset_and_decrst() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?1006h");
    assert!(state.modes().is_mouse_sgr_encoding_enabled());
    assert!(!state.modes().is_mouse_urxvt_encoding_enabled());

    state.feed_bytes(b"\x1b[?1015h");
    assert!(state.modes().is_mouse_urxvt_encoding_enabled());

    state.feed_bytes(b"\x1b[?1006l");
    assert!(!state.modes().is_mouse_sgr_encoding_enabled());

    state.feed_bytes(b"\x1b[?1015l");
    assert!(!state.modes().is_mouse_urxvt_encoding_enabled());

    state.feed_bytes(b"\x1b[?1005h");
    assert!(state.modes().is_mouse_utf8_encoding_enabled());
    state.feed_bytes(b"\x1b[?1005l");
    assert!(!state.modes().is_mouse_utf8_encoding_enabled());
}

#[test]
fn multi_param_mouse_modes_set_and_clear_in_order() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?1002;1006h");
    assert!(state.modes().is_mouse_button_event_tracking_enabled());
    assert!(state.modes().is_mouse_sgr_encoding_enabled());

    state.feed_bytes(b"\x1b[?1002;1006l");
    assert!(!state.modes().is_mouse_button_event_tracking_enabled());
    assert!(!state.modes().is_mouse_sgr_encoding_enabled());
}

#[test]
fn multi_param_private_modes_keep_known_modes_when_unknown_is_mixed_in() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?9999;1002h");
    assert!(state.modes().is_mouse_button_event_tracking_enabled());
}

#[test]
fn multi_param_sequence_split_across_feed_bytes_calls_still_applies() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?1002;");
    assert!(!state.modes().is_mouse_button_event_tracking_enabled());
    state.feed_bytes(b"1006h");

    assert!(state.modes().is_mouse_button_event_tracking_enabled());
    assert!(state.modes().is_mouse_sgr_encoding_enabled());
}

#[test]
fn mouse_modes_are_independent_of_screen_and_paste_modes() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[?1000h\x1b[?1006h\x1b[?1049h\x1b[?2004h");

    let modes = state.modes();
    assert!(modes.is_mouse_normal_tracking_enabled());
    assert!(modes.is_mouse_sgr_encoding_enabled());
    assert!(modes.is_alternate_screen_active());
    assert!(modes.is_bracketed_paste_enabled());

    // Leaving the alternate screen keeps mouse modes.
    state.feed_bytes(b"\x1b[?1049l");
    let modes = state.modes();
    assert!(!modes.is_alternate_screen_active());
    assert!(modes.is_mouse_normal_tracking_enabled());
    assert!(modes.is_mouse_sgr_encoding_enabled());
}

#[test]
fn decset_split_across_feed_bytes_calls_is_still_detected() {
    // A DECSET sequence split across two feed_bytes calls must still update
    // state. This is the case where a hand-rolled byte scanner that resets
    // on chunk boundaries would miss the transition; the incremental parser
    // retains its CSI state between calls.
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?10");
    // Mode is not yet set: the sequence is incomplete.
    assert!(!state.modes().is_mouse_normal_tracking_enabled());

    state.feed_bytes(b"00h");
    assert!(state.modes().is_mouse_normal_tracking_enabled());

    // Split a DECRST the same way.
    state.feed_bytes(b"\x1b[?100");
    assert!(state.modes().is_mouse_normal_tracking_enabled());

    state.feed_bytes(b"0l");
    assert!(!state.modes().is_mouse_normal_tracking_enabled());
}

#[test]
fn mouse_modes_survive_snapshot() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[?1003h\x1b[?1006h");

    let snap = state.snapshot();
    let modes = snap.modes();
    assert!(modes.is_mouse_any_event_tracking_enabled());
    assert!(modes.is_mouse_sgr_encoding_enabled());
    assert!(!modes.is_mouse_normal_tracking_enabled());
}
