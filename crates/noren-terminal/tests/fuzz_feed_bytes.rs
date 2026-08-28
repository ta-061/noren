//! Fuzz harness for the `TerminalState::feed_bytes` untrusted-input boundary.
//!
//! Route decision (recorded because M7 names fuzzing and this is its first
//! slice): `cargo-fuzz` is NOT installed on this machine and the workspace
//! pins a stable, minimal-profile toolchain (`rust-toolchain.toml` = 1.88.0,
//! clippy + rustfmt only), while `cargo-fuzz` targets require a nightly
//! toolchain and a `cargo install`. Adding either would be a dependency the
//! project's CI cannot run. This harness is therefore hand-rolled and runs
//! under plain `cargo test`, exactly like `tests/soak_feed_bytes.rs`, and
//! adds NO dependency.
//!
//! Relationship to the soak: the soak randomly interleaves a FIXED hostile
//! corpus with resize/mode/region operations — it explores interleavings of
//! known-bad input. This file explores the BYTE SPACE around known-good
//! input: it synthesises near-valid sequences from a grammar seeded by real
//! program output (Zellij attach, vim/tmux mouse modes, DECSET/DECRST, SGR,
//! DECSTBM, OSC/DCS), applies structure-aware mutations (truncate, flip,
//! insert, delete, splice), and feeds the result through `feed_bytes` in
//! randomly chunked and byte-at-a-time splits. Random bytes mostly exercise
//! the "ignore garbage" path; structured near-valid input is where parsers
//! break.
//!
//! The ORACLE has three layers, and its guarantees are only as strong as
//! the union of them:
//!
//! 1. Shape (verbatim): `assert_invariants` copied VERBATIM from
//!    `tests/adversarial.rs` (each integration test file is its own crate,
//!    so it is copied, not imported — same provenance rule the soak
//!    states). It asserts: dimensions non-zero, cells within
//!    `MAX_SCREEN_CELLS`, grid cell count consistent with the dimensions,
//!    cursor row/column in bounds, and scroll region ordered (top <=
//!    bottom) and within the screen. A panic anywhere in `feed_bytes` is
//!    the other failure mode and fails the case.
//! 2. Structural content, any input (fuzz-local): [`assert_state_oracles`]
//!    additionally asserts after every feed/resize that scrollback stays
//!    within `MAX_SCROLLBACK_LINES`, the cursor never rests on the
//!    continuation half of a wide character, and every wide-character
//!    lead/continuation pair in the grid is intact.
//! 3. Decoded/pen/buffer content (fuzz-local): [`run_content_probes`]
//!    appends deterministic probes to EVERY generated case (and to the
//!    corpus sweep's originals plus a one-in-eight mutation lattice). Each
//!    probe forces its own preconditions (`ESC \` grounds the parser, CUP
//!    1;1 positions and clears pending autowrap, SGR 0 clears the pen), so
//!    its expected outcome holds for ANY preceding input: a printed
//!    character is retrievable verbatim from the cell it landed in (all
//!    UTF-8 byte classes plus a combining mark), a compound SGR lands
//!    exactly in the pen and in cells written under it, `SGR 0` (and the
//!    empty `SGR`) returns the pen to the exact default, an overflowing
//!    SGR leaves the pen untouched, SCS final bytes neither print nor move
//!    the cursor, tabs stay inside the row without leaving autowrap
//!    pending, and mode-1049 alternate-screen switching moves writes
//!    between buffers without touching the primary's content.
//!
//! What this oracle still does NOT check (known-unasserted; do not quote
//! the clean-run numbers as covering these): scrollback CONTENT (only the
//! cap is asserted — retention, order, and row-width preservation are the
//! scrollback host tests' job); erase/scroll/insert/delete CONTENT effects
//! (shape only — semantics live in the erase_operation/scroll_regions host
//! tests); exact tab-stop arithmetic (only bounds: row kept, column in
//! bounds, wrap cleared — the 8-column stop math and its interaction with
//! wide characters is unasserted here); the wide-character squeeze/wrap
//! policy at the right screen edge (probes only print where the full width
//! fits); alternate-screen modes 47/1047/1048 (this parser maps only 1049,
//! so only 1049 is probed); charset translation (none is implemented —
//! only final-byte non-leak is asserted); the combining-marks-per-cell cap
//! (one-mark attachment is asserted, the cap policy is not); and pen
//! persistence across a 1049 switch (terminal-global by design, covered by
//! the sgr host tests).
//!
//! Reproducibility: every case is derived deterministically from
//! (`root seed`, case index) — see [`case_rng`]. On failure the panic
//! prints the root seed, the case index, and the exact failing bytes as a
//! Rust `b"..."` literal, so the input can be pinned as a regression test
//! immediately. Environment controls:
//!
//! - `FUZZ_ROOT_SEED` (hex `u64`, default [`DEFAULT_ROOT_SEED`]): replay a
//!   run under a different root seed.
//! - `FUZZ_CASES` (`u64`, default [`DEFAULT_CASES`]): how many generated
//!   cases `fuzz_feed_bytes_generated_streams` runs.
//! - `FUZZ_CASE_INDEX` (`u64`): run exactly that one generated case with
//!   per-step tracing (the workflow for pinning a newly found defect).
//! - `FUZZ_SECONDS` (`u64`): when set, `fuzz_feed_bytes_campaign` runs
//!   generated cases until the deadline instead of returning immediately,
//!   then prints the iteration count. This is the "run the fuzzer for a
//!   bounded time" door; it is OFF in normal `cargo test` so the suite
//!   stays fast (the committed bound is [`DEFAULT_CASES`] cases, measured
//!   in the module docs of the test below).
//!
//! Iteration bounds (documented, deterministic): the generated-streams test
//! runs `DEFAULT_CASES` = 2500 cases of at most 8 sequences each — each now
//! also carrying the full content-probe suite; the corpus mutation sweep is
//! systematic (every truncation, byte flip, substitution, and insertion
//! site of every corpus entry) and needs no randomness. Both together stay
//! under the 10-second test budget on tiny grids (measured: the whole file
//! runs in ~0.9 s on this macOS arm64 debug machine under load; ~0.2 s of
//! that is the probe suites), so `cargo test --workspace` gains under a
//! second.
//!
//! Campaign evidence, shape-only revision (2026-08-28, macOS arm64 debug,
//! `FUZZ_SECONDS=60` under three root seeds — `0xF00F_BEEF_5EED_0A11`,
//! `0x1`, and `0xC0FF_EE42_DEAD_BEEF`): 718_097 + 728_935 + 853_179 =
//! 2_300_211 generated cases, ~219 MiB fed, ZERO panics and ZERO
//! structural violations. Note the limit of that result: the oracle then
//! asserted shape only, so it could not see decode/pen defects (an
//! independently injected dropped-UTF-8-continuation defect and an
//! SGR-0-no-op defect both survived it green; both are caught at case 0 by
//! the content revision below).
//!
//! Campaign evidence, content-oracle revision (2026-08-28, macOS arm64
//! debug, `FUZZ_SECONDS=20` under the same three root seeds): seed
//! `0xF00F_BEEF_5EED_0A11`: 46_126 cases / ~4.2 MiB, ZERO panics, ZERO
//! violations. Seed `0x1`: 77_008 cases / ~7.0 MiB, ZERO panics, ZERO
//! violations. Seed `0xC0FF_EE42_DEAD_BEEF`: HALTED at case 1336 by the
//! open defect below (1 violation). Iteration rates are lower than the
//! shape-only revision (718k+ cases in 60 s there) because every case now
//! also runs the any-input state oracles per feed and the full content
//! probe suite; the machine was also under heavy load (load average ~58).
//! The committed default seed/bound (2500 cases) stays green. Future
//! campaigns should append their totals here.
//!
//! # OPEN DEFECT FOUND BY THE CONTENT ORACLES (unfixed, pre-existing on
//! # main, reported for its own issue/review)
//!
//! The "cursor never rests on a wide-character continuation cell"
//! invariant — which `move_cursor` documents and which LF/IND/NEL/RI
//! maintain via `snap_cursor_to_lead` ("a path added later cannot forget
//! the re-snap") — is VIOLATED by the three content-scrolling paths that
//! shift rows under a STATIONARY cursor and do not re-snap:
//!
//! - `CSI Ps T` (SD, scroll down): `TerminalState::new(2,4)` +
//!   `feed_bytes(b"\xf0\x9f\x98\x80\n\x08\x1b[1T")` — the emoji pair from
//!   row 0 lands under the cursor at (1,1), a continuation cell.
//! - `CSI Ps S` (SU, scroll up): `TerminalState::new(3,4)` +
//!   `feed_bytes(b"\n\xf0\x9f\x98\x80\x1b[1;2H\x1b[1S")`.
//! - `CSI Ps M` (DL, delete lines): `TerminalState::new(3,4)` +
//!   `feed_bytes(b"\n\xf0\x9f\x98\x80\x1b[1;2H\x1b[1M")`.
//!
//! LF at the bottom margin scrolls under the cursor too but stays clean
//! because `index()` re-snaps afterwards. Consequence of the defect: a
//! print at the stranded position overwrites only the continuation half
//! and `repair_row` then blanks the orphaned lead, so the wide glyph is
//! destroyed one column to the left of where a snapped cursor would have
//! replaced it. Fuzz repro: `FUZZ_ROOT_SEED=0xc0ffee42deadbeef
//! FUZZ_CASE_INDEX=1336 cargo test -p noren-terminal --test fuzz_feed_bytes
//! fuzz_feed_bytes_generated_streams -- --nocapture` (fails at "feed byte
//! 23"). The oracle stays enabled on purpose; the committed default bound
//! does not reach a triggering case.

