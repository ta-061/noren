//! `Search` benchmarks — find-in-scrollback is the path a user feels when
//! they press Ctrl+F over a full history.
//!
//! The search suite scans every logical row (scrollback plus visible grid),
//! rebuilding each row's text from cells as it goes, so the cost scales with
//! retained history, not with the visible grid. These cases pin that trend at
//! the scrollback cap (`MAX_SCROLLBACK_LINES` = 10,000 rows):
//!
//! - `first_hit_near_end` — the worst realistic interactive search: the only
//!   hit sits on the last retained row, so `first()` scans the entire
//!   history before returning.
//! - `all_hits_spread` — `all()` with hits every 500 rows (21 matches).
//! - `count_no_hits` — full scan with zero matches: the pure per-row floor.
//!
//! Run with: `cargo bench -p noren-terminal --features bench-support search`.

use std::sync::OnceLock;

use criterion::{Criterion, criterion_group, criterion_main};
use noren_terminal::{CaseSensitivity, Search, TerminalSnapshot, TerminalState};

/// The app's renderer ceilings, mirrored as numbers (see `feed_bytes` bench).
const ROWS: u16 = 60;
const COLS: u16 = 160;

/// Enough lines to saturate the 10,000-row scrollback cap.
const LINE_COUNT: usize = 10_100;

const NEEDLE: &str = "needle-zq47";
const OTHER: &str = "haystack-9931";

/// Deterministic filler rows of ordinary text width.
fn filler_line(seed: usize) -> String {
    format!("{OTHER}-{seed:05} ordinary terminal line of middling length")
}

fn build_snapshot(hit_every: Option<usize>) -> TerminalSnapshot {
    let mut terminal = TerminalState::new(ROWS, COLS).expect("valid grid");
    for seed in 0..LINE_COUNT {
        let line = match hit_every {
            Some(interval) if seed % interval == 0 && seed > 0 => {
                format!("{} {NEEDLE}", filler_line(seed))
            }
            Some(_) | None => filler_line(seed),
        };
        terminal.feed_bytes(line.as_bytes());
        terminal.feed_bytes(b"\r\n");
    }
    // The near-end corpus puts its only hit on the final fed line.
    if hit_every.is_none() {
        terminal.feed_bytes(format!("last line {NEEDLE}\r\n").as_bytes());
    }
    terminal.snapshot()
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("search");

    let near_end = OnceLock::new();
    let near_end = near_end.get_or_init(|| build_snapshot(None));
    let spread = OnceLock::new();
    let spread = spread.get_or_init(|| build_snapshot(Some(500)));
    let miss_corpus = OnceLock::new();
    let miss_corpus = miss_corpus.get_or_init(|| {
        // Same volume, needle never present.
        let mut terminal = TerminalState::new(ROWS, COLS).expect("valid grid");
        for seed in 0..LINE_COUNT {
            terminal.feed_bytes(filler_line(seed).as_bytes());
            terminal.feed_bytes(b"\r\n");
        }
        terminal.snapshot()
    });

    group.bench_function("first_hit_near_end", |b| {
        b.iter(|| {
            std::hint::black_box(
                Search::new(near_end, NEEDLE, CaseSensitivity::InsensitiveAscii).first(),
            )
        })
    });

    group.bench_function("all_hits_spread", |b| {
        b.iter(|| {
            std::hint::black_box(
                Search::new(spread, NEEDLE, CaseSensitivity::InsensitiveAscii).all(),
            )
        })
    });

    group.bench_function("count_no_hits", |b| {
        b.iter(|| {
            std::hint::black_box(
                Search::new(miss_corpus, NEEDLE, CaseSensitivity::InsensitiveAscii).count(),
            )
        })
    });
    group.finish();
}

criterion_group!(benches, bench_search);
criterion_main!(benches);
