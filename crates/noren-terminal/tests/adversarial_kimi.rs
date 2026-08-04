//! Independent adversarial sweep for the Terminal Core state machine (Kimi
//! lane). This suite deliberately avoids re-running the attacks in
//! `tests/adversarial.rs` (GLM lane); see
//! `docs/coordination/reviews/terminal-adversarial-kimi.md` for the coverage
//! split. Focus areas GLM did not attack:
//!
//! - scrollback under feature *combinations* (alt-screen + DECSC/DECRC thrash,
//!   wide-character lines, resize storms between scroll batches)
//! - exhaustive `EscapeIntermediate` final-byte handling (the code added in
//!   `5e266d4`): every intermediate x every final, plus ESC/C0 abuse inside it
//! - determinism of a compound hostile script across *every* feed chunking
//! - wide-character pairs vs. tab stops, scroll storms, and edit ops
//! - `wrap_pending` combined with edit ops, scroll regions, and screen switches
//! - idempotence/round-trip properties checked by full snapshot equality
//! - unbounded-growth hunting in per-cell text (zero-width combining marks)

use noren_terminal::{
    AnsiColor, Cell, Color, CursorMove, MAX_SCROLLBACK_LINES, ScreenBuffer, TerminalError,
    TerminalState,
};

/// Public invariants that must hold after any sequence of public calls.
fn assert_invariants(state: &TerminalState, context: &str) {
    let (rows, cols) = state.size();
    assert!(rows > 0 && cols > 0, "{context}: non-zero size");
    assert_eq!(
        state.screen().cells().len(),
        usize::from(rows) * usize::from(cols),
        "{context}: cell count matches grid"
    );
    let cursor = state.cursor();
    assert!(cursor.row() < rows, "{context}: cursor row in bounds");
    assert!(cursor.column() < cols, "{context}: cursor column in bounds");
    let region = state.scroll_region();
    assert!(region.top() <= region.bottom(), "{context}: region ordered");
    assert!(region.bottom() < rows, "{context}: region within screen");
    assert!(
        state.scrollback_len() <= MAX_SCROLLBACK_LINES,
        "{context}: scrollback cap"
    );
}

/// The wide-character invariant, checked through the public cell view: every
/// width-2 lead is immediately followed by its continuation cell, and no
/// continuation cell exists without its lead. `apply()` only checks this with
/// `debug_assert!`; here it is asserted for real.
fn assert_wide_pairs_intact(screen: &ScreenBuffer, context: &str) {
    for row in 0..screen.rows() {
        let mut column = 0;
        while column < screen.cols() {
            let cell = screen.cell(row, column).expect("in-bounds cell");
            if cell.width() == 2 {
                assert!(
                    column + 1 < screen.cols(),
                    "{context}: wide lead on the last column at r{row}c{column}"
                );
                assert!(
                    screen
                        .cell(row, column + 1)
                        .is_some_and(Cell::is_continuation),
                    "{context}: wide lead without continuation at r{row}c{column}"
                );
                column += 2;
            } else {
                assert!(
                    !cell.is_continuation(),
                    "{context}: orphaned continuation at r{row}c{column}"
                );
                column += 1;
            }
        }
    }
}

// ===== Scrollback under feature combinations (GLM never fed scrollback) =====

#[test]
fn scrollback_is_byte_identical_after_alt_screen_decsc_decrc_thrash() {
    let mut state = TerminalState::new(3, 6).expect("valid terminal");
    // Accumulate primary scrollback with recognizable content.
    state.feed_bytes(b"HIST01\r\nHIST02\r\nHIST03\r\nLIVE01");
    let baseline = state.snapshot();

    for iteration in 0..50_u16 {
        state.feed_bytes(b"\x1b[?1049h");
        // Thrash the alternate screen: scrolling output, DECSC/DECRC around a
        // cursor move, and a scroll-region-local scroll.
        state.feed_bytes(b"aaaaaa\r\nbbbbbb\r\ncccccc\r\n");
        state.feed_bytes(b"\x1b[2;2H\x1b7\x1b[1;1H\x1b8");
        state.feed_bytes(b"\x1b[2;3r\x1b[3;1H\x1bD\x1b[r");
        state.feed_bytes(b"\x1b[?1049l");
        assert!(
            !state.modes().is_alternate_screen_active(),
            "iteration {iteration}: back on primary"
        );
    }

    let after = state.snapshot();
    assert_eq!(
        after.scrollback(),
        baseline.scrollback(),
        "scrollback cells drifted across alternate-screen thrash"
    );
    assert_eq!(
        after.lines(),
        baseline.lines(),
        "visible primary content drifted across alternate-screen thrash"
    );
    assert_invariants(&state, "alt-screen scrollback thrash");
}

