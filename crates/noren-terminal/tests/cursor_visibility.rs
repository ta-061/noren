//! DEC private mode 25 (DECTCEM, text cursor enable) tracking in terminal
//! state.
//!
//! Programs rely on hiding the cursor while they own the screen: `vim` sends
//! `CSI ?25l` before every redraw and `CSI ?25h` at rest, and a renderer that
//! ignores the mode paints a caret over screens that asked for none. The
//! renderer therefore needs the mode as terminal state, alongside every other
//! render-affecting mode, and the default must be *visible* — a terminal that
//! never draws its caret (issue #197/#200) hides where typing will land.

use noren_terminal::TerminalState;

#[test]
fn cursor_defaults_to_visible() {
    let state = TerminalState::new(2, 4).expect("valid terminal");
    assert!(state.modes().is_cursor_visible());
    assert!(state.snapshot().is_cursor_visible());
}

#[test]
fn cursor_visibility_toggles_with_csi_question_25_h_and_l() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?25l");
    assert!(!state.modes().is_cursor_visible());
    assert!(!state.snapshot().is_cursor_visible());

    state.feed_bytes(b"\x1b[?25h");
    assert!(state.modes().is_cursor_visible());
    assert!(state.snapshot().is_cursor_visible());

    // Repeated hiding is idempotent.
    state.feed_bytes(b"\x1b[?25l\x1b[?25l");
    assert!(!state.modes().is_cursor_visible());
}

#[test]
fn cursor_visibility_survives_alternate_screen_transitions() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[?25l");
    // 1049 saves and restores the *position*, not DECTCEM; xterm keeps mode
    // 25 global across screen buffers, and a program leaving the alternate
    // screen without re-showing expects its own final `?25h` to be the
    // authority.
    state.feed_bytes(b"\x1b[?1049h");
    assert!(!state.modes().is_cursor_visible());
    state.feed_bytes(b"\x1b[?1049l");
    assert!(!state.modes().is_cursor_visible());
    state.feed_bytes(b"\x1b[?25h");
    assert!(state.modes().is_cursor_visible());
}

#[test]
fn cursor_visibility_participates_in_multi_param_private_modes() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");

    state.feed_bytes(b"\x1b[?1;25;2004h");
    let modes = state.modes();
    assert!(modes.is_application_cursor_key_mode());
    assert!(modes.is_cursor_visible());
    assert!(modes.is_bracketed_paste_enabled());

    state.feed_bytes(b"\x1b[?1;25;2004l");
    let modes = state.modes();
    assert!(!modes.is_application_cursor_key_mode());
    assert!(!modes.is_cursor_visible());
    assert!(!modes.is_bracketed_paste_enabled());
}

#[test]
fn hiding_the_cursor_leaves_the_tracked_position_intact() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"ab");
    let (row, column) = {
        let cursor = state.cursor();
        (cursor.row(), cursor.column())
    };
    state.feed_bytes(b"\x1b[?25l");
    let cursor = state.cursor();
    assert_eq!((cursor.row(), cursor.column()), (row, column));
}
