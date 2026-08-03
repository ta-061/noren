use noren_terminal::{Cell, TerminalState};

fn cursor(state: &TerminalState) -> (u16, u16) {
    (state.cursor().row(), state.cursor().column())
}

#[test]
fn entering_1049_selects_a_blank_home_positioned_alternate_buffer() {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    state.feed_bytes(b"ABCDEFGHIJ\x1b[3;4HZ");

    state.feed_bytes(b"\x1b[?1049h");

    assert_eq!(state.size(), (3, 5));
    assert_eq!(cursor(&state), (0, 0));
    assert!(state.screen().cells().iter().all(Cell::is_blank));
    assert!(state.snapshot().lines().is_empty());
    assert!(!state.is_wrap_pending());
    assert_eq!(
        (state.scroll_region().top(), state.scroll_region().bottom()),
        (0, 2)
    );
}

#[test]
fn alternate_mode_is_visible_on_state_and_captured_snapshots() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    let primary = state.snapshot();

    assert!(!state.modes().is_alternate_screen_active());
    assert!(!primary.modes().is_alternate_screen_active());

    state.feed_bytes(b"\x1b[?1049h");
    let alternate = state.snapshot();

    assert!(state.modes().is_alternate_screen_active());
    assert!(alternate.modes().is_alternate_screen_active());
    assert!(!primary.modes().is_alternate_screen_active());

    state.feed_bytes(b"\x1b[?1049l");

    assert!(!state.modes().is_alternate_screen_active());
    assert!(!state.snapshot().modes().is_alternate_screen_active());
    assert!(alternate.modes().is_alternate_screen_active());
}

#[test]
fn leaving_1049_restores_the_isolated_primary_buffer_and_entry_cursor() {
    let mut state = TerminalState::new(4, 8).expect("valid terminal");
    state.feed_bytes(b"PRIMARY\x1b[3;4H");
    let primary = state.snapshot();

    state.feed_bytes(b"\x1b[?1049hALT\x1b[4;8HZ");
    assert_eq!(state.snapshot().lines(), ["ALT", "", "", "       Z"]);
    assert_ne!(state.snapshot(), primary);

    state.feed_bytes(b"\x1b[?1049l");

    assert_eq!(state.snapshot(), primary);
    assert_eq!(cursor(&state), (2, 3));
}

#[test]
fn esc_and_csi_save_restore_sequences_restore_the_public_cursor() {
    let sequences: &[(&[u8], &[u8])] = &[
        (b"\x1b7".as_slice(), b"\x1b8".as_slice()),
        (b"\x1b[s".as_slice(), b"\x1b[u".as_slice()),
    ];

    for (save, restore) in sequences {
        let mut state = TerminalState::new(4, 6).expect("valid terminal");
        state.feed_bytes(b"\x1b[2;3H");
        state.feed_bytes(save);
        state.feed_bytes(b"\x1b[4;6H");
        assert_eq!(cursor(&state), (3, 5), "move after save {save:?}");

        state.feed_bytes(restore);
        assert_eq!(cursor(&state), (1, 2), "restore sequence {restore:?}");
    }
}

#[test]
fn repeated_1049_set_and_reset_are_idempotent() {
    let mut state = TerminalState::new(3, 6).expect("valid terminal");
    state.feed_bytes(b"PRIMARY\x1b[2;2H");
    let primary = state.snapshot();

    state.feed_bytes(b"\x1b[?1049hALT\x1b[3;5H");
    let alternate = state.snapshot();
    state.feed_bytes(b"\x1b[?1049h");
    assert_eq!(state.snapshot(), alternate);

    state.feed_bytes(b"\x1b[?1049l");
    assert_eq!(state.snapshot(), primary);
    state.feed_bytes(b"\x1b[?1049l");
    assert_eq!(state.snapshot(), primary);
}

#[test]
fn resize_while_alternate_is_active_updates_visible_and_saved_buffers() {
    let mut state = TerminalState::new(4, 5).expect("valid terminal");
    state.feed_bytes(b"AB\x1b[2;1HCD\x1b[3;1HEF\x1b[4;1HGH\x1b[4;5H");
    state.feed_bytes(b"\x1b[?1049hALT\x1b[3;3HZ\x1b[4;5H");

    state.resize(3, 4).expect("valid resize");

    let alternate = state.snapshot();
    assert_eq!(state.size(), (3, 4));
    assert_eq!((alternate.rows(), alternate.cols()), (3, 4));
    assert_eq!(alternate.lines(), ["ALT", "", "  Z"]);
    assert_eq!(cursor(&state), (2, 3));
    assert!(state.modes().is_alternate_screen_active());

    state.feed_bytes(b"\x1b[?1049l");

    let primary = state.snapshot();
    assert_eq!(state.size(), (3, 4));
    assert_eq!((primary.rows(), primary.cols()), (3, 4));
    assert_eq!(primary.lines(), ["AB", "CD", "EF"]);
    assert_eq!(cursor(&state), (2, 3));
    assert!(!state.modes().is_alternate_screen_active());
}

#[test]
fn unsupported_private_modes_leave_public_state_unchanged() {
    let mut state = TerminalState::new(3, 6).expect("valid terminal");
    state.feed_bytes(b"base\x1b[2;3H");
    let before = state.snapshot();

    state.feed_bytes(b"\x1b[?47h\x1b[?1047h\x1b[?2004h\x1b[?47l\x1b[?1047l\x1b[?2004l");

    assert_eq!(state.snapshot(), before);
    assert!(!state.modes().is_alternate_screen_active());

    state.feed_bytes(b"X");
    assert_eq!(state.snapshot().lines(), ["base", "  X"]);
    assert_eq!(cursor(&state), (1, 3));
}
