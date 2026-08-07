//! Seeded soak harness over the `feed_bytes` untrusted-input boundary.
//!
//! `TerminalState::feed_bytes` is the byte boundary named by the threat model
//! (TM-04) and `docs/testing/strategy.md` asks for "at least 100 rapid
//! resize/input/output interleavings and preserve the smallest failing seed as
//! a regression fixture". Neither `adversarial.rs` (23 hand-written hostile
//! sequences) nor `adversarial_kimi.rs` (20 more) do *randomised* interleaving;
//! that is the unique value of this suite.
//!
//! What this harness does:
//!
//! - Drives a small seeded PRNG (splitmix64 to seed xorshift64) so every run is
//!   reproducible from a `u64` seed. No `rand`, no nightly, no new dependency.
//! - Each step picks one of the boundary operations named in the strategy:
//!   `feed_bytes` (from a hostile corpus), `resize`, scroll-region changes
//!   (CSI and the public `set_scroll_region`), alternate-screen toggles, and
//!   other DEC private mode/keypad switches.
//! - Asserts the public invariants after **every** step. The oracle is the same
//!   one `tests/adversarial.rs::assert_invariants` uses — copied verbatim, not
//!   reinvented. A panic or an invariant breach is the only failure mode.
//! - On any failure the panic names the seed and step. Setting `SOAK_SEED`
//!   replays exactly that seed with per-step tracing so the case can be pinned
//!   as a permanent `#[ignore = "reproduces <id>"]` regression.
//!
//! Iteration bounds (documented): `STEPS_PER_SEED = 100` (the strategy's floor),
//! `SEED_SWEEP_COUNT = 8192` seeds in the sweep -> 819_200 randomised steps,
//! plus the default-seed test's 100. Measured wall time is recorded in the
//! handoff; the whole file runs in well under the 10-second budget by staying
//! on tiny grids.

use std::panic::AssertUnwindSafe;

use noren_terminal::{CursorMove, MAX_SCREEN_CELLS, TerminalState};

// ===== Invariant oracle (verbatim from tests/adversarial.rs) =====

/// Public invariants that must hold after *any* sequence of public calls.
///
/// This is the same oracle as `adversarial.rs::assert_invariants`; each test
/// file is its own crate so it is copied, not imported. Do not diverge from the
/// adversarial definition without coordinating both suites.
fn assert_invariants(state: &TerminalState, context: &str) {
    let (rows, cols) = state.size();
    assert!(rows > 0 && cols > 0, "{context}: non-zero size");
    assert!(
        usize::from(rows) * usize::from(cols) <= MAX_SCREEN_CELLS,
        "{context}: cell cap"
    );
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
}

// ===== Seeded PRNG: splitmix64 seeding + xorshift64 stream =====
//
// xorshift64 needs a non-zero, well-mixed initial state. splitmix64 turns any
// seed (including 0) into one, so seeds are directly reproducible. Both are
// public-domain algorithms; no crate is pulled in.

struct Xorshift64(u64);

impl Xorshift64 {
    fn from_seed(seed: u64) -> Self {
        let mut mixer = seed;
        let state = splitmix64(&mut mixer);
        // splitmix64 cannot return 0 except for a single alignment of the
        // 128-bit stream; OR-ing 1 keeps xorshift64 valid with no practical
        // loss of entropy.
        Self(state | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Uniform-ish value in `0..n` (n must be >= 1). Modulo bias is negligible
    /// for a soak harness; determinism, not cryptographic uniformity, is the
    /// property being relied on.
    fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % u64::from(n)) as u32
    }

    fn u16_below(&mut self, n: u16) -> u16 {
        (self.next_u64() % u64::from(n)) as u16
    }

    fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    fn pick<'a, T>(&mut self, slice: &'a [T]) -> &'a T {
        let idx = self.u16_below(slice.len() as u16) as usize;
        &slice[idx]
    }

    /// Pick one borrowed byte slice out of a corpus (`&[&[u8]]`). Separate from
    /// the generic [`pick`](Self::pick) so type inference never tries to treat
    /// `[u8]` itself as a `Sized` element.
    fn pick_bytes<'a>(&mut self, slice: &[&'a [u8]]) -> &'a [u8] {
        let idx = self.u16_below(slice.len() as u16) as usize;
        slice[idx]
    }
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

// ===== Hostile byte corpus fed through `feed_bytes` =====
//
// Self-contained chunks of untrusted PTY output. Variety is the point: CSI with
// saturating parameters, OSC, invalid UTF-8, wide/combining characters, scroll
// storms, edit ops, aborted sequences, and bare printables. None of this is
// duplicated from the adversarial suites — those fix *one* sequence per test;
// the harness randomly interleaves many of these with resize and mode changes.