use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use noren_terminal::{
    Cell, CellAttributes, Color, MAX_SCREEN_CELLS, MAX_SCROLLBACK_LINES, TerminalState,
};

// ===== Invariant oracle (verbatim from tests/adversarial.rs) =====

/// Public invariants that must hold after *any* sequence of public calls.
///
/// This is the same oracle as `adversarial.rs::assert_invariants` and
/// `soak_feed_bytes.rs::assert_invariants`; each test file is its own crate
/// so it is copied, not imported. Do not diverge from the adversarial
/// definition without coordinating all three suites.
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

// ===== Extended any-input oracles (fuzz-local; NOT part of the verbatim
// adversarial copy above) =====
//
// The shape-only oracle above cannot see a decoder or SGR regression: a
// parser that drops every CJK glyph or turns `SGR 0` into a no-op keeps all
// structural invariants green. The two checks in this section close that
// gap from both directions:
//
// 1. [`assert_state_oracles`] — structural-content invariants that must
//    hold after ANY input: scrollback stays within its cap, the cursor
//    never rests on the continuation half of a wide character, and every
//    wide-character pair in the grid is intact (each width-2 lead is
//    directly followed by its continuation cell; each continuation
//    directly follows a width-2 lead). The pair scan is a public-API
//    mirror of the crate-private `ScreenBuffer::wide_cells_intact`.
//
// 2. [`run_content_probes`] — deterministic content probes appended to
//    EVERY case (generated and corpus-sweep alike). Each probe first
//    forces the parser into a known state (`ESC \` grounds the state
//    machine from any parser state; `CUP 1;1` clears pending autowrap and
//    positions absolutely; `SGR 0` clears the pen), so its expected
//    outcome is independent of whatever hostile bytes preceded it. That
//    is what makes the probes valid for ANY generated input rather than
//    hand-picked cases.

/// Content-level invariants that must hold after any sequence of public
/// calls, checked after every feed and resize alongside the verbatim shape
/// oracle.
fn assert_state_oracles(state: &TerminalState, context: &str) {
    assert!(
        state.scrollback_len() <= MAX_SCROLLBACK_LINES,
        "{context}: scrollback within cap"
    );
    let cursor = state.cursor();
    assert!(
        !state
            .screen()
            .cell(cursor.row(), cursor.column())
            .is_some_and(Cell::is_continuation),
        "{context}: cursor off continuation cell"
    );
    let (_, cols) = state.size();
    for row_cells in state.screen().cells().chunks(usize::from(cols)) {
        let mut index = 0;
        while index < row_cells.len() {
            if row_cells[index].is_continuation() {
                assert!(
                    index > 0 && row_cells[index - 1].width() == 2,
                    "{context}: continuation cell follows a wide lead"
                );
                index += 1;
            } else if row_cells[index].width() == 2 {
                assert!(
                    index + 1 < row_cells.len() && row_cells[index + 1].is_continuation(),
                    "{context}: wide lead has its continuation cell"
                );
                index += 2;
            } else {
                index += 1;
            }
        }
    }
}

