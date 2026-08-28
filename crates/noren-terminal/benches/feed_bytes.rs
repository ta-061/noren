//! `TerminalState::feed_bytes` throughput benchmarks — the hot path for every
//! byte a child program prints.
//!
//! History: this workspace twice shipped performance defects that only
//! surfaced when someone measured (the quadratic `SshConfig::from_blocks`
//! walk, and #137's mixed literal+wildcard resolution), and twice had to
//! convert wall-clock test assertions into operation counts (#154, #159).
//! These benchmarks exist so parser throughput is a *measured* property
//! instead of an occasional discovery. They report numbers; they never gate
//! on a timing threshold — the regression guard for bounded work stays the
//! operation-count assertions in the unit suites.
//!
//! Input shapes follow what a user actually feels:
//!
//! - `plain_*` — `cat` of a large file: pure printable ASCII lines. The 256 KiB
//!   and 1 MiB sizes at one geometry expose superlinear behavior (scrollback
//!   saturates at `MAX_SCROLLBACK_LINES` = 10_000 roughly 70 KiB in, so both
//!   sizes measure steady-state scrolling and their MiB/s must agree).
//! - `sgr_*` — `grep --color`/`ls --color` shape: color escapes churn the pen
//!   on nearly every token.
//! - `utf8_*` — mixed Latin/CJK text: wide characters exercise the
//!   display-width placement and continuation-cell path.
//!
//! Bytes are fed in 16 KiB chunks, mirroring the app's `READ_CHUNK_BYTES`
//! read size (the constant lives in `noren-app`; it is mirrored here with a
//! comment rather than making the terminal crate depend on the app).
//!
//! Run with: `cargo bench -p noren-terminal --features bench-support`.

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use noren_terminal::TerminalState;

/// App-level `READ_CHUNK_BYTES` (16 KiB), mirrored; see the module docs.
const CHUNK_BYTES: usize = 16 * 1024;

/// The app's renderer ceilings (`MAX_RENDER_ROWS`/`MAX_RENDER_COLS` = 60×160),
/// mirrored as numbers: `noren-terminal` must not depend on `noren-app`.
const LARGE_ROWS: u16 = 60;
const LARGE_COLS: u16 = 160;

/// Classic terminal geometry, for comparison against the large grid.
const CLASSIC_ROWS: u16 = 24;
const CLASSIC_COLS: u16 = 80;

/// A small fixed vocabulary so corpora are deterministic without a PRNG.
const WORDS: [&str; 16] = [
    "fn", "let", "match", "return", "struct", "impl", "pub", "use", "self", "crate", "where",
    "async", "await", "move", "loop", "break",
];

/// One deterministic source-code-ish line for `seed`.
fn plain_line(seed: usize) -> String {
    let mixed = (seed as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let word_count = 2 + (mixed >> 33) as usize % 9;
    (0..word_count)
        .map(|index| WORDS[(mixed >> (index * 7)) as usize % WORDS.len()])
        .collect::<Vec<_>>()
        .join("-")
}

/// Printable ASCII corpus of at least `target_bytes` (the `cat` shape).
fn plain_corpus(target_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(target_bytes + 128);
    let mut seed = 0;
    while out.len() < target_bytes {
        out.extend_from_slice(plain_line(seed).as_bytes());
        out.push(b'\n');
        seed += 1;
    }
    out
}

/// SGR-churning corpus (`--color` tool output shape) of at least
/// `target_bytes`.
fn sgr_corpus(target_bytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(target_bytes + 128);
    let mut seed = 0;
    while out.len() < target_bytes {
        let color = 17 + seed % 216;
        let line = plain_line(seed);
        let rest = plain_line(seed.wrapping_add(1));
        out.extend_from_slice(
            format!("\x1b[38;5;{color}m{line}\x1b[0m:{rest}\x1b[1m:\x1b[0m\n").as_bytes(),
        );
        seed = seed.wrapping_add(2);
    }
    out
}

/// Mixed Latin/CJK corpus of at least `target_bytes`.
fn utf8_corpus(target_bytes: usize) -> Vec<u8> {
    // U+4E00..U+9FA5 are wide CJK ideographs; every other line is pure ASCII.
    let cjk_line = |seed: usize| -> String {
        (0..8u32)
            .map(|index| {
                let code = 0x4E00u32 + ((seed as u32 * 7 + index * 131) % 0x1765);
                char::from_u32(code).expect("in CJK block")
            })
            .collect()
    };
    let mut out = Vec::with_capacity(target_bytes + 128);
    let mut seed = 0;
    while out.len() < target_bytes {
        out.extend_from_slice(plain_line(seed).as_bytes());
        out.push(b'\n');
        out.extend_from_slice(cjk_line(seed).as_bytes());
        out.push(b'\n');
        seed += 1;
    }
    out
}

/// Feed `bytes` in `CHUNK_BYTES` slices, then surface a cheap aggregate so
/// the mutation cannot be optimized away.
fn feed_all(terminal: &mut TerminalState, bytes: &[u8]) -> usize {
    for chunk in bytes.chunks(CHUNK_BYTES) {
        terminal.feed_bytes(chunk);
    }
    std::hint::black_box(terminal.scrollback_len())
}

fn bench_feed_bytes(c: &mut Criterion) {
    let mut group = c.benchmark_group("feed_bytes");
    // These iterations are long (the scrolled-grid memmove dominates), so the
    // criterion defaults (100 samples, 3 s warm-up) would keep the whole
    // suite running for many minutes. Ten samples per case are plenty to see
    // a regression in a throughput number of this magnitude.
    group.sample_size(10);
    group.warm_up_time(std::time::Duration::from_secs(1));

    // 256 KiB and 1 MiB at one geometry expose superlinear behavior: the
    // scrollback cap (`MAX_SCROLLBACK_LINES` = 10_000) is reached after roughly
    // 70 KiB of this corpus, so both sizes measure steady-state scrolling and
    // their MiB/s must agree — a falling MiB/s with size means something new
    // is quadratic.
    let cases: [(&str, Vec<u8>, u16, u16); 5] = [
        (
            "plain_256kib_60x160",
            plain_corpus(256 * 1024),
            LARGE_ROWS,
            LARGE_COLS,
        ),
        (
            "plain_1mib_60x160",
            plain_corpus(1024 * 1024),
            LARGE_ROWS,
            LARGE_COLS,
        ),
        (
            "plain_256kib_24x80",
            plain_corpus(256 * 1024),
            CLASSIC_ROWS,
            CLASSIC_COLS,
        ),
        (
            "sgr_256kib_60x160",
            sgr_corpus(256 * 1024),
            LARGE_ROWS,
            LARGE_COLS,
        ),
        (
            "utf8_256kib_60x160",
            utf8_corpus(256 * 1024),
            LARGE_ROWS,
            LARGE_COLS,
        ),
    ];

    for (name, bytes, rows, cols) in cases {
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_function(name, |b| {
            b.iter_batched(
                || TerminalState::new(rows, cols).expect("valid grid"),
                |mut terminal| feed_all(&mut terminal, &bytes),
                BatchSize::PerIteration,
            )
        });
    }
    group.finish();
}

criterion_group!(benches, bench_feed_bytes);
criterion_main!(benches);
