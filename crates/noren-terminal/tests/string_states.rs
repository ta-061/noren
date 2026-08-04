//! BUG-01 / BUG-03 regressions for control-string swallowing and private CSI
//! marker poisoning.
//!
//! These cover the two adversarial parser defects in Issue #41:
//! - DCS/SOS/PM/APC payloads must not render as screen text (BUG-01).
//! - CSI private markers `<` and `=` must poison the sequence (BUG-03).
//!
//! The five control-string introducers all share one swallowing state that
//! consumes their payload until ST (`ESC \`) or BEL and stores nothing, so the
//! machine stays bounded for arbitrarily long payloads.

use noren_terminal::TerminalState;

/// Feed `bytes` one byte per `feed_bytes` call: the worst case for parser
/// state retention across chunk boundaries.
fn feed_bytewise(state: &mut TerminalState, bytes: &[u8]) {
    for byte in bytes {
        state.feed_bytes(std::slice::from_ref(byte));
    }
}

// ===== BUG-01: control-string payloads must not render =====

/// Each of `ESC P` (DCS), `ESC X` (SOS), `ESC ^` (PM), `ESC _` (APC) swallows
/// its payload and leaves the screen empty, terminated by either ST or BEL.
/// Output after the terminator renders normally.
#[test]
fn control_string_payloads_are_swallowed_until_st_or_bel() {
    for introducer in [b'P', b'X', b'^', b'_'] {
        for terminator in [b"\x1b\\".as_slice(), b"\x07".as_slice()] {
            let mut sequence = vec![0x1b, introducer];
            sequence.extend_from_slice(b"1;2|SPOOFED-payload");
            sequence.extend_from_slice(terminator);

            let label = format!("introducer {introducer:?}, terminator {terminator:?}");

            // Whole-feed: screen stays empty, then a trailing byte renders.
            let mut state = TerminalState::new(1, 24).expect("valid terminal");
            state.feed_bytes(&sequence);
            assert!(state.snapshot().lines().is_empty(), "{label}: screen empty");
            state.feed_bytes(b"Z");
            assert_eq!(
                state.snapshot().lines(),
                ["Z".to_owned()],
                "{label}: recovers"
            );

            // Byte-at-a-time: identical result.
            let mut split = TerminalState::new(1, 24).expect("valid terminal");
            feed_bytewise(&mut split, &sequence);
            assert!(
                split.snapshot().lines().is_empty(),
                "{label}: split screen empty"
            );
            split.feed_bytes(b"Z");
            assert_eq!(
                split.snapshot().lines(),
                ["Z".to_owned()],
                "{label}: split recovers"
            );
        }
    }
}

/// A payload split one byte at a time never lets an introducer byte or a
/// payload byte reach Ground as printable text, and the terminator recovers
/// the parser no matter where the chunk boundaries fall.
#[test]
fn control_string_payloads_survive_byte_at_a_time_feeding() {
    for introducer in [b'P', b'X', b'^', b'_'] {
        // ST-terminated and BEL-terminated, each with a trailing print.
        let st_seq: Vec<u8> = [0x1b, introducer]
            .into_iter()
            .chain(b"mid-payload".iter().copied())
            .chain([0x1b, b'\\'])
            .chain([b'Q'])
            .collect();
        let bel_seq: Vec<u8> = [0x1b, introducer]
            .into_iter()
            .chain(b"mid-payload".iter().copied())
            .chain([0x07])
            .chain([b'R'])
            .collect();

        for (name, seq) in [("ST", st_seq), ("BEL", bel_seq)] {
            let mut whole = TerminalState::new(1, 24).expect("valid terminal");
            whole.feed_bytes(&seq);

            let mut split = TerminalState::new(1, 24).expect("valid terminal");
            feed_bytewise(&mut split, &seq);

            assert_eq!(
                whole.snapshot(),
                split.snapshot(),
                "introducer {introducer:?} / {name}: byte-at-a-time diverged"
            );
            assert_eq!(
                split.snapshot().lines().len(),
                1,
                "introducer {introducer:?} / {name}: exactly one row rendered"
            );
        }
    }
}

/// An 8 MiB unterminated DCS payload must not accumulate: the screen stays
/// empty, the cell count stays exactly at the grid size across repeated rounds,
/// and the parser still recovers once a terminator finally arrives. The parser
/// is fixed-size (no heap payload buffer), so this is a regression guard, not a
/// stress test of allocation.
#[test]
fn eight_mib_unterminated_dcs_does_not_accumulate() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    let cap = usize::from(state.size().0) * usize::from(state.size().1);

    // ESC P then 8 MiB of payload with no terminator.
    let mut flood = vec![0x1b, b'P'];
    flood.extend(std::iter::repeat_n(b'x', 8 * 1024 * 1024));

    state.feed_bytes(&flood);
    assert_eq!(
        state.screen().cells().len(),
        cap,
        "round 1: cell count bounded"
    );
    assert!(
        state.snapshot().lines().is_empty(),
        "round 1: nothing printed"
    );

    // A second unterminated round must not grow anything.
    state.feed_bytes(&flood);
    assert_eq!(
        state.screen().cells().len(),
        cap,
        "round 2: cell count unchanged"
    );
    assert!(
        state.snapshot().lines().is_empty(),
        "round 2: nothing printed"
    );

    // Terminating with ST and printing recovers normally.
    state.feed_bytes(b"\x1b\\OK");
    assert_eq!(
        state.snapshot().lines(),
        ["OK".to_owned()],
        "recovers after terminator"
    );
    assert_eq!(
        state.screen().cells().len(),
        cap,
        "post-terminator: still bounded"
    );
}