/// One print round-trip probe: force the cursor to the grid origin and
/// print `bytes`; the cell that the character landed in must return exactly
/// `expect` as its text.
fn print_round_trip(
    state: &mut TerminalState,
    bytes: &[u8],
    expect: &str,
    label: &str,
    context: &str,
) {
    // `CUP 1;1` is absolute (origin mode is not implemented), clamps inside
    // the grid, and clears pending autowrap, so the print lands at (0, 0)
    // regardless of the preceding stream. Width-2 characters additionally
    // need a second column to land in place rather than wrap.
    state.feed_bytes(b"\x1b[1;1H");
    state.feed_bytes(bytes);
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").text(),
        expect,
        "{context}: {label} round-trip identity"
    );
}

/// Deterministic content probes run at the end of every case. Every probe
/// re-establishes its own preconditions, so each expectation holds for ANY
/// preceding input; a failure here names the probe in the panic message and
/// the case replay (`FUZZ_ROOT_SEED`/`FUZZ_CASE_INDEX`) reproduces it.
fn run_content_probes(state: &mut TerminalState, context: &str) {
    let (_, cols) = state.size();

    // Ground the state machine: from every parser state (escape, escape-
    // intermediate, CSI, control string, string-escape) the two bytes
    // `ESC \` end in Ground, and leaving Ground drops any pending partial
    // UTF-8 sequence, so the probes below always parse from a clean start.
    state.feed_bytes(b"\x1b\\");

    // Pen reset identity: whatever the fuzz stream left set, `SGR 0` and
    // the empty `SGR` form must return the pen to the exact default.
    state.feed_bytes(b"\x1b[0m");
    assert_eq!(
        *state.attributes(),
        CellAttributes::DEFAULT,
        "{context}: SGR 0 resets the pen"
    );
    state.feed_bytes(b"\x1b[m");
    assert_eq!(
        *state.attributes(),
        CellAttributes::DEFAULT,
        "{context}: empty SGR resets the pen"
    );

    // Print round-trip identity across the UTF-8 byte classes: ASCII, a
    // 2-byte Latin-1 character, a 3-byte CJK character, a 4-byte emoji,
    // and a combining mark attaching to a base character. A decoder that
    // drops or mangles any continuation byte fails its class immediately.
    print_round_trip(state, b"N", "N", "ASCII", context);
    print_round_trip(state, "é".as_bytes(), "é", "2-byte", context);
    if cols >= 2 {
        state.feed_bytes(b"\x1b[1;1H");
        state.feed_bytes("日".as_bytes());
        assert_eq!(
            state.screen().cell(0, 0).expect("probe cell").text(),
            "日",
            "{context}: 3-byte round-trip identity"
        );
        assert!(
            state
                .screen()
                .cell(0, 1)
                .expect("continuation")
                .is_continuation(),
            "{context}: CJK width-2 continuation cell"
        );
        state.feed_bytes(b"\x1b[1;1H");
        state.feed_bytes("😀".as_bytes());
        assert_eq!(
            state.screen().cell(0, 0).expect("probe cell").text(),
            "😀",
            "{context}: 4-byte round-trip identity"
        );
        assert!(
            state
                .screen()
                .cell(0, 1)
                .expect("continuation")
                .is_continuation(),
            "{context}: emoji width-2 continuation cell"
        );
    }
    // A zero-width combining mark attaches to the preceding cell's text
    // without occupying a column.
    state.feed_bytes(b"\x1b[1;1He");
    state.feed_bytes("\u{0301}".as_bytes());
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").text(),
        "e\u{0301}",
        "{context}: combining mark attaches"
    );

    // Pen set identity: a compound SGR (style flags plus indexed and
    // direct-color extended forms) must land in the pen exactly, and a
    // cell written under it must capture exactly that.
    let expected = CellAttributes::DEFAULT
        .with_bold(true)
        .with_underline(true)
        .with_reverse(true)
        .with_foreground(Color::Indexed(196))
        .with_background(Color::Rgb(10, 20, 30));
    state.feed_bytes(b"\x1b[1;4;7;38;5;196;48;2;10;20;30m");
    assert_eq!(
        *state.attributes(),
        expected,
        "{context}: compound SGR sets the pen"
    );
    state.feed_bytes(b"\x1b[1;1HN");
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").attributes(),
        &expected,
        "{context}: cell captures the pen"
    );
    // The colon sub-parameter form of extended color must parse too.
    state.feed_bytes(b"\x1b[38:2::1:2:3m");
    assert_eq!(
        state.attributes().foreground(),
        Color::Rgb(1, 2, 3),
        "{context}: colon-form extended color sets the pen"
    );
    // And `SGR 0` must clear it all through the cell path as well.
    state.feed_bytes(b"\x1b[0m\x1b[1;1HN");
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").attributes(),
        &CellAttributes::DEFAULT,
        "{context}: SGR 0 clears attributes on later cells"
    );
    // An SGR list overflowing the CSI parameter cap is dropped whole, so
    // the pen is untouched (still default here).
    let mut overflow = b"\x1b[".to_vec();
    overflow.extend(b"1;".repeat(40));
    overflow.push(b'm');
    state.feed_bytes(&overflow);
    assert_eq!(
        *state.attributes(),
        CellAttributes::DEFAULT,
        "{context}: overflowing SGR leaves the pen untouched"
    );

    // SCS final-byte non-leak: charset designators (and the DECALN-shaped
    // `ESC # 8`) must neither print their final byte nor move the cursor.
    state.feed_bytes(b"\x1b[1;1Hy");
    let before = (state.cursor(), state.is_wrap_pending());
    state.feed_bytes(b"\x1b(B\x1b)0\x1b#8");
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").text(),
        "y",
        "{context}: SCS final byte does not print"
    );
    assert_eq!(
        (state.cursor(), state.is_wrap_pending()),
        before,
        "{context}: SCS does not move the cursor"
    );

    // Tab-stop bounds: tabs clamp inside the current row and never leave
    // autowrap pending.
    state.feed_bytes(b"\x1b[1;1H\t\t\t\t");
    let cursor = state.cursor();
    assert_eq!(cursor.row(), 0, "{context}: tab keeps the row");
    assert!(cursor.column() < cols, "{context}: tab stays in the row");
    assert!(
        !state.is_wrap_pending(),
        "{context}: tab clears wrap pending"
    );

    // Alternate-screen consistency: the mode flag follows 1049 exactly, a
    // write while switched lands on the active (alternate) buffer, and the
    // primary buffer keeps its own content.
    state.feed_bytes(b"\x1b[?1049l");
    assert!(
        !state.modes().is_alternate_screen_active(),
        "{context}: 1049l leaves the alternate screen"
    );
    state.feed_bytes(b"\x1b[1;1HP");
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").text(),
        "P",
        "{context}: primary witness round-trip"
    );
    state.feed_bytes(b"\x1b[?1049h");
    assert!(
        state.modes().is_alternate_screen_active(),
        "{context}: 1049h enters the alternate screen"
    );
    state.feed_bytes(b"\x1b[1;1HZ");
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").text(),
        "Z",
        "{context}: alternate witness round-trip"
    );
    state.feed_bytes(b"\x1b[?1049l");
    assert!(
        !state.modes().is_alternate_screen_active(),
        "{context}: alternate screen left again"
    );
    assert_eq!(
        state.screen().cell(0, 0).expect("probe cell").text(),
        "P",
        "{context}: alternate-screen writes never touch the primary buffer"
    );

    // The probes themselves must leave the any-input oracles holding.
    assert_state_oracles(state, &format!("{context} (post-probe)"));
}

