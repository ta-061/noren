//! Scrollback retention: lines that scroll off the top of the primary screen
//! are retained in a bounded buffer and exposed renderer-independently.
//!
//! These tests pin the design constraints from the terminal-core-foundation
//! roadmap: bounded memory, primary-screen-only contribution, no contribution
//! from non-screen-aligned scroll regions, deliberate (no-reflow) resize
//! behavior, and hostile-output safety.
//!
//! Lines are separated with CRLF (`\r\n`): in this core, LF moves the cursor
//! down (and scrolls) without returning to column 0, exactly like a real
//! terminal, so CR is needed to start each label at the left margin.

use std::sync::Arc;

use noren_terminal::{Cell, MAX_SCROLLBACK_LINES, TerminalState};

/// Text rendering of retained scrollback rows, oldest first.
fn scrollback_lines(state: &TerminalState) -> Vec<String> {
    state.snapshot().scrollback_lines()
}

/// Width (cell count) of each retained scrollback row, in order.
fn scrollback_widths(state: &TerminalState) -> Vec<usize> {
    state
        .snapshot()
        .scrollback()
        .iter()
        .map(|row| row.len())
        .collect()
}

#[test]
fn lines_scrolled_off_the_top_are_retained_in_order() {
    // 3-row grid, full-screen margins. Each CRLF past the third evicts the top
    // row into scrollback, oldest first.
    let mut state = TerminalState::new(3, 4).expect("valid terminal");
    state.feed_bytes(b"AAAA\r\nBBBB\r\nCCCC\r\nDDDD\r\n");

    // After CCCC's line feed the screen scrolled once (AAAA evicted); after
    // DDDD's line feed it scrolled again (BBBB evicted). The visible screen
    // holds the two freshest rows; scrollback holds the evicted rows in order.
    assert_eq!(scrollback_lines(&state), ["AAAA", "BBBB"]);
    assert_eq!(state.snapshot().lines(), ["CCCC", "DDDD"]);
    assert_eq!(state.scrollback_len(), 2);
    assert_eq!(state.snapshot().scrollback().len(), 2);

    // The retained rows carry full cell fidelity, not just text.
    let snapshot = state.snapshot();
    let first_row = &snapshot.scrollback()[0];
    assert_eq!(first_row.len(), 4);
    assert_eq!(first_row[0].text(), "A");
}

#[test]
fn the_bound_holds_and_evicts_the_oldest_lines() {
    // One-row grid: every CRLF evicts the just-printed row. Emit well past the
    // cap and assert the count clamps and the OLDEST (earliest emitted) are the
    // ones dropped.
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    let overflow = 200;
    let total = MAX_SCROLLBACK_LINES + overflow;

    let mut script = String::new();
    for index in 0..total {
        script.push_str(&format!("{index:05}\r\n"));
    }
    state.feed_bytes(script.as_bytes());

    // The count is clamped exactly at the cap.
    assert_eq!(state.scrollback_len(), MAX_SCROLLBACK_LINES);
    assert_eq!(state.snapshot().scrollback().len(), MAX_SCROLLBACK_LINES);

    // The oldest `overflow` lines were evicted; the retained window is the most
    // recent `cap` labels: [overflow .. total).
    let lines = scrollback_lines(&state);
    assert_eq!(lines.len(), MAX_SCROLLBACK_LINES);
    assert_eq!(lines.first().map(String::as_str), Some("00200"));
    assert_eq!(
        lines.last().map(String::as_str),
        Some(format!("{:05}", total - 1).as_str())
    );
    // Monotonic increase proves in-order retention with no duplication/gaps.
    let mut previous = 0u32;
    for line in &lines {
        let value = line.parse::<u32>().expect("numeric label");
        assert!(value >= previous, "retention order broke at {line}");
        previous = value;
    }
}

