//! Focused state regressions for DECCKM and DECKPAM/DECKPNM.

use noren_terminal::TerminalState;

fn assert_modes(state: &TerminalState, cursor_keys: bool, keypad: bool) {
    let modes = state.modes();
    assert_eq!(modes.is_application_cursor_key_mode(), cursor_keys);
    assert_eq!(modes.is_application_keypad_mode(), keypad);
}

#[test]
fn decckm_set_reset_is_incremental_and_idempotent() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    assert_modes(&state, false, false);

    state.feed_bytes(b"\x1b[?1");
    assert_modes(&state, false, false);
    state.feed_bytes(b"h\x1b[?1h");
    assert_modes(&state, true, false);

    state.feed_bytes(b"\x1b[?1l\x1b[?1l");
    assert_modes(&state, false, false);
}

#[test]
fn deckpam_and_deckpnm_set_reset_keypad_mode() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b");
    assert_modes(&state, false, false);
    state.feed_bytes(b"=\x1b=");
    assert_modes(&state, false, true);

    state.feed_bytes(b"\x1b>\x1b>");
    assert_modes(&state, false, false);
}

#[test]
fn cursor_and_keypad_modes_are_independent() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?1h");
    assert_modes(&state, true, false);
    state.feed_bytes(b"\x1b=");
    assert_modes(&state, true, true);
    state.feed_bytes(b"\x1b[?1l");
    assert_modes(&state, false, true);
    state.feed_bytes(b"\x1b>");
    assert_modes(&state, false, false);
}

#[test]
fn snapshots_capture_modes_without_following_later_changes() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[?1h\x1b=");
    let snapshot = state.snapshot();

    state.feed_bytes(b"\x1b[?1l\x1b>");

    assert!(snapshot.modes().is_application_cursor_key_mode());
    assert!(snapshot.modes().is_application_keypad_mode());
    assert_modes(&state, false, false);
}

#[test]
fn application_modes_survive_resize_and_alternate_screen_switching() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"P\x1b[?1h\x1b=\x1b[?1049hA");

    assert!(state.modes().is_alternate_screen_active());
    assert_modes(&state, true, true);
    assert_eq!(state.snapshot().lines(), ["A"]);

    state.resize(3, 5).expect("valid resize");
    state.feed_bytes(b"\x1b[?1049l");

    assert!(!state.modes().is_alternate_screen_active());
    assert_modes(&state, true, true);
    assert_eq!(state.snapshot().lines(), ["P"]);
}

#[test]
fn unsupported_private_modes_do_not_mutate_but_known_lists_still_apply() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?999h");
    assert_modes(&state, false, false);
    assert!(!state.modes().is_alternate_screen_active());

    state.feed_bytes(b"\x1b[?1;1049h");
    assert_modes(&state, true, false);
    assert!(state.modes().is_alternate_screen_active());

    state.feed_bytes(b"\x1b[?1;1049l");
    assert_modes(&state, false, false);
    assert!(!state.modes().is_alternate_screen_active());
}