// ===== Seeded PRNG: same design as tests/soak_feed_bytes.rs =====
//
// splitmix64 seeding + xorshift64 stream: reproducible from a `u64` seed,
// no `rand`, no new dependency. Copied from the soak (both files are
// separate crates; keep the algorithms identical).

struct Xorshift64(u64);

impl Xorshift64 {
    fn from_seed(seed: u64) -> Self {
        let mut mixer = seed;
        let state = splitmix64(&mut mixer);
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

    fn below(&mut self, n: u32) -> u32 {
        (self.next_u64() % u64::from(n)) as u32
    }

    fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    fn pick<'a, T>(&mut self, slice: &'a [T]) -> &'a T {
        let idx = (self.next_u64() % slice.len() as u64) as usize;
        &slice[idx]
    }

    /// Pick one borrowed byte slice out of a corpus (`&[&[u8]]`). Separate
    /// from the generic [`pick`](Self::pick) so type inference never tries
    /// to treat `[u8]` itself as a `Sized` element (same as the soak).
    fn pick_bytes<'a>(&mut self, slice: &[&'a [u8]]) -> &'a [u8] {
        let idx = (self.next_u64() % slice.len() as u64) as usize;
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

/// Per-case PRNG. Deterministically derived from the run's root seed and the
/// case index, so `FUZZ_ROOT_SEED=<seed> FUZZ_CASE_INDEX=<i>` replays
/// exactly case `i` of a run rooted at `<seed>`, including its chunking and
/// mutation decisions (they all draw from this stream in a fixed order).
fn case_rng(root_seed: u64, case: u64) -> Xorshift64 {
    let mut salt = case;
    Xorshift64::from_seed(root_seed ^ splitmix64(&mut salt))
}

// ===== Real-sequence seed corpus =====
//
// Structured near-valid input is where parsers break, so the corpus is real
// program output, not random noise. Sources are cited per group; the
// malformed variants are DERIVED (deterministically, in the mutation sweep
// and by the generator's mutation pass) rather than hand-listed, so every
// corpus entry's mutations stay covered as the corpus grows.

/// Zellij 0.44.3 attach/detach wire shape. The empirical record in
/// `crates/noren-app/tests/zellij_live.rs` (single-parameter DECSETs for
/// 1002/1006 on attach, zero multi-parameter sequences) plus the pinned
/// client-mode evidence in `docs/compatibility/zellij.md` (1000/1002/1003/
/// 1015/1006 requested by the client; focus events 1004). The combined
/// `1002;1006` form is the PR #113 regression site pinned beside the live
/// evidence.
const CORPUS_ZELLIJ: &[&[u8]] = &[
    b"\x1b[?1049h",
    b"\x1b[?1h",
    b"\x1b=",
    b"\x1b[?2004h",
    b"\x1b[?1002h\x1b[?1006h",
    b"\x1b[?1002;1006h",
    b"\x1b[?1000h\x1b[?1003h\x1b[?1015h",
    b"\x1b[?1004h",
    b"\x1b[?1002l\x1b[?1006l",
    b"\x1b[?1049l\x1b[?2004l\x1b[?1l\x1b>",
];

/// vim: the mouse-mode enable/disable sets a `set mouse=a` vim emits, the
/// cursor visibility pair around startup/exit, and truecolor SGR. Cursor
/// save/restore via DECSC/DECRC plus ANSI Save/Restore.
const CORPUS_VIM: &[&[u8]] = &[
    b"\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h",
    b"\x1b[?1000l\x1b[?1002l\x1b[?1015l\x1b[?1006l",
    b"\x1b[?25l",
    b"\x1b[?12l\x1b[?25h",
    b"\x1b7\x1b[2;1H\x1b[8",
    b"\x1b[s\x1b[u",
    b"\x1b[38;2;255;100;0m\x1b[48;2;0;0;255m\x1b[m",
    b"\x1b[1m\x1b[4m\x1b[31m\x1b[42m\x1b[0m",
];

/// tmux: `mouse on` pairs, the DCS passthrough wrapper tmux uses to forward
/// sequences it does not rewrite itself, and a window-title OSC.
const CORPUS_TMUX: &[&[u8]] = &[
    b"\x1b[?1000h\x1b[?1006h",
    b"\x1b[?1000l\x1b[?1006l",
    b"\x1bPtmux;\x1b\x1b[?1049h\x1b\\",
    b"\x1b]0;tmux\x07",
];

/// DECSET/DECRST and query sequences beyond the mouse family: wrap (7),
/// cursor visibility (25), blink (12), the alternate-screen variants real
/// programs still emit (47/1047/1048), DECRQM (`$p`), and device queries.
/// The 9999/0/65535 entries are unknown modes that must be ignored cleanly.
const CORPUS_DECSET: &[&[u8]] = &[
    b"\x1b[?7h\x1b[?7l",
    b"\x1b[?25h\x1b[?25l",
    b"\x1b[?12h\x1b[?12l",
    b"\x1b[?47h\x1b[?47l",
    b"\x1b[?1047h\x1b[?1047l",
    b"\x1b[?1048h\x1b[?1048l",
    b"\x1b[?1006$p",
    b"\x1b[?1049$p",
    b"\x1b[>c",
    b"\x1b[>0q",
    b"\x1b[c",
    b"\x1b[6n",
    b"\x1b[?9999h",
    b"\x1b[?0h",
    b"\x1b[?65535l",
];