#[test]
fn alternate_screen_contributes_nothing_and_preserves_primary_scrollback() {
    // Accumulate primary scrollback first.
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"A\r\nB\r\nC\r\n");
    assert_eq!(scrollback_lines(&state), ["A", "B"]);

    // Enter the alternate screen and thrash it with scrolling output. The final
    // line has no CRLF so both Z and W stay visible on the 2-row grid.
    state.feed_bytes(b"\x1b[?1049h");
    assert!(state.modes().is_alternate_screen_active());
    let alternate_scroll_before = state.scrollback_len();
    state.feed_bytes(b"X\r\nY\r\nZ\r\nW");
    assert_eq!(state.snapshot().lines(), ["Z", "W"]);
    // The alternate screen added nothing to scrollback.
    assert_eq!(state.scrollback_len(), alternate_scroll_before);
    assert_eq!(scrollback_lines(&state), ["A", "B"]);

    // Leaving the alternate screen leaves primary scrollback intact.
    state.feed_bytes(b"\x1b[?1049l");
    assert!(!state.modes().is_alternate_screen_active());
    assert_eq!(scrollback_lines(&state), ["A", "B"]);
    assert_eq!(state.snapshot().lines(), ["C"]);
    assert_eq!(state.scrollback_len(), 2);

    // And primary-screen scrolling after returning resumes appending.
    state.feed_bytes(b"D\r\n");
    assert_eq!(scrollback_lines(&state), ["A", "B", "C"]);
}

#[test]
fn scroll_region_not_at_the_top_of_the_screen_does_not_push_to_scrollback() {
    // 5-row grid. Set margins to rows 3..5 (1-based) so scrolling inside the
    // region never reaches the top of the screen.
    let mut state = TerminalState::new(5, 3).expect("valid terminal");
    state.feed_bytes(b"AAA\x1b[2;1HBBB\x1b[3;1HCCC\x1b[4;1HDDD\x1b[5;1HEEE");
    state.feed_bytes(b"\x1b[3;5r\x1b[5;1H\n");

    // Rows above the margin (AAA, BBB) are untouched; the region scrolled
    // internally but nothing left the visible screen, so scrollback is empty.
    assert_eq!(state.scrollback_len(), 0);
    assert!(state.snapshot().scrollback().is_empty());
    assert_eq!(state.snapshot().lines(), ["AAA", "BBB", "DDD", "EEE"]);

    // A full-screen scroll from the same state DOES push, proving the gate is
    // region placement, not the scroll operation itself.
    state.feed_bytes(b"\x1b[r\x1b[5;1H\n");
    assert_eq!(scrollback_lines(&state), ["AAA"]);
}

#[test]
fn resize_does_not_reflow_or_corrupt_retained_lines() {
    // Known limitation, asserted explicitly: retained rows keep the column
    // width they had when they scrolled off; resize neither reflows nor
    // truncates them, and never panics.
    let mut state = TerminalState::new(2, 4).expect("valid terminal");
    state.feed_bytes(b"AAAA\r\nBBBB\r\nCCCC\r\n");
    assert_eq!(scrollback_lines(&state), ["AAAA", "BBBB"]);
    assert_eq!(scrollback_widths(&state), [4, 4]);

    // Grow: retained rows stay width-4 (not padded to the new width).
    state.resize(4, 6).expect("grow");
    assert_eq!(scrollback_lines(&state), ["AAAA", "BBBB"]);
    assert_eq!(scrollback_widths(&state), [4, 4]);

    // Shrink below the original width: retained rows stay width-4 (not
    // truncated to the narrower grid).
    state.resize(1, 2).expect("shrink");
    assert_eq!(scrollback_lines(&state), ["AAAA", "BBBB"]);
    assert_eq!(scrollback_widths(&state), [4, 4]);
    assert_eq!(state.scrollback_len(), 2);

    // Cell content survives the resize storm verbatim.
    let snapshot = state.snapshot();
    let row0 = &snapshot.scrollback()[0];
    assert_eq!(row0.iter().map(Cell::text).collect::<String>(), "AAAA");
}

/// Fill `state`'s scrollback to exactly the hard cap with labelled lines.
///
/// Shared fixture for the per-frame-cost guards below: a 1-row grid makes
/// every CRLF evict one labelled row, so the retained history is exactly the
/// last `MAX_SCROLLBACK_LINES` labels, oldest first.
fn capped_scrollback_state() -> TerminalState {
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    let mut script = String::with_capacity(MAX_SCROLLBACK_LINES * 8);
    for index in 0..MAX_SCROLLBACK_LINES {
        script.push_str(&format!("{index:05}\r\n"));
    }
    state.feed_bytes(script.as_bytes());
    assert_eq!(state.scrollback_len(), MAX_SCROLLBACK_LINES);
    state
}