#[test]
fn scrollback_stays_capped_with_wide_character_lines_and_keeps_pairs() {
    // One-row grid: every CRLF evicts the just-printed line of wide chars.
    let mut state = TerminalState::new(1, 8).expect("valid terminal");
    let total = MAX_SCROLLBACK_LINES + 100;
    let mut script = String::new();
    for _ in 0..total {
        script.push_str("日日日日\r\n");
    }
    state.feed_bytes(script.as_bytes());

    assert_eq!(state.scrollback_len(), MAX_SCROLLBACK_LINES);
    let snapshot = state.snapshot();
    let newest = &snapshot.scrollback()[MAX_SCROLLBACK_LINES - 1];
    // The retained rows kept full cell fidelity: 4 leads + 4 continuations.
    assert_eq!(newest.len(), 8);
    assert_eq!(newest[0].text(), "日");
    assert_eq!(newest[0].width(), 2);
    assert!(newest[1].is_continuation());
    assert_eq!(
        snapshot.scrollback_lines().last().map(String::as_str),
        Some("日日日日")
    );
}

#[test]
fn scrollback_of_mixed_widths_survives_a_resize_storm_between_batches() {
    let mut state = TerminalState::new(2, 8).expect("valid terminal");
    let sizes = [(3u16, 8u16), (2, 3), (4, 11), (1, 2), (2, 8)];
    for (batch, &(rows, cols)) in sizes.iter().enumerate() {
        state.resize(rows, cols).expect("valid storm resize");
        for line in 0..40_u32 {
            state.feed_bytes(format!("B{batch}L{line:02}\r\n").as_bytes());
        }
        assert_invariants(&state, "resize-storm scrollback batch");
    }

    // The append-only order is preserved up to line truncation: retained
    // rows were captured at different historical widths, so labels may be
    // cut short, but the newest retained row is exactly the second-to-last
    // line printed in the final batch.
    let lines = state.snapshot().scrollback_lines();
    assert!(lines.len() <= MAX_SCROLLBACK_LINES);
    assert_eq!(lines.last().map(String::as_str), Some("B4L38"));
    // Rows retained their original widths (no reflow): witnessed widths are
    // drawn only from the historical column counts.
    for row in state.snapshot().scrollback() {
        assert!(
            [8, 3, 11, 2].contains(&row.len()),
            "unexpected retained row width {}",
            row.len()
        );
    }
}

// ===== EscapeIntermediate: the state added in 5e266d4, attacked exhaustively =====

#[test]
fn every_escape_intermediate_final_byte_is_consumed_without_leaking() {
    // Every intermediate byte (0x20..=0x2f) x every possible final byte
    // (0x30..=0x7e): the final must terminate the sequence and vanish; only
    // the trailing X may print, at column 1.
    for intermediate in 0x20..=0x2f_u8 {
        for final_byte in 0x30..=0x7e_u8 {
            let mut state = TerminalState::new(1, 4).expect("valid terminal");
            state.feed_bytes(&[0x1b, intermediate, final_byte, b'X']);
            assert_eq!(
                state.snapshot().lines(),
                ["X".to_owned()],
                "ESC {intermediate:#04x} {final_byte:#04x} leaked"
            );
            assert_eq!(
                (state.cursor().row(), state.cursor().column()),
                (0, 1),
                "ESC {intermediate:#04x} {final_byte:#04x} moved the cursor"
            );
        }
    }
}