/// SGR including the forms the parser's parameter model has to survive:
/// empty, reset, colon sub-parameters, indexed and truecolor both sides,
/// saturating values, and a list that overflows the 32-slot parameter cap
/// (the whole sequence must be dropped, pen untouched).
const CORPUS_SGR: &[&[u8]] = &[
    b"\x1b[m",
    b"\x1b[0m",
    b"\x1b[38:5:196m",
    b"\x1b[38:2::255:100:0m",
    b"\x1b[48;2;10;20;30;58;2;1;2;3m",
    b"\x1b[999999999;1;4;999999999m",
];

/// Scroll-region (DECSTBM) shapes: full reset, valid window, inverted
/// window (must be rejected), zero params, and saturating params.
const CORPUS_DECSTBM: &[&[u8]] = &[
    b"\x1b[r",
    b"\x1b[2;10r",
    b"\x1b[10;2r",
    b"\x1b[0;0r",
    b"\x1b[99999;99999r",
];

/// OSC/DCS/SOS/PM/APC strings: BEL- and ST-terminated titles, the clipboard
/// (52) and zsh shell-integration (133) OSCs real shells emit, an
/// unterminated OSC, a long-but-bounded payload, and string introducers
/// other than OSC whose payloads must be swallowed whole.
const CORPUS_OSC: &[&[u8]] = &[
    b"\x1b]0;zellij\x07",
    b"\x1b]2;vim\x1b\\",
    b"\x1b]52;c;cGFzdGU=\x1b\\",
    b"\x1b]133;A\x1b\\",
    b"\x1b]0;unterminated-osc",
    b"\x1b]0;eseseseseseseseseseseseseseseseseseseseseseseseseseseseseses\x07",
    b"\x1bP1;2;3q payload \x1b\\",
    b"\x1b^pm-private\x07",
    b"\x1b_apc\x1b\\",
    b"\x1bXsos\x07",
];

/// Character-set selection and other multi-byte escapes: SCS with `(`,
/// `)`, `#` intermediates (the final byte must never leak as printable),
/// and the RIS reset.
const CORPUS_SCS: &[&[u8]] = &[b"\x1b(B", b"\x1b)0", b"\x1b#8", b"\x1b(B\x1b)0", b"\x1bc"];

/// Printable text and C0 controls including valid multibyte UTF-8, a
/// combining mark, invalid bytes (bare continuation, overlong, surrogate),
/// and NUL/DEL.
const CORPUS_TEXT: &[&[u8]] = &[
    b"hello world\r\n",
    b"\t\t\t",
    b"\x08\x08\x08",
    "e\u{0301}".as_bytes(),
    "日".as_bytes(),
    "😀".as_bytes(),
    &[0xff, 0xc0, 0xaf, 0xed, 0xa0, 0x80],
    b"\x00\x00\x00",
    b"\x7f\x7f",
];

/// The flattened corpus. A function (not a `const`) because `concat` is
/// not const-evaluable; the groups above stay separately citable.
fn real_sequence_corpus() -> Vec<&'static [u8]> {
    [
        CORPUS_ZELLIJ,
        CORPUS_VIM,
        CORPUS_TMUX,
        CORPUS_DECSET,
        CORPUS_SGR,
        CORPUS_DECSTBM,
        CORPUS_OSC,
        CORPUS_SCS,
        CORPUS_TEXT,
    ]
    .concat()
}

// ===== Near-valid sequence generator =====
//
// Each family synthesises one sequence a real terminal program could plaus
// emit, with parameter values drawn from the hostile corners (empty, zero,
// saturating u16, 9-digit overflow) alongside ordinary ones. A mutation
// pass then corrupts the result structure-aware-ed: truncation, byte
// flips, ESC/C0/BEL insertion, deletion, span duplication. The mixture is
// deliberately biased to NEAR-valid: most sequences parse fully, some are
// poisoned at one byte — the boundary where a parser's error recovery is
// weakest.

/// Hostile-but-plausible parameter values shared by the CSI families.
fn gen_param(rng: &mut Xorshift64) -> String {
    match rng.below(10) {
        0 => String::new(),
        1 => "0".to_owned(),
        2 => "1".to_owned(),
        3 => (2 + rng.below(80)).to_string(),
        4 => "255".to_owned(),
        5 => "256".to_owned(),
        6 => "65535".to_owned(),
        7 => "65536".to_owned(),
        8 => "999999999".to_owned(),
        _ => format!("0000000{}", 1 + rng.below(9)),
    }
}

/// Separator between parameters: mostly `;`, sometimes the ECMA-48
/// sub-parameter `:` that only SGR may carry.
fn gen_separator(rng: &mut Xorshift64) -> u8 {
    *rng.pick(b";;;;:")
}

/// Standard CSI: cursor movement, erase, edit, scroll, or CUP finals with
/// 0..=4 parameters, occasionally an embedded C0 control (executes without
/// aborting the sequence) or a rare poisoning intermediate byte.
fn gen_csi_standard(rng: &mut Xorshift64) -> Vec<u8> {
    let mut out = b"\x1b[".to_vec();
    let param_count = rng.below(5);
    for index in 0..param_count {
        if index > 0 {
            out.push(gen_separator(rng));
        }
        out.extend_from_slice(gen_param(rng).as_bytes());
        if rng.below(16) == 0 {
            out.push(*rng.pick(b"\n\r\t\x08\x18\x1a"));
        }
    }
    if rng.below(12) == 0 {
        out.push(*rng.pick(b"\x20\x21\x2f"));
    }
    out.push(*rng.pick(b"ABCDEFGHfdJK@PXLMST"));
    out
}

/// DECSET/DECRST: mostly the private modes this parser knows (plus unknown
/// and saturating ones), single- or multi-parameter, `h`/`l` finals, and
/// the `$p` query final. A non-`?` private marker (`<`, `=`, `>`) is
/// occasionally prepended to exercise marker poisoning.
fn gen_csi_private(rng: &mut Xorshift64) -> Vec<u8> {
    let mut out = b"\x1b[".to_vec();
    match rng.below(10) {
        0..=1 => out.push(*rng.pick(b"<>=")),
        _ => out.push(b'?'),
    }
    let mode_count = 1 + rng.below(3);
    for index in 0..mode_count {
        if index > 0 {
            out.push(b';');
        }
        out.extend_from_slice(
            match rng.below(10) {
                0..=5 => (*rng.pick(&[
                    1_u16, 7, 12, 25, 47, 1000, 1002, 1003, 1004, 1005, 1006, 1015, 1047, 1048,
                    1049, 2004,
                ]))
                .to_string(),
                6 => 9999.to_string(),
                7 => 65535.to_string(),
                _ => gen_param(rng),
            }
            .as_bytes(),
        );
    }
    let final_byte = *rng.pick(b"hlhl$");
    out.push(final_byte);
    if final_byte == b'$' {
        out.push(b'p');
    }
    out
}