const HOSTILE_CORPUS: &[&[u8]] = &[
    b"hello world\r\n",
    b"\x1b[2;4H",
    b"\x1b[999999999;999999999H",
    b"\x1b[65535A\x1b[65535B\x1b[65535C\x1b[65535D",
    b"\x1b[65535S\x1b[65535T\x1b[65535L\x1b[65535M",
    b"\x1b[2J\x1b[2K\x1b[J\x1b[K\x1b[1J\x1b[1K",
    b"\x1b[1;31m\x1b[4m\x1b[m",
    b"\x1b[999999999;1;4;999999999m",
    b"\x1b[2;4r",
    b"\x1b[r",
    b"\x1b[4;2r",
    b"\x1b]0;title\x07",
    b"\x1b]0;x\x1b\\",
    b"\x1b]0;unterminated-osc",
    b"\x1b7\x1b8",
    b"\x1bD\x1bM",
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[?7l\x1b[?7h",
    b"\x1b[?25l\x1b[?25h",
    b"\x1b[?1h\x1b=",
    b"\x1b[?1l\x1b>",
    b"\x1b[?2004h",
    b"\x1b[?2004l",
    b"\x1b(B",
    b"\t\t\t",
    b"\r\n",
    b"\x1b[2P\x1b[2@\x1b[2X\x1b[2C",
    &[0xff, 0xc0, 0xaf, 0xed, 0xa0, 0x80],
    "\u{0301}".as_bytes(),
    "日".as_bytes(),
    "e\u{0301}".as_bytes(),
    b"\x1b[;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;H",
    b"\x1b[",
    b"\x1b",
    b"",
    b"A",
];

// DEC private mode escape sequences used for the "mode switch" action.
const MODE_SWITCHES: &[&[u8]] = &[
    b"\x1b[?1h",
    b"\x1b[?1l",
    b"\x1b[?2004h",
    b"\x1b[?2004l",
    b"\x1b=",
    b"\x1b>",
];

/// The randomised boundary operations the harness interleaves.
#[derive(Debug)]
enum Action {
    FeedCorpus,
    FeedByteAtATime,
    Resize,
    ScrollRegionCsi,
    ScrollRegionPublic,
    AltScreenToggle,
    ModeSwitch,
    CursorMove,
}

const ACTION_TABLE: &[Action] = &[
    Action::FeedCorpus,
    Action::FeedCorpus,
    Action::FeedCorpus,
    Action::FeedByteAtATime,
    Action::Resize,
    Action::Resize,
    Action::ScrollRegionCsi,
    Action::ScrollRegionPublic,
    Action::AltScreenToggle,
    Action::ModeSwitch,
    Action::CursorMove,
];

/// One randomised step. Every public call lands inside here, and
/// [`assert_invariants`] is checked by the caller immediately after.
fn apply_step(state: &mut TerminalState, rng: &mut Xorshift64, action: &Action) {
    match action {
        Action::FeedCorpus => {
            let bytes = rng.pick_bytes(HOSTILE_CORPUS);
            state.feed_bytes(bytes);
        }
        Action::FeedByteAtATime => {
            // Splitting a corpus entry across feed boundaries stresses the
            // parser's partial-sequence recovery, which whole-feed never hits.
            let bytes = rng.pick_bytes(HOSTILE_CORPUS);
            for byte in bytes {
                state.feed_bytes(std::slice::from_ref(byte));
            }
        }
        Action::Resize => {
            // Tiny grids keep the sweep fast and stay far below the cell cap.
            let rows = 1 + rng.u16_below(12);
            let cols = 1 + rng.u16_below(16);
            let _ = state.resize(rows, cols);
        }
        Action::ScrollRegionCsi => {
            let (rows, _cols) = state.size();
            // Random 1-based params, deliberately out of range sometimes to
            // exercise DECSTBM clamping and rejection.
            let top = 1 + rng.u16_below(rows);
            let bottom = 1 + rng.u16_below(rows);
            state.feed_bytes(format!("\x1b[{top};{bottom}r").as_bytes());
        }
        Action::ScrollRegionPublic => {
            let (rows, _cols) = state.size();
            let top = rng.u16_below(rows);
            let bottom = rng.u16_below(rows);
            // Rejection is a valid outcome; invariants must hold either way.
            let _ = state.set_scroll_region(top, bottom);
        }
        Action::AltScreenToggle => {
            if rng.next_bool() {
                state.feed_bytes(b"\x1b[?1049h");
            } else {
                state.feed_bytes(b"\x1b[?1049l");
            }
        }
        Action::ModeSwitch => {
            state.feed_bytes(rng.pick_bytes(MODE_SWITCHES));
        }
        Action::CursorMove => {
            let mv = match rng.below(9) {
                0 => CursorMove::Up(1 + rng.u16_below(8)),
                1 => CursorMove::Down(1 + rng.u16_below(8)),
                2 => CursorMove::Right(1 + rng.u16_below(8)),
                3 => CursorMove::Left(1 + rng.u16_below(8)),
                4 => CursorMove::NextLine(1 + rng.u16_below(8)),
                5 => CursorMove::PreviousLine(1 + rng.u16_below(8)),
                6 => CursorMove::To {
                    row: rng.u16_below(16),
                    column: rng.u16_below(16),
                },
                7 => CursorMove::ToColumn(rng.u16_below(16)),
                _ => CursorMove::ToRow(rng.u16_below(16)),
            };
            state.move_cursor(mv);
        }
    }
}

/// The deterministic per-seed scenario: a fresh tiny terminal driven for
/// `steps` randomised interleavings, with the invariant oracle asserted after
/// every single step.
fn run_seed(seed: u64, steps: usize, trace: bool) {
    let mut rng = Xorshift64::from_seed(seed);
    let start_rows = 1 + rng.u16_below(6);
    let start_cols = 1 + rng.u16_below(10);
    let mut state = TerminalState::new(start_rows, start_cols).expect("valid initial terminal");
    assert_invariants(&state, &format!("seed={seed:#018x} init"));

    for step in 0..steps {
        let action = rng.pick(ACTION_TABLE);
        if trace {
            eprintln!(
                "[soak] seed={seed:#018x} step={step} size={:?} cursor={:?} region={:?} wrap={} action={action:?}",
                state.size(),
                state.cursor(),
                state.scroll_region(),
                state.is_wrap_pending(),
            );
        }
        apply_step(&mut state, &mut rng, action);
        assert_invariants(
            &state,
            &format!("seed={seed:#018x} step={step} action={action:?}"),
        );
    }
}

/// Iterate the strategy's named floor: 100 randomised interleavings per seed.
const STEPS_PER_SEED: usize = 100;

/// Number of seeds swept by [`soak_feed_bytes_seed_sweep`]. Bounded so the
/// whole file stays well under the 10-second budget; the total number of
/// randomised steps is `SEED_SWEEP_COUNT` times `STEPS_PER_SEED`. Measured at
/// ~0.3 ms/seed on macOS arm64 debug, so 8192 seeds (~820k randomised steps)
/// lands around 2.5 s. See the handoff for the recorded wall time.
const SEED_SWEEP_COUNT: u64 = 8192;

/// The default seed for the deterministic regression. Fixed so this test is a
/// stable, always-on baseline; any change to it must keep the test green.
const DEFAULT_SEED: u64 = 0x5050_C0DE;

// ===== Tests =====

/// The literal strategy.md requirement: exactly 100 randomised interleavings
/// against the `feed_bytes` boundary, invariants asserted after every step,
/// fully reproducible from `DEFAULT_SEED`.
#[test]
fn soak_feed_bytes_default_seed_100_interleavings() {
    run_seed(DEFAULT_SEED, STEPS_PER_SEED, false);
}

/// The randomised-interleaving value the hand-written suites cannot give: many
/// distinct seeds, each driving 100 interleavings of feed/resize/mode/region.
///
/// Each seed is isolated behind `catch_unwind` so a panic or invariant breach
/// is attributed to its exact seed. If `SOAK_SEED` is set (e.g. `0xC0FFEE`),
/// only that seed runs with per-step tracing — the workflow for pinning a newly
/// found defect as an `#[ignore]` regression.
#[test]
fn soak_feed_bytes_seed_sweep() {
    if let Ok(replay) = std::env::var("SOAK_SEED") {
        let seed = u64::from_str_radix(replay.trim_start_matches("0x"), 16)
            .unwrap_or_else(|_| panic!("SOAK_SEED={replay:?} is not a hex u64"));
        eprintln!("[soak] replaying single seed {seed:#018x} with tracing");
        run_seed(seed, STEPS_PER_SEED, true);
        return;
    }

    for seed in 0..SEED_SWEEP_COUNT {
        let outcome =
            std::panic::catch_unwind(AssertUnwindSafe(|| run_seed(seed, STEPS_PER_SEED, false)));
        if outcome.is_err() {
            panic!(
                "SOAK DEFECT: seed={seed:#018x} step=<see panic above> \
                 reproduce with: SOAK_SEED={seed:#x} cargo test --package noren-terminal \
                 --test soak_feed_bytes soak_feed_bytes_seed_sweep -- --nocapture"
            );
        }
    }
}