#[test]
fn stacked_intermediates_and_esc_restarts_never_leak() {
    // Multiple intermediates before one final: everything is swallowed.
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b()# \x1b(BX");
    // The second ESC restarts; its `(` re-enters EscapeIntermediate; `B` is
    // the consumed final. Only X prints.
    assert_eq!(state.snapshot().lines(), ["X".to_owned()]);

    // DEL and NUL inside EscapeIntermediate do not terminate it; the eventual
    // final byte is still consumed.
    let mut state = TerminalState::new(1, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b(\x7f\x00\x7f\x00BY");
    assert_eq!(state.snapshot().lines(), ["Y".to_owned()]);
    assert_invariants(&state, "stacked intermediates");
}

#[test]
fn escape_aborting_csi_then_intermediate_sequence_keeps_later_csi_working() {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    // Partial CSI 2;3... aborted by ESC; an SCS-like sequence follows; then a
    // real CUP must still land exactly.
    state.feed_bytes(b"\x1b[2;3\x1b(B\x1b[2;2HZ");
    assert_eq!(state.screen().cell(1, 1).map(Cell::text), Some("Z"));
    assert_eq!((state.cursor().row(), state.cursor().column()), (1, 2));
    assert_invariants(&state, "esc-aborted csi then intermediate");
}

#[test]
fn mid_osc_resize_does_not_corrupt_the_string_state() {
    let mut state = TerminalState::new(2, 6).expect("valid terminal");
    state.feed_bytes(b"\x1b]0;part");
    state.resize(3, 9).expect("valid resize mid-OSC");
    state.feed_bytes(b"ial\x07Z");
    // The OSC swallowed everything up to BEL despite the resize; Z prints on
    // the resized grid.
    assert_eq!(state.screen().cell(0, 0).map(Cell::text), Some("Z"));
    assert_eq!(state.size(), (3, 9));
    assert_invariants(&state, "mid-OSC resize");
}

// ===== Determinism: compound hostile script across EVERY feed chunking =====

#[test]
fn compound_hostile_script_is_deterministic_across_all_chunkings() {
    // Individually valid but collectively awkward: regions, alt screen, SGR,
    // OSC, tabs at the edge, wide chars, combining marks, invalid UTF-8,
    // DECSC/DECRC, aborted sequences.
    let script: &[u8] = b"\x1b[31;1mab\xe6\x97\xa5\x1b[2;4r\x1b[4;1H\x1bD\tZ\
        \x1b]8;;payload\x07\x1b[?1049hW\xe6\x97\xa5\x1b[?1049l\
        \x1b7\x1b[1;1HQ\x1b8e\xcc\x81\xff\xc0\xaf\x1b[9\x1b[2G\\\x1b[r\x1b[mR";

    let mut whole = TerminalState::new(5, 9).expect("valid terminal");
    whole.feed_bytes(script);
    let reference = whole.snapshot();

    for chunk in 1..=script.len() {
        let mut split = TerminalState::new(5, 9).expect("valid terminal");
        for piece in script.chunks(chunk) {
            split.feed_bytes(piece);
        }
        assert_eq!(
            split.snapshot(),
            reference,
            "chunk size {chunk} diverged from the whole-feed result"
        );
    }
    assert_invariants(&whole, "compound script");
    assert_wide_pairs_intact(whole.screen(), "compound script");
}

// ===== Wide characters vs. tab stops, scroll storms, and edit ops =====

#[test]
fn tab_landing_on_a_continuation_cell_snaps_forward_off_the_pair() {
    let mut state = TerminalState::new(2, 10).expect("valid terminal");
    // 日 leads at 0,2,4; 'a' at 6; 日 lead at 7 with continuation at 8.
    state.feed_bytes("日日日a日".as_bytes());
    assert_eq!((state.cursor().row(), state.cursor().column()), (0, 9));

    state.feed_bytes(b"\r\t");
    // Tab stop 8 is the continuation half: the cursor must move past the pair
    // to column 9, never rest on the continuation.
    assert_eq!(state.cursor().column(), 9);
    assert!(
        !state
            .screen()
            .cell(0, state.cursor().column())
            .is_some_and(Cell::is_continuation)
    );

    // Printing there fills the last column and arms the wrap; pairs stay whole.
    state.feed_bytes(b"X");
    assert!(state.is_wrap_pending());
    assert_wide_pairs_intact(state.screen(), "tab onto continuation");
}

#[test]
fn scroll_and_edit_storm_never_splits_a_wide_pair() {
    let mut state = TerminalState::new(6, 10).expect("valid terminal");
    state.feed_bytes(
        "\x1b[1;1H日a日b\x1b[2;1Hc日d日\x1b[3;1H日日日日日\
         \x1b[4;1He日f日g\x1b[5;1Hh日日i日\x1b[6;1H日日日日日"
            .as_bytes(),
    );
    state.feed_bytes(b"\x1b[2;5r\x1b[1;1H");

    let ops: &[&[u8]] = &[
        b"\x1b[2S", // scroll region up
        b"\x1b[T",  // scroll region down
        b"\x1bD",   // index
        b"\x1bM",   // reverse index
        b"\x1b[2L", // insert lines
        b"\x1b[M",  // delete lines
        b"\x1b[3@", // insert characters
        b"\x1b[2P", // delete characters
        b"\x1b[4X", // erase characters
        b"\x1b[1K", // erase line to beginning
        b"\x1b[2K", // erase whole line
        b"\x1b[2C", // move within the row
        b"\x1b[B",  // move down a row
    ];
    for round in 0..40 {
        for op in ops {
            state.feed_bytes(op);
            assert_wide_pairs_intact(state.screen(), &format!("round {round} op {op:?}"));
        }
        assert_invariants(&state, &format!("storm round {round}"));
    }
}

#[test]
fn erase_in_display_at_every_wide_boundary_leaves_no_dangling_half() {
    // ED0/ED1/ED2 with the cursor on every column of a row of wide chars:
    // an erase boundary that cuts a pair must blank the orphaned half.
    for mode in 0..=2_u8 {
        for column in 1..=8_u16 {
            let mut state = TerminalState::new(3, 8).expect("valid terminal");
            state.feed_bytes("日日日日\x1b[2;1H日日日日".as_bytes());
            state.feed_bytes(format!("\x1b[1;{column}H\x1b[{mode}J").as_bytes());
            assert_wide_pairs_intact(state.screen(), &format!("ED{mode} at column {column}"));
        }
    }
}

// ===== wrap_pending combined with edit ops, regions, and screen switches =====

#[test]
fn edit_and_erase_ops_cancel_a_pending_wrap_before_the_next_print() {
    // Each op clears wrap_pending; the following printable must land at the
    // cursor WITHOUT triggering the deferred wrap/scroll.
    let ops: &[&[u8]] = &[b"\x1b[K", b"\x1b[1P", b"\x1b[1X", b"\x1b[1@", b"\x1b[J"];
    for op in ops {
        let mut state = TerminalState::new(2, 4).expect("valid terminal");
        state.feed_bytes(b"abcd");
        assert!(state.is_wrap_pending(), "{op:?}: precondition");
        state.feed_bytes(op);
        assert!(!state.is_wrap_pending(), "{op:?}: wrap not cancelled");
        state.feed_bytes(b"Z");
        assert_eq!(
            (state.cursor().row(), state.cursor().column()),
            (0, 3),
            "{op:?}: print after the op wrapped unexpectedly"
        );
        assert!(state.is_wrap_pending(), "{op:?}: wrap re-armed at the edge");
        assert_invariants(&state, "edit op cancels wrap");
    }
}

#[test]
fn alternate_screen_round_trip_restores_content_but_clears_pending_wrap() {
    let mut state = TerminalState::new(3, 4).expect("valid terminal");
    state.feed_bytes(b"abcd"); // fills row 0, wrap pending on primary
    assert!(state.is_wrap_pending());

    state.feed_bytes(b"\x1b[?1049h");
    assert!(
        !state.is_wrap_pending(),
        "alternate starts without a pending wrap"
    );
    state.feed_bytes(b"WXYZ"); // wrap pending on the alternate instead
    assert!(state.is_wrap_pending());

    state.feed_bytes(b"\x1b[?1049l");
    // Leaving runs DECRC on the restored primary, and this core's
    // restore_cursor unconditionally clears wrap_pending. The cursor position
    // and cells round-trip exactly; the deferred wrap does NOT. Whether xterm
    // preserves do_wrap across mode 1049 is a compat question, not a crash:
    // pinned here as the current behavior (see the review report).
    assert!(!state.is_wrap_pending());
    assert_eq!((state.cursor().row(), state.cursor().column()), (0, 3));
    assert_eq!(state.screen().cell(0, 3).map(Cell::text), Some("d"));
    state.feed_bytes(b"Q");
    // With the wrap cleared, Q overwrites the last column instead of wrapping.
    assert_eq!(state.screen().cell(0, 3).map(Cell::text), Some("Q"));
    assert!(state.is_wrap_pending());
    assert_invariants(&state, "wrap pending screen round trip");
}

#[test]
fn wrap_at_a_region_bottom_scrolls_only_the_region() {
    let mut state = TerminalState::new(5, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\x1b[2;1HBBB\x1b[3;1HCCC\x1b[4;1HDDD\x1b[5;1HEEE");
    // Region rows 2..=4 (1-based); park on the region's bottom row and fill it
    // to the right edge so the next printable wraps and indexes at the bottom.
    state.feed_bytes(b"\x1b[3;4r\x1b[4;1Hddd");
    assert!(state.is_wrap_pending());

    state.feed_bytes(b"Z");
    // The wrap moved to region-bottom column 0 and scrolled the region (rows
    // 2..=3 zero-based) up by one: CCC rotated out of the region, ddd moved
    // up. Rows outside the region (AAA, BBB, EEE) are untouched.
    assert_eq!(state.snapshot().lines(), ["AAA", "BBB", "ddd", "Z", "EEE"]);
    assert_eq!((state.cursor().row(), state.cursor().column()), (3, 1));
    // The region top is not row 0, so nothing entered scrollback.
    assert_eq!(state.scrollback_len(), 0);
    assert_invariants(&state, "wrap at region bottom");
}

// ===== Idempotence and round-trip properties via full snapshot equality =====

#[test]
fn alternate_screen_round_trip_restores_the_exact_snapshot() {
    let mut state = TerminalState::new(3, 6).expect("valid terminal");
    state.feed_bytes(b"\x1b[1;31mRED\x1b[2;4r\x1b[3;2H\x1b7mid");
    let baseline = state.snapshot();

    for round in 0..2 {
        state.feed_bytes(b"\x1b[?1049h");
        state.feed_bytes(b"junk junk junk\x1b[2J\x1b[3;3H!!");
        state.feed_bytes(b"\x1b[?1049l");
        assert_eq!(
            state.snapshot(),
            baseline,
            "round trip {round} did not restore the exact primary state"
        );
    }
}

#[test]
fn double_enter_single_leave_restores_the_exact_snapshot() {
    let mut state = TerminalState::new(2, 5).expect("valid terminal");
    state.feed_bytes(b"prim\x1b[2;2H");
    let baseline = state.snapshot();

    state.feed_bytes(b"\x1b[?1049h\x1b[?1049h"); // second enter is a no-op
    assert!(state.modes().is_alternate_screen_active());
    state.feed_bytes(b"XXXXX\r\nYYYYY");
    state.feed_bytes(b"\x1b[?1049l"); // one leave must suffice

    assert!(!state.modes().is_alternate_screen_active());
    assert_eq!(state.snapshot(), baseline);
}

#[test]
fn rejected_operations_leave_the_full_snapshot_unchanged() {
    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    state.feed_bytes(b"\x1b[33;4mgreen\x1b[2;2H\x1b7");
    let baseline = state.snapshot();

    // Rejected resizes: over the cell cap, and zero dimensions.
    assert_eq!(state.resize(1025, 1024), Err(TerminalError::ScreenTooLarge));
    assert_eq!(state.resize(0, 5), Err(TerminalError::InvalidSize));
    assert_eq!(state.resize(3, 0), Err(TerminalError::InvalidSize));
    assert_eq!(state.snapshot(), baseline, "rejected resize mutated state");

    // Rejected scroll regions, via CSI and via the public API.
    state.feed_bytes(b"\x1b[4;2r"); // inverted after clamping -> rejected
    assert_eq!(state.snapshot(), baseline, "rejected DECSTBM mutated state");
    assert_eq!(
        state.set_scroll_region(2, 1),
        Err(TerminalError::InvalidScrollRegion)
    );
    assert_eq!(
        state.snapshot(),
        baseline,
        "rejected public region mutated state"
    );
}

#[test]
fn sgr_pen_is_terminal_global_across_screen_switches_and_resets_cleanly() {
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"\x1b[1;31m");
    let armed = *state.attributes();
    assert!(armed.is_bold());
    assert_eq!(armed.foreground(), Color::ansi(AnsiColor::Red));

    // The pen follows into the alternate screen: cells written there carry it.
    state.feed_bytes(b"\x1b[?1049hZ");
    let alt_cell = state.screen().cell(0, 0).expect("alt cell");
    assert_eq!(*alt_cell.attributes(), armed);

    // Reset on the alternate screen; the default pen persists after leaving.
    state.feed_bytes(b"\x1b[m\x1b[?1049lQ");
    let primary_cell = state.screen().cell(0, 0).expect("primary cell");
    assert_eq!(*state.attributes(), Default::default());
    assert_eq!(*primary_cell.attributes(), Default::default());
}

// ===== Public API surface the suites never call =====

#[test]
fn public_api_edge_inputs_are_rejected_or_clamped() {
    assert_eq!(
        TerminalState::new(0, 5).unwrap_err(),
        TerminalError::InvalidSize
    );
    assert_eq!(
        TerminalState::new(5, 0).unwrap_err(),
        TerminalError::InvalidSize
    );

    let mut state = TerminalState::new(3, 5).expect("valid terminal");
    // Out-of-bounds cell access yields None, never a panic.
    assert!(state.screen().cell(3, 0).is_none());
    assert!(state.screen().cell(0, 5).is_none());
    assert!(state.screen().cell(u16::MAX, u16::MAX).is_none());

    // Saturating public cursor moves clamp to the grid.
    state.move_cursor(CursorMove::Down(u16::MAX));
    state.move_cursor(CursorMove::Right(u16::MAX));
    assert_eq!((state.cursor().row(), state.cursor().column()), (2, 4));
    state.move_cursor(CursorMove::Up(u16::MAX));
    state.move_cursor(CursorMove::Left(u16::MAX));
    assert_eq!((state.cursor().row(), state.cursor().column()), (0, 0));

    // Public save/restore round trip returns to the saved position exactly.
    state.move_cursor(CursorMove::To { row: 1, column: 3 });
    state.save_cursor();
    state.move_cursor(CursorMove::To { row: 2, column: 4 });
    state.restore_cursor();
    assert_eq!((state.cursor().row(), state.cursor().column()), (1, 3));

    // A fresh terminal exposes empty scrollback through the snapshot.
    assert!(state.snapshot().scrollback().is_empty());
    assert!(state.snapshot().scrollback_lines().is_empty());
    assert_invariants(&state, "public api edges");
}

// ===== Unbounded growth hunting: per-cell text has no size bound =====

/// A hostile program can append an unlimited number of zero-width combining
/// characters to a single cell: `attach_zero_width` pushes onto the cell's
/// `String` with no cap, so memory grows linearly with input volume on a grid
/// whose cell COUNT never changes. This violates the bounded-memory contract
/// documented on `MAX_SCROLLBACK_LINES`/`MAX_SCREEN_CELLS` (the bounds cover
/// cells, not the text inside them). The wrap-pending path (`attach` targets
/// the last column while a wrap is armed) has the same hole.
#[test]
#[ignore = "reproduces KBUG-01"]
fn combining_marks_grow_a_single_cell_without_bound() {
    const REASONABLE_GRAPHEME_CAP: usize = 32;

    let mut state = TerminalState::new(2, 8).expect("valid terminal");
    state.feed_bytes(b"a");
    let marks = "\u{0301}".repeat(200_000);
    state.feed_bytes(marks.as_bytes());
    let text_len = state
        .screen()
        .cell(0, 0)
        .map_or(0, |cell| cell.text().len());
    assert!(
        text_len <= REASONABLE_GRAPHEME_CAP,
        "KBUG-01: one cell holds {text_len} bytes after a combining-mark flood"
    );

    let mut state = TerminalState::new(2, 2).expect("valid terminal");
    state.feed_bytes(b"ab"); // arm wrap_pending on the last column
    state.feed_bytes(marks.as_bytes());
    let text_len = state
        .screen()
        .cell(0, 1)
        .map_or(0, |cell| cell.text().len());
    assert!(
        text_len <= REASONABLE_GRAPHEME_CAP,
        "KBUG-01: wrap-pending cell holds {text_len} bytes after a combining-mark flood"
    );
}