/// DECSTBM with 0..=2 parameters biased to rows-range values, including
/// inverted windows and saturating bounds that must clamp or be rejected.
fn gen_decstbm(rng: &mut Xorshift64) -> Vec<u8> {
    let mut out = b"\x1b[".to_vec();
    let param_count = rng.below(3);
    for index in 0..param_count {
        if index > 0 {
            out.push(b';');
        }
        out.extend_from_slice(
            match rng.below(6) {
                0 | 1 => (1 + rng.below(40)).to_string(),
                2 => "0".to_owned(),
                3 => "1".to_owned(),
                4 => "65535".to_owned(),
                _ => gen_param(rng),
            }
            .as_bytes(),
        );
    }
    out.push(b'r');
    out
}

/// SGR: parameter lists from 0 to 40 entries (crossing the 32-slot cap) of
/// known codes, extended colors in both `;` and `:` forms, and hostile
/// values.
fn gen_sgr(rng: &mut Xorshift64) -> Vec<u8> {
    let mut out = b"\x1b[".to_vec();
    let param_count = rng.below(41);
    for index in 0..param_count {
        if index > 0 {
            out.push(gen_separator(rng));
        }
        out.extend_from_slice(
            match rng.below(6) {
                0 => (*rng.pick(&[0_u16, 1, 2, 4, 7, 22, 27, 31, 39, 49, 90, 97])).to_string(),
                1 => "38".to_string(),
                2 => "48".to_string(),
                3 => "58".to_string(),
                _ => gen_param(rng),
            }
            .as_bytes(),
        );
    }
    out.push(b'm');
    out
}

/// OSC/DCS/SOS/PM/APC: introducer, numeric terminator, a bounded payload
/// (printables, occasionally an embedded ESC or BEL to attack terminator
/// detection), and BEL/ST/no terminator.
fn gen_osc(rng: &mut Xorshift64) -> Vec<u8> {
    let mut out = vec![0x1b, *rng.pick(b"]PX^_")];
    if rng.next_bool() {
        out.extend_from_slice((rng.below(200)).to_string().as_bytes());
        out.push(b';');
    }
    let payload_len = rng.below(48) as usize;
    for _ in 0..payload_len {
        match rng.below(16) {
            0 => out.push(0x1b),
            1 => out.push(0x07),
            _ => out.push(b'a' + (rng.below(26) as u8)),
        }
    }
    match rng.below(3) {
        0 => out.push(0x07),
        1 => out.extend_from_slice(b"\x1b\\"),
        _ => {}
    }
    out
}

/// Multi-byte escapes with intermediate bytes (SCS and friends) plus the
/// one-byte escapes the parser acts on (DECSC/DECRC, IND/RI/NEL, keypad).
fn gen_escape(rng: &mut Xorshift64) -> Vec<u8> {
    if rng.next_bool() {
        let mut out = b"\x1b".to_vec();
        out.push(*rng.pick(b"()#% "));
        if rng.next_bool() {
            out.push(*rng.pick(b"()# "));
        }
        out.push(b' ' + (rng.below(0x5e) as u8));
        out
    } else {
        b"\x1b"
            .to_vec()
            .into_iter()
            .chain([*rng.pick(b"78DEM=>cH")])
            .collect()
    }
}

/// Printable runs of ASCII and valid/invalid UTF-8 interleaved with C0
/// controls.
fn gen_text(rng: &mut Xorshift64) -> Vec<u8> {
    let mut out = Vec::new();
    let parts = 1 + rng.below(6);
    for _ in 0..parts {
        match rng.below(8) {
            0 => out.push(*rng.pick(b"\n\r\t\x08\x0b\x0c\x00\x18")),
            1 => out.extend_from_slice(rng.pick_bytes(&[
                "日".as_bytes(),
                "e\u{0301}".as_bytes(),
                "😀".as_bytes(),
            ])),
            2 => out.extend_from_slice(rng.pick_bytes(&[
                [0xff].as_slice(),
                [0xc0, 0xaf].as_slice(),
                [0xed, 0xa0, 0x80].as_slice(),
            ])),
            _ => {
                let run = 1 + rng.below(8);
                for _ in 0..run {
                    out.push(b'!' + (rng.below(90) as u8));
                }
            }
        }
    }
    out
}

/// One synthesised sequence: pick a family, generate, optionally mutate.
fn gen_sequence(rng: &mut Xorshift64) -> Vec<u8> {
    let mut bytes = match rng.below(12) {
        0..=3 => gen_csi_standard(rng),
        4..=5 => gen_csi_private(rng),
        6 => gen_decstbm(rng),
        7..=8 => gen_sgr(rng),
        9 => gen_osc(rng),
        10 => gen_escape(rng),
        _ => gen_text(rng),
    };
    let mutations = rng.below(3);
    for _ in 0..mutations {
        mutate(rng, &mut bytes);
    }
    bytes
}

/// Structure-aware mutation of a byte slice. Every operator keeps the
/// result bounded (duplications append at most 16 bytes).
fn mutate(rng: &mut Xorshift64, bytes: &mut Vec<u8>) {
    if bytes.is_empty() {
        bytes.push(0x1b);
        return;
    }
    let position = (rng.next_u64() % bytes.len() as u64) as usize;
    match rng.below(6) {
        0 => {
            bytes.truncate(position);
        }
        1 => {
            let mask = *rng.pick(&[0x01_u8, 0x20, 0x80]);
            bytes[position] ^= mask;
        }
        2 => {
            bytes[position] = *rng.pick(&[0x1b, 0x07, b'[', b';', b'?', 0xff, 0x00]);
        }
        3 => {
            bytes.insert(
                position,
                *rng.pick(&[0x1b, 0x07, b'[', b';', b':', b'?', 0xff]),
            );
        }
        4 => {
            bytes.remove(position);
        }
        _ => {
            let span = 1 + rng.below(16.min(bytes.len()) as u32) as usize;
            let start = position.min(bytes.len() - span.min(bytes.len()));
            let end = (start + span).min(bytes.len());
            let chunk = bytes[start..end].to_vec();
            bytes.extend_from_slice(&chunk);
        }
    }
}

// ===== Case runner =====

/// How one case feeds its stream into the terminal under test.
enum FeedMode {
    /// Random chunk boundaries: the realistic PTY read pattern.
    Chunks(Vec<usize>),
    /// One byte per `feed_bytes` call: parser boundary recovery.
    ByteAtATime,
}

