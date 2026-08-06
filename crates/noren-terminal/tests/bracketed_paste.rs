//! DEC private mode 2004 (bracketed paste) tracking in terminal state.
//!
//! Noren pastes are gated on this mode: paste text may only reach the PTY
//! wrapped in `CSI 200 ~` / `CSI 201 ~` when the application has enabled
//! 2004. Tracking therefore lives in the terminal state where every other
//! input-affecting mode lives.

use noren_terminal::TerminalState;

#[test]
fn bracketed_paste_defaults_to_disabled() {
    let state = TerminalState::new(2, 4).expect("valid terminal");
    assert!(!state.modes().is_bracketed_paste_enabled());
    assert!(!state.snapshot().modes().is_bracketed_paste_enabled());
}

#[test]
fn bracketed_paste_toggles_with_csi_question_2004_h_and_l() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?2004h");
    assert!(state.modes().is_bracketed_paste_enabled());
    assert!(state.snapshot().modes().is_bracketed_paste_enabled());

    state.feed_bytes(b"\x1b[?2004l");
    assert!(!state.modes().is_bracketed_paste_enabled());

    // Repeated enabling is idempotent.
    state.feed_bytes(b"\x1b[?2004h\x1b[?2004h");
    assert!(state.modes().is_bracketed_paste_enabled());
}

#[test]
fn bracketed_paste_is_independent_of_cursor_and_screen_modes() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[?2004h\x1b[?1h\x1b[?1049h");

    let modes = state.modes();
    assert!(modes.is_bracketed_paste_enabled());
    assert!(modes.is_application_cursor_key_mode());
    assert!(modes.is_alternate_screen_active());

    // Leaving the alternate screen keeps the paste mode.
    state.feed_bytes(b"\x1b[?1049l");
    assert!(state.modes().is_bracketed_paste_enabled());
    assert!(!state.modes().is_alternate_screen_active());
}