/// Issue #172 regression guard: `snapshot()` must copy **zero** scrollback
/// cells, at any history depth.
///
/// This is the copied-work assertion in the style #158 established for #137:
/// the guard pins the amount of copying (a count), never the wall clock a
/// copy takes, so it holds on every machine and in every build profile.
/// Scrollback rows are immutable from the moment they scroll off, so the
/// snapshot shares each row by `Arc` handle instead of cloning `cols *
/// size_of::<Cell>()` bytes per row. Two snapshots of one state therefore
/// reference the same row allocation iff the rows were shared: a deep copy
/// (the pre-fix `from_state`, or any regression back to it) produces distinct
/// allocations and fails the pointer equality below — at the full 10_000-row
/// cap, the exact history depth where the 150x cost showed up.
#[test]
fn snapshot_copies_no_scrollback_cells_at_full_history() {
    let state = capped_scrollback_state();

    let first = state.snapshot();
    let second = state.snapshot();
    assert_eq!(first.scrollback().len(), MAX_SCROLLBACK_LINES);

    let deep_copied: Vec<usize> = first
        .scrollback()
        .iter()
        .zip(second.scrollback())
        .filter(|(a, b)| !Arc::ptr_eq(a, b))
        .map(|(a, _)| a.len())
        .collect();
    assert!(
        deep_copied.is_empty(),
        "snapshot deep-copied {} scrollback rows ({} cells) instead of sharing them",
        deep_copied.len(),
        deep_copied.iter().sum::<usize>(),
    );

    // Sharing must not change what the consumer reads: contents, order, and
    // the text view still match the fixture exactly.
    assert_eq!(
        first.scrollback_lines().first().map(String::as_str),
        Some("00000")
    );
    let last_label = format!("{:05}", MAX_SCROLLBACK_LINES - 1);
    assert_eq!(
        first.scrollback_lines().last().map(String::as_str),
        Some(last_label.as_str())
    );
}

/// Snapshot isolation under sharing (issue #172): sharing a row with the
/// state is safe only because retained rows are immutable. This is the
/// staleness counter-test — it fails if a future change ever lets a state
/// mutation reach rows a live snapshot already handed out, which is the
/// failure mode a cache-without-invalidation would have.
#[test]
fn snapshot_scrollback_rows_are_stable_after_further_scrolling() {
    let state = capped_scrollback_state();
    let frozen = state.snapshot();
    let frozen_lines = frozen.scrollback_lines();

    // Scroll a full cap of new lines past: every row the snapshot holds is
    // evicted from the live state, and the visible screen is overwritten.
    let mut state = state;
    let mut script = String::with_capacity(MAX_SCROLLBACK_LINES * 8);
    for index in 0..MAX_SCROLLBACK_LINES {
        script.push_str(&format!("N{index:04}\r\n"));
    }
    state.feed_bytes(script.as_bytes());

    // The live history moved on entirely...
    assert_eq!(state.scrollback_len(), MAX_SCROLLBACK_LINES);
    assert_eq!(
        state
            .snapshot()
            .scrollback_lines()
            .first()
            .map(String::as_str),
        Some("N0000")
    );
    // ...while the earlier snapshot still holds exactly what it captured:
    // same length, same contents, unchanged.
    assert_eq!(frozen.scrollback().len(), MAX_SCROLLBACK_LINES);
    assert_eq!(frozen.scrollback_lines(), frozen_lines);
    assert_eq!(
        frozen.scrollback_lines().first().map(String::as_str),
        Some("00000")
    );
}

#[test]
fn hostile_output_is_bounded_and_evicts_correctly() {
    // 100k lines into a one-row grid: memory must stay bounded at the cap and
    // the retained window must be exactly the most recent `cap` lines.
    let mut state = TerminalState::new(1, 6).expect("valid terminal");
    let total = 100_000;

    let mut script = String::with_capacity(total * 7);
    for index in 0..total {
        script.push_str(&format!("{index:05}\r\n"));
    }
    state.feed_bytes(script.as_bytes());

    // Bounded: scrollback is exactly at the cap, never above.
    assert_eq!(state.scrollback_len(), MAX_SCROLLBACK_LINES);
    assert!(state.scrollback_len() <= MAX_SCROLLBACK_LINES);

    // Correct eviction: oldest retained is the (total - cap)-th line, newest is
    // the last line emitted.
    let lines = scrollback_lines(&state);
    assert_eq!(lines.first().map(String::as_str), Some("90000"));
    assert_eq!(lines.last().map(String::as_str), Some("99999"));

    // The visible grid itself never grew beyond rows*cols.
    let (rows, cols) = state.size();
    assert_eq!(
        state.screen().cells().len(),
        usize::from(rows) * usize::from(cols)
    );
}