/// Everything needed to replay one case, computed before any feeding so a
/// failure report can print the exact input and chunking.
struct CasePlan {
    stream: Vec<u8>,
    feed: FeedMode,
    /// Some((rows, cols)) resizes to apply interleaved between feeds.
    resizes: Vec<(u16, u16)>,
}

fn plan_case(rng: &mut Xorshift64) -> CasePlan {
    let sequence_count = 1 + rng.below(8);
    let mut stream = Vec::new();
    let mut resizes = Vec::new();
    for _ in 0..sequence_count {
        stream.extend_from_slice(&gen_sequence(rng));
        if rng.below(12) == 0 {
            resizes.push((1 + rng.below(12) as u16, 1 + rng.below(24) as u16));
        }
    }
    if stream.is_empty() {
        stream.push(b'A');
    }
    let feed = if rng.below(10) < 3 {
        FeedMode::ByteAtATime
    } else {
        let chunk_count = 1 + rng.below(6) as usize;
        let mut cuts: Vec<usize> = (0..chunk_count)
            .map(|_| (rng.next_u64() % (stream.len() as u64 + 1)) as usize)
            .collect();
        cuts.push(0);
        cuts.push(stream.len());
        cuts.sort_unstable();
        cuts.dedup();
        FeedMode::Chunks(cuts)
    };
    CasePlan {
        stream,
        feed,
        resizes,
    }
}

/// Outcome of one case: pass, or the failing bytes plus the panic message.
struct CaseOutcome {
    passed: bool,
    stream: Vec<u8>,
    message: Option<String>,
}

/// Execute one case: fresh tiny terminal, feed the planned stream in the
/// planned mode, assert the shape oracle and the any-input state oracles
/// after EVERY chunk (and every resize), then run the deterministic
/// content probes on the terminal the stream left behind, and catch any
/// panic. Tiny grids keep the sweep fast and far below the cell cap.
fn run_case(root_seed: u64, case: u64, trace: bool) -> CaseOutcome {
    let mut rng = case_rng(root_seed, case);
    let plan = plan_case(&mut rng);
    let rows = 1 + rng.below(8) as u16;
    let cols = 1 + rng.below(16) as u16;
    let mut state = TerminalState::new(rows, cols).expect("valid initial terminal");
    assert_invariants(&state, "init");

    let context = format!("fuzz root_seed={root_seed:#018x} case={case}");
    if trace {
        eprintln!(
            "[fuzz] {context} stream={:?} feed={}",
            plan.stream,
            plan.feed.feed_mode_label()
        );
    }

    // Bound each resize's application point: resizes apply before every
    // `resizes.len()`-th feed boundary, deterministic from the plan.
    let mut resize_queue = plan.resizes.iter();

    let mut feed_and_check = |state: &mut TerminalState, chunk: &[u8], label: &str| {
        if let Some((rows, cols)) = resize_queue.next().copied() {
            let _ = state.resize(rows, cols);
            assert_invariants(state, &format!("{context} resize {label}"));
            assert_state_oracles(state, &format!("{context} resize {label}"));
        }
        state.feed_bytes(chunk);
        assert_invariants(state, &format!("{context} feed {label}"));
        assert_state_oracles(state, &format!("{context} feed {label}"));
    };

    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        match &plan.feed {
            FeedMode::ByteAtATime => {
                for (index, byte) in plan.stream.iter().enumerate() {
                    feed_and_check(
                        &mut state,
                        std::slice::from_ref(byte),
                        &format!("byte {index}"),
                    );
                }
            }
            FeedMode::Chunks(cuts) => {
                for window in cuts.windows(2) {
                    let chunk = &plan.stream[window[0]..window[1]];
                    feed_and_check(
                        &mut state,
                        chunk,
                        &format!("chunk {}/{}", window[0], window[1]),
                    );
                }
            }
        }
        // Content probes run after the hostile stream on the SAME terminal,
        // so every case also asserts decode/pen/buffer content identity
        // over whatever state the fuzz stream left behind.
        run_content_probes(&mut state, &context);
    }));

    CaseOutcome {
        passed: outcome.is_ok(),
        stream: plan.stream,
        message: outcome.err().map(|payload| {
            if let Some(text) = payload.downcast_ref::<&str>() {
                (*text).to_owned()
            } else if let Some(text) = payload.downcast_ref::<String>() {
                text.clone()
            } else {
                "panic with a non-string payload".to_owned()
            }
        }),
    }
}

impl FeedMode {
    fn feed_mode_label(&self) -> &'static str {
        match self {
            Self::Chunks(_) => "chunks",
            Self::ByteAtATime => "byte-at-a-time",
        }
    }
}

/// Render bytes as a Rust `b"..."` literal suitable for pasting straight
/// into a regression test.
fn escaped_literal(bytes: &[u8]) -> String {
    let mut out = String::from("b\"");
    for &byte in bytes {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            0x20..=0x7e => out.push(byte as char),
            _ => out.push_str(&format!("\\x{byte:02x}")),
        }
    }
    out.push('"');
    out
}

/// Run `cases` generated cases; on the first failure panic with a complete
/// repro: root seed, case index, the exact bytes, and the original message.
fn run_generated_cases(root_seed: u64, cases: u64) {
    for case in 0..cases {
        let outcome = run_case(root_seed, case, false);
        if !outcome.passed {
            panic!(
                "FUZZ DEFECT: generated case failed\n  root_seed={root_seed:#018x} case={case}\n  \
                 message: {}\n  bytes: {}\n  replay with: FUZZ_ROOT_SEED={root_seed:#x} \
                 FUZZ_CASE_INDEX={case} cargo test -p noren-terminal --test fuzz_feed_bytes \
                 -- --nocapture",
                outcome.message.unwrap_or_default(),
                escaped_literal(&outcome.stream),
            );
        }
    }
}

// ===== Environment controls =====

/// Fixed default seed so the committed run is a stable, always-green
/// baseline; any change to it must keep the test green.
const DEFAULT_ROOT_SEED: u64 = 0xF00F_BEEF_5EED_0A11;

/// Bounded default case count for `cargo test`. Measured on macOS arm64
/// debug at roughly 0.5 ms/case, so 2500 cases lands around 1.5 s — the
/// committed bound. `FUZZ_CASES` overrides it for longer local campaigns.
const DEFAULT_CASES: u64 = 2500;

