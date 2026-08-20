# Handoff — Q2 seeded soak harness over `feed_bytes` (`glm-q2`)

Status: merge candidate. Updated 2026-08-07.

## Purpose

The threat model (TM-04) names `TerminalState::feed_bytes` as the untrusted-input
boundary and asks for a "fuzz/no-panic target"; `docs/testing/strategy.md`
requires "at least 100 rapid resize/input/output interleavings and preserve the
smallest failing seed as a regression fixture". Neither existed: the only
hostile-input coverage was `tests/adversarial.rs` (23 tests) and
`tests/adversarial_kimi.rs` (20 tests), both hand-written, single-sequence
sweeps. This lane adds the missing piece — **randomised interleaving** — as a
single new test file, with no new dependency and no nightly toolchain.

## File lease (exactly one file)

- `crates/noren-terminal/tests/soak_feed_bytes.rs` (new).

No source file, `Cargo.toml`, `Cargo.lock`, or other test file was touched.
`rust-toolchain.toml` still pins stable 1.88.0; `cargo-fuzz`/nightly were
deliberately avoided.

## What the harness does

A small seeded PRNG (splitmix64 to seed xorshift64 — both public-domain, std-
only) drives randomised interleavings of the five boundary operations named in
the strategy:

- `feed_bytes` — from a hostile corpus (CSI with saturating params, OSC,
  invalid UTF-8, wide/combining characters, scroll storms, edit ops, aborted
  sequences, bare printables);
- `resize` (public API, tiny grids 1..=12 × 1..=16);
- scroll-region changes via CSI DECSTBM and via the public `set_scroll_region`;
- alternate-screen toggles (`?1049h`/`?1049l`); and
- other DEC private mode / keypad switches (`?1`, `?2004`, `=`, `>`).

A `FeedByteAtATime` action additionally feeds a corpus entry one byte per call
to stress partial-sequence recovery across `feed_bytes` boundaries, which
whole-feed never exercises.

The **public invariants are asserted after every step**. The oracle is copied
verbatim from `tests/adversarial.rs::assert_invariants` (each test file is its
own crate, so it cannot be imported). The only failure mode is a panic or an
invariant breach; on failure the panic names the seed and step, and setting the
`SOAK_SEED` env var replays exactly that seed with per-step tracing
(seed/size/cursor/region/wrap/action) so the case can be pinned as a permanent
`#[ignore = "reproduces <id>"]` regression.

## Iteration bounds (the documented answer)

- `STEPS_PER_SEED = 100` — the strategy's named floor.
- `SEED_SWEEP_COUNT = 8192` seeds → **819 200** randomised steps in the sweep,
  plus the default-seed test's 100 → **819 300** total randomised steps per run.
- Grids stay tiny (≤ 12 × 16) so cell allocation never dominates; measured at
  ~0.3 ms/seed on macOS arm64 debug.

## Wall time

`cargo test --package noren-terminal --test soak_feed_bytes` runs both tests in
**~1.7–2.4 s** wall (timed repeatedly with `/usr/bin/time -p`; `real` 1.7–2.4 s
across runs, stable around 2 s). This is well under the 10-second budget and
does not add a second long test next to the existing 94-second one.

## Gate (real output)

On `agent/soak-feed-bytes`, macOS arm64, rustc 1.88.0 (stable):

```
$ cargo fmt --all -- --check            → exit 0 (clean)
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile; exit 0, 0 warnings
$ cargo test --workspace                → exit 0
    390 passed; 0 failed; 1 pre-existing ignored
```

Baseline before this lane was 388 passed; the new file adds 2 tests
(`soak_feed_bytes_default_seed_100_interleavings`,
`soak_feed_bytes_seed_sweep`), so 390 reconciles exactly. The single ignored
test is pre-existing and unrelated.

The two new tests:

```
test soak_feed_bytes_default_seed_100_interleavings ... ok
test soak_feed_bytes_seed_sweep ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured
```

## Defects found

**0.** All 8192 swept seeds (819 200 randomised interleavings) held the public
invariants with no panic. This is the expected outcome: the invariants
(bounded grid, cursor in bounds, ordered scroll region within the screen) are
robust by construction, and the adversarial suites already pressure the parser
with hand-written hostile sequences. The value delivered here is the
randomised-interleaving coverage that neither hand-written suite provides.

If a future change regresses `feed_bytes`/`resize`/`set_scroll_region`, the
sweep will fail and print the offending seed; pin it with
`SOAK_SEED=<seed> cargo test … --nocapture`, then add an
`#[ignore = "reproduces <id>"]` test that replays it.

## Constraints honoured

- **Zero new dependencies.** xorshift64 + splitmix64 are ~25 lines of std code;
  no `rand`, no `cargo-fuzz`, nothing added to any manifest.
- **Stable only.** No nightly feature is used; `catch_unwind`/`AssertUnwindSafe`
  are stable std.
- **No duplication.** The corpus and the randomised-interleaving driver are
  net-new; the invariant oracle is explicitly copied (not reinvented) from
  `adversarial.rs`, with a comment forbidding divergence.
- **Under 10 s.** Measured ~2 s; iteration count is a named, tunable constant.

## Authorship / conflict of interest

I (GLM `glm-q2`) authored the new test file and this handoff. No independent
review is recorded here; per the development model an independent reviewer must
cover the current head before merge. A different lane owns any fix should the
soak later surface a genuine defect — this lane only adds the harness and, by
contract, does not fix defects it finds.

## Resume instructions

1. `git checkout agent/soak-feed-bytes`.
2. Re-run the gate: `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` (expect 390 passed / 1 ignored).
3. To exercise the pinning workflow on a known-good seed:
   `SOAK_SEED=0x5050c0de cargo test --package noren-terminal --test
   soak_feed_bytes -- --nocapture`.
4. To widen coverage, raise `SEED_SWEEP_COUNT` (each doubling adds ~2 s on this
   machine); keep the whole file comfortably under 10 s.
