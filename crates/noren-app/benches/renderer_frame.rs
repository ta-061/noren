//! Renderer frame-preparation benchmarks: the CPU work that runs every frame
//! before the GPU is touched, plus the snapshot the main loop builds for it.
//!
//! Two things run per presented frame in `main.rs`:
//!
//! 1. `TerminalEngine::snapshot()` — and `TerminalSnapshot::from_state` deep
//!    -clones the visible screen *and the entire retained scrollback*
//!    (up to `MAX_SCROLLBACK_LINES` = 10,000 rows of cells), so its cost is
//!    a per-frame constant that grows with history, independent of what
//!    changed. Benchmarked empty vs full scrollback to expose that growth.
//! 2. `Renderer::render`'s CPU half — `Target::new` plus
//!    `glyph_vertices_for`, measured here through the **same shipped
//!    source**, re-included via `#[path]` exactly like the frame oracle's
//!    `renderer_capture.rs` does (no parallel copy that could drift; only
//!    the window-bound `Renderer` is unused, hence `allow(dead_code)`).
//!
//! Cases:
//!
//! - `renderer_frame/dense_full_sidebar` — worst realistic frame: every cell
//!   of a 60x160 grid holds a glyph, assorted SGR foreground/background
//!   colours, sidebar host list, status line.
//! - `renderer_frame/idle_prompt_sidebar` — the common idle frame: a prompt
//!   row, blank grid, sidebar and status still drawn.
//! - `snapshot/empty_scrollback` / `snapshot/full_scrollback` — per-frame
//!   snapshot cost at both scrollback extremes.
//!
//! Run with: `cargo bench -p noren-app --features bench-support renderer_frame`
//! (add `snapshot` for the snapshot group).

// The re-included renderer source carries the window-bound `Renderer` and
// its error/outcome enums, which this bench never drives. Its `#[cfg(test)]`
// module is also compiled (bench targets set `cfg(test)`) but its `#[test]`
// functions are not roots under `harness = false`, so `dead_code` and
// `unused_imports` would otherwise fire on live-only test code. The shipped
// compile still checks all of it normally. Same `#[path]` precedent as
// `renderer_capture.rs`.
#[allow(dead_code, unused_imports)]
#[path = "../src/renderer.rs"]
mod renderer_source;

use criterion::{Criterion, criterion_group, criterion_main};
use noren_app::GridGeometry;
use noren_app::theme::DARK;
use noren_terminal::TerminalState;
use renderer_source::{SIDEBAR_COLS, Target, glyph_vertices_for};

/// The renderer's ceilings (`MAX_RENDER_ROWS`/`MAX_RENDER_COLS`).
const ROWS: u16 = 60;
const COLS: u16 = 160;

/// PoC cell metrics (10x20 px), via the same public geometry the binary uses.
fn metrics() -> noren_app::CellMetrics {
    GridGeometry::poc().cell_metrics()
}

/// Window size that fits the sidebar plus a full 60x160 grid.
fn window_size(metrics: noren_app::CellMetrics) -> (u32, u32) {
    (
        (SIDEBAR_COLS as u32 + u32::from(COLS)) * metrics.width(),
        u32::from(ROWS) * metrics.height(),
    )
}

fn sidebar_lines() -> Vec<String> {
    (0..24)
        .map(|index| format!("host-{index:02}.example"))
        .collect()
}

const STATUS: &str = "2 worktrees · host-04 · zsh";

/// One deterministic 160-column line with an SGR foreground run and, every
/// fourth line, an SGR background block.
fn dense_line(seed: usize) -> String {
    let color = 31 + seed % 7;
    let mut line = String::new();
    let word = format!("{seed:04}-noren ");
    while line.chars().count() + word.chars().count() <= COLS as usize {
        line.push_str(&word);
    }
    if seed % 4 == 0 {
        format!("\x1b[48;5;238m{line}\x1b[0m")
    } else {
        format!("\x1b[{color}m{line}\x1b[0m")
    }
}

/// A terminal whose visible grid is dense text (worst realistic frame).
fn dense_terminal() -> TerminalState {
    let mut terminal = TerminalState::new(ROWS, COLS).expect("valid grid");
    for seed in 0..=ROWS {
        let mut line = dense_line(usize::from(seed));
        // Trim/pad to exactly COLS characters so every cell holds a glyph.
        line = line.chars().take(COLS as usize).collect();
        terminal.feed_bytes(line.as_bytes());
        terminal.feed_bytes(b"\r\n");
    }
    terminal
}

/// An idle terminal: cleared screen with a shell prompt on the first row.
fn idle_terminal() -> TerminalState {
    let mut terminal = TerminalState::new(ROWS, COLS).expect("valid grid");
    terminal.feed_bytes(b"\x1b[2J\x1b[Huser@host-04 ~ % ");
    terminal
}

/// A terminal with the scrollback completely full (10,000 rows) plus the
/// visible grid, built by feeding more lines than the cap retains.
fn full_scrollback_terminal() -> TerminalState {
    let mut terminal = TerminalState::new(ROWS, COLS).expect("valid grid");
    let line = "x".repeat(COLS as usize);
    for _ in 0..10_100 {
        terminal.feed_bytes(line.as_bytes());
        terminal.feed_bytes(b"\r\n");
    }
    terminal
}

fn bench_renderer_frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("renderer_frame");

    // Built once per process, before any timing: only frame prep and the
    // snapshot itself are measured.
    let dense = (dense_terminal(), sidebar_lines(), STATUS.to_owned());
    let idle = (idle_terminal(), sidebar_lines(), STATUS.to_owned());
    let cell_metrics = metrics();
    let (width, height) = window_size(cell_metrics);

    group.bench_function("dense_full_sidebar", |b| {
        let (terminal, sidebar, status) = &dense;
        let snapshot = terminal.snapshot();
        b.iter(|| {
            let target = Target::new(&DARK, width, height, cell_metrics);
            let vertices = glyph_vertices_for(
                target,
                Some(&snapshot),
                Some(sidebar),
                Some(status.as_str()),
            );
            std::hint::black_box(vertices.len())
        })
    });

    group.bench_function("idle_prompt_sidebar", |b| {
        let (terminal, sidebar, status) = &idle;
        let snapshot = terminal.snapshot();
        b.iter(|| {
            let target = Target::new(&DARK, width, height, cell_metrics);
            let vertices = glyph_vertices_for(
                target,
                Some(&snapshot),
                Some(sidebar),
                Some(status.as_str()),
            );
            std::hint::black_box(vertices.len())
        })
    });
    group.finish();
}

fn bench_snapshot(c: &mut Criterion) {
    let mut group = c.benchmark_group("snapshot");

    let empty = idle_terminal();
    let full = full_scrollback_terminal();

    group.bench_function("empty_scrollback_60x160", |b| {
        b.iter(|| std::hint::black_box(empty.snapshot()))
    });
    group.bench_function("full_scrollback_60x160", |b| {
        b.iter(|| std::hint::black_box(full.snapshot()))
    });
    group.finish();
}

criterion_group!(benches, bench_renderer_frame, bench_snapshot);
criterion_main!(benches);