fn env_u64(name: &str) -> Option<u64> {
    let raw = std::env::var(name).ok()?;
    // Underscores are accepted (a `0xC0FF_EE42`-style seed must not
    // silently fall back to the default — a failed replay is worse than a
    // failed parse).
    let trimmed = raw.trim().replace('_', "");
    if let Some(hex) = trimmed.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse().ok()
    }
}

fn fuzz_root_seed() -> u64 {
    env_u64("FUZZ_ROOT_SEED").unwrap_or(DEFAULT_ROOT_SEED)
}

// ===== Tests =====

/// The default bounded fuzz run: 2500 generated near-valid streams, each
/// fed whole-chunked or byte-at-a-time into a fresh tiny terminal with the
/// invariant oracle asserted after every chunk and interleaved resize.
#[test]
fn fuzz_feed_bytes_generated_streams() {
    if let Some(case) = env_u64("FUZZ_CASE_INDEX") {
        eprintln!("[fuzz] replaying single case {case} with tracing");
        let outcome = run_case(fuzz_root_seed(), case, true);
        assert!(
            outcome.passed,
            "FUZZ DEFECT (replayed case {case}): {} bytes: {}",
            outcome.message.unwrap_or_default(),
            escaped_literal(&outcome.stream),
        );
        return;
    }
    let cases = env_u64("FUZZ_CASES").unwrap_or(DEFAULT_CASES);
    run_generated_cases(fuzz_root_seed(), cases);
}

/// Systematic corpus mutation sweep — deterministic, no randomness: every
/// corpus entry is fed whole and byte-at-a-time, and every TRUNCATION
/// (prefix), byte FLIP (three masks), SUBSTITUTION (seven hostile bytes),
/// and INSERTION (seven hostile bytes) of every entry is fed through
/// `feed_bytes` on a fresh terminal with the oracle asserted after. This
/// guarantees each corpus entry's malformed neighbourhood is covered, not
/// sampled. The shape/structural oracles run on every variant; the content
/// probe suite (see [`run_content_probes`]) runs on every original and on a
/// deterministic one-in-eight lattice of the mutations — the probes
/// re-establish their own preconditions, so their sensitivity does not
/// depend on which variant preceded them, and every GENERATED case carries
/// the full probe suite anyway.
#[test]
fn fuzz_feed_bytes_seed_corpus_mutation_sweep() {
    let check = |bytes: &[u8], provenance: &str, probes: bool| {
        let mut state = TerminalState::new(4, 8).expect("valid terminal");
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            state.feed_bytes(bytes);
            assert_invariants(&state, provenance);
            assert_state_oracles(&state, provenance);
            for byte in bytes {
                state.feed_bytes(std::slice::from_ref(byte));
            }
            assert_invariants(&state, &format!("{provenance} (byte-at-a-time)"));
            assert_state_oracles(&state, &format!("{provenance} (byte-at-a-time)"));
            if probes {
                run_content_probes(&mut state, provenance);
            }
        }));
        if outcome.is_err() {
            panic!(
                "FUZZ DEFECT: corpus mutation sweep failed\n  {provenance}\n  bytes: {}",
                escaped_literal(bytes),
            );
        }
    };

    let mut variant = 0_u64;

    for (entry_index, entry) in real_sequence_corpus().iter().enumerate() {
        let base = format!("corpus entry {entry_index}");
        check(entry, &format!("{base} original"), true);

        for end in 0..entry.len() {
            variant += 1;
            check(
                &entry[..end],
                &format!("{base} truncated to {end}"),
                variant % 8 == 0,
            );
        }
        for position in 0..entry.len() {
            for mask in [0x01_u8, 0x20, 0x80] {
                let mut flipped = entry.to_vec();
                flipped[position] ^= mask;
                variant += 1;
                check(
                    &flipped,
                    &format!("{base} byte {position} ^ {mask:#04x}"),
                    variant % 8 == 0,
                );
            }
            for replacement in [0x1b_u8, 0x07, b'[', b';', b':', 0xff, 0x00] {
                let mut substituted = entry.to_vec();
                substituted[position] = replacement;
                variant += 1;
                check(
                    &substituted,
                    &format!("{base} byte {position} -> {replacement:#04x}"),
                    variant % 8 == 0,
                );
            }
        }
        for position in 0..=entry.len() {
            for insertion in [0x1b_u8, 0x07, b'[', b';', b':', 0xff, 0x00] {
                let mut inserted = entry.to_vec();
                inserted.insert(position, insertion);
                variant += 1;
                check(
                    &inserted,
                    &format!("{base} inserted {insertion:#04x} at {position}"),
                    variant % 8 == 0,
                );
            }
        }
    }
}

/// Time-bounded campaign door, OFF by default so `cargo test --workspace`
/// stays fast: when `FUZZ_SECONDS` is unset this returns immediately after
/// the bounded default run. When set (e.g. `FUZZ_SECONDS=60`), it keeps
/// generating fresh cases from the root seed until the deadline, then
/// prints iterations and total bytes fed (visible with `-- --nocapture`).
/// Determinism note: the SEQUENCE of cases is a pure function of the root
/// seed, so any case a campaign finds replays exactly via
/// `FUZZ_ROOT_SEED=... FUZZ_CASE_INDEX=<n>` regardless of the deadline.
#[test]
fn fuzz_feed_bytes_campaign() {
    let Some(seconds) = env_u64("FUZZ_SECONDS") else {
        // Off in normal `cargo test`: the bounded default run lives in
        // fuzz_feed_bytes_generated_streams above; this test adds nothing.
        return;
    };
    let root_seed = fuzz_root_seed();
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut case = 0_u64;
    let mut bytes_fed = 0_u64;
    while Instant::now() < deadline {
        let outcome = run_case(root_seed, case, false);
        if !outcome.passed {
            panic!(
                "FUZZ DEFECT (campaign at case {case}): {}\n  bytes: {}\n  replay with: \
                 FUZZ_ROOT_SEED={root_seed:#x} FUZZ_CASE_INDEX={case} cargo test -p noren-terminal \
                 --test fuzz_feed_bytes fuzz_feed_bytes_campaign -- --nocapture",
                outcome.message.unwrap_or_default(),
                escaped_literal(&outcome.stream),
            );
        }
        bytes_fed += outcome.stream.len() as u64;
        case += 1;
    }
    println!(
        "[fuzz] campaign done: {case} cases ({bytes_fed} bytes fed) in {seconds}s, \
         root_seed={root_seed:#018x}, zero panics, zero invariant violations",
    );
}