/// `ESC` inside a DCS payload follows the string-terminator semantics shared
/// with OSC and the Williams reference state machine:
/// - `ESC \` completes ST and ends the string;
/// - a second `ESC` keeps waiting for a terminator (`ESC ESC \` still ends it);
/// - any other byte after `ESC` returns the parser to payload swallowing, so
///   that byte (and the `ESC`) never leak as printable text.
#[test]
fn esc_inside_control_string_payload_terminates_only_on_st() {
    // Plain ST terminates; the trailing Z renders.
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes(b"\x1bPdata\x1b\\Z");
    assert_eq!(state.snapshot().lines(), ["Z".to_owned()]);

    // ESC followed by a non-backslash byte resumes swallowing; neither the ESC
    // nor that byte print. The string then ends on a later ST.
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes(b"\x1bPda\x1bta\x1b\\Z");
    assert_eq!(state.snapshot().lines(), ["Z".to_owned()]);

    // Consecutive ESCs keep waiting for ST: `ESC ESC \` still terminates.
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes(b"\x1bPda\x1b\x1b\\Z");
    assert_eq!(state.snapshot().lines(), ["Z".to_owned()]);

    // A dangling ESC at the end of a payload holds the parser in escape-wait;
    // feeding `\\Z` afterwards completes ST and prints Z (split-friendly).
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    state.feed_bytes(b"\x1bPdata\x1b");
    assert!(
        state.snapshot().lines().is_empty(),
        "dangling ESC prints nothing"
    );
    state.feed_bytes(b"\\Z");
    assert_eq!(state.snapshot().lines(), ["Z".to_owned()]);
}

// ===== BUG-03: `<` and `=` must poison CSI sequences =====

/// `<` and `=` poison destructive CSI finals. `CSI < 2 M` is SGR-mouse-shaped
/// and must not execute as DeleteLines; `< 2 J` must not erase; `= 2 ; 4 r`
/// must not rewrite the scroll region; `< 1 m` must not apply SGR.
#[test]
fn lt_and_eq_poison_destructive_csi_finals() {
    for marker in ['<', '='] {
        // DeleteLines (`M`) is poisoned: a filled screen is untouched. This is
        // the exact coordinator reproduction (`CSI < 2 M` deleting a line).
        let mut state = TerminalState::new(3, 8).expect("valid terminal");
        state.feed_bytes(b"AAA\r\nBBB\r\nCCC");
        assert_eq!(
            state.snapshot().lines(),
            ["AAA".to_owned(), "BBB".to_owned(), "CCC".to_owned()],
            "marker {marker}: setup"
        );
        let before = state.snapshot().lines().to_vec();
        state.feed_bytes(format!("\x1b[{marker}2M").as_bytes());
        assert_eq!(
            state.snapshot().lines(),
            before.as_slice(),
            "marker {marker}: M must not delete a line"
        );

        // EraseInDisplay (`J`) is poisoned: the screen is untouched.
        let mut state = TerminalState::new(2, 8).expect("valid terminal");
        state.feed_bytes(b"AAA\r\nBBB");
        let before = state.snapshot().lines().to_vec();
        state.feed_bytes(format!("\x1b[{marker}2J").as_bytes());
        assert_eq!(
            state.snapshot().lines(),
            before.as_slice(),
            "marker {marker}: J must not erase"
        );

        // DECSTBM (`r`) is poisoned: the scroll region is unchanged.
        let mut state = TerminalState::new(5, 3).expect("valid terminal");
        let (top0, bottom0) = (state.scroll_region().top(), state.scroll_region().bottom());
        state.feed_bytes(format!("\x1b[{marker}2;4r").as_bytes());
        assert_eq!(
            (state.scroll_region().top(), state.scroll_region().bottom()),
            (top0, bottom0),
            "marker {marker}: r must not set scroll region"
        );

        // SGR (`m`) is poisoned: the pen is unchanged (bold is not applied).
        let mut state = TerminalState::new(1, 4).expect("valid terminal");
        let pen_before = *state.attributes();
        state.feed_bytes(format!("\x1b[{marker}1m").as_bytes());
        assert_eq!(
            *state.attributes(),
            pen_before,
            "marker {marker}: m must not apply SGR"
        );
        assert!(
            !state.attributes().is_bold(),
            "marker {marker}: bold not applied"
        );
    }
}

/// `?` and `>` keep their existing behavior: `?` drives DEC private modes and
/// `>` is a recognized-but-unsupported private marker that does nothing
/// destructive. Neither executes a standard final.
#[test]
fn question_and_gt_markers_behave_as_before() {
    // `?` still selects DEC private modes (DECCKM, alternate screen).
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[?1h");
    assert!(
        state.modes().is_application_cursor_key_mode(),
        "?1h sets DECCKM"
    );
    state.feed_bytes(b"\x1b[?1049h");
    assert!(
        state.modes().is_alternate_screen_active(),
        "?1049h switches alternate"
    );

    // `>` is a recognized private marker; its sequences are no-ops and never
    // execute the standard final. `CSI > 2 J` must NOT erase the screen.
    let mut state = TerminalState::new(2, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\nBBB");
    let before = state.snapshot().lines().to_vec();
    state.feed_bytes(b"\x1b[>2J");
    assert_eq!(
        state.snapshot().lines(),
        before.as_slice(),
        ">2J must not erase"
    );

    // `?` on a non-private-mode final poisons it too (unchanged): `CSI ? 2 J`
    // does not erase.
    let mut state = TerminalState::new(2, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\nBBB");
    let before = state.snapshot().lines().to_vec();
    state.feed_bytes(b"\x1b[?2J");
    assert_eq!(
        state.snapshot().lines(),
        before.as_slice(),
        "?2J must not erase"
    );
}
