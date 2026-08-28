//! Snapshot cost benchmark for issue #172.
//!
//! `TerminalEngine::snapshot()` is called on the per-frame render path
//! (`noren-app`'s `redraw`), so its cost must not scale with data the user
//! accumulates. This bench reports the two ends of that scaling:
//! an empty scrollback and a scrollback filled to the
//! [`MAX_SCROLLBACK_LINES`] cap.
//!
//! It is `harness = false` with no benchmarking dependency on purpose: it
//! reports numbers and asserts nothing, so no wall clock can become a
//! machine-dependent test gate here (#154/#159). The regression *guard* lives
//! in the test suite as a copied-work assertion (row sharing), not a timer.
//!
//! Run with `cargo bench -p noren-terminal`.

use std::hint::black_box;
use std::time::Instant;

use noren_terminal::{MAX_SCROLLBACK_LINES, TerminalState};

/// Build a terminal whose scrollback holds exactly `lines` non-blank rows.
///
/// The renderer-shaped 60x160 grid measures the snapshot at the size a real
/// frame uses. A `rows`-row grid absorbs `rows - 1` line feeds before the
/// first eviction, so the script emits `lines + rows - 1` labelled lines and
/// the assert pins the retained count exactly at `lines`.
fn filled_scrollback(lines: usize) -> TerminalState {
    let mut state = TerminalState::new(60, 160).expect("valid terminal size");
    let rows = usize::from(state.size().0);
    let total = lines + rows - 1;
    let mut script = String::with_capacity(total * 8);
    for index in 0..total {
        script.push_str(&format!("{index:05}\r\n"));
    }
    state.feed_bytes(script.as_bytes());
    assert_eq!(
        state.scrollback_len(),
        lines,
        "fixture must retain exactly `lines` rows"
    );
    state
}

/// Measure `snapshot()` over a fixed iteration count and report ns/iter.
fn bench_snapshot(label: &str, state: &TerminalState, iterations: u32) {
    // Warm up once so the first allocation does not dominate a short loop.
    black_box(state.snapshot());

    let start = Instant::now();
    for _ in 0..iterations {
        black_box(state.snapshot());
    }
    let elapsed = start.elapsed();

    let per_iter = elapsed.as_nanos() / u128::from(iterations);
    let per_iter_us = f64::from(u32::try_from(per_iter).unwrap_or(u32::MAX)) / 1000.0;
    println!("{label}: {per_iter_us:.3} µs/iter ({iterations} iterations, {elapsed:?} total)");
}

fn main() {
    // Iteration counts are fixed work counts, not deadlines: the bench reports
    // whatever time that work takes on whatever machine runs it.
    let empty = TerminalState::new(60, 160).expect("valid terminal size");
    bench_snapshot("snapshot_empty_scrollback", &empty, 2_000);

    let full = filled_scrollback(MAX_SCROLLBACK_LINES);
    bench_snapshot("snapshot_full_scrollback", &full, 200);
}
