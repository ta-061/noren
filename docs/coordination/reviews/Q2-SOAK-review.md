# Independent review — Q2 seeded soak harness over `feed_bytes`

Status: current merge-candidate review record. Updated 2026-08-14.

- Branch: `agent/soak-feed-bytes`
- Head SHA: `334f8a72813383d76f59017fe9ac3f7880207f16`
- Base: `origin/main` at review time (`309c0b4`, PR #131); the branch diverged at
  `91a0536` (PR #75) and is 54 commits behind current main.
- Author handoff: `docs/coordination/handoffs/glm-q2.md`
- Reviewer: independent (qwen-rv1-q2-soak lane); did not author the code.

## Spec provenance note

`state/tasks/Q2-SOAK.md` does not exist in the fleet repo
(`noren-fleet-private/state/tasks/` contains only the M3 task files and
TEMPLATE). The acceptance criteria below are taken from the task prompt the
author was given, `noren-fleet-private/prompts/glm-q2-soak.md`, which the
review brief treats as the authority. This is a process gap, not an author
defect; it changes none of the criteria.

## Gate (run by the reviewer, real output)

`rustc 1.88.0 (6b00bc388 2025-06-23)`, macOS arm64, on the branch head:

```
$ cargo fmt --all -- --check
(exit 0, no output)

$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-terminal v0.1.0 (...pool-q2/crates/noren-terminal)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.85s
(exit 0; re-run after `touch`ing the new test file to defeat a stale cache)

$ cargo test --workspace
(exit 0)
test soak_feed_bytes_default_seed_100_interleavings ... ok
test soak_feed_bytes_seed_sweep ... ok
totals across all test targets: 390 passed; 0 failed; 1 ignored
wall: real 73.94 s (whole workspace, parallel test binaries)
```

The single ignored test is `noren_app`'s pre-existing
`clipboard::tests::system_clipboard_round_trips_user_text ... ignored, touches
the real macOS system clipboard` — unrelated to this branch. 390 = the
handoff's claimed 388 baseline + the 2 new tests; it reconciles.

Soak suite in isolation, timed:

```
$ /usr/bin/time -p cargo test --package noren-terminal --test soak_feed_bytes
test result: ok. 2 passed; 0 failed; 0 ignored; ... finished in 1.82s
real 2.00
```

## Acceptance criteria (from the task prompt), one by one

1. **Small seeded PRNG, deterministic and reproducible** — MET.
   splitmix64-seeded xorshift64, std-only (`soak_feed_bytes.rs:68-124`).
   Verified empirically: `SOAK_SEED=0x2a` replayed twice produced
   byte-identical `[soak]` trace lines (same md5,
   `deda98b6f0b4a8597c8bb061d863d099`).
2. **Randomised interleavings of feed_bytes, resize, mode switches,
   scroll-region changes, alternate-screen toggles** — MET. All five are in
   `ACTION_TABLE` (`soak_feed_bytes.rs:197-209`), plus two extra stressors the
   spec did not require: byte-at-a-time feeding across `feed_bytes` boundaries
   and public `move_cursor` calls.
3. **Assert public invariants after every step, reusing the
   `adversarial.rs` oracle** — MET. `run_seed` asserts after init and after
   every step (`soak_feed_bytes.rs:286, 300-303`). The oracle was diffed
   against `tests/adversarial.rs::assert_invariants` on both the branch base
   and current `origin/main`: byte-identical, not reinvented. (Each test file
   is its own crate, so a copy is the only lease-compliant reuse; the
   divergence warning at `soak_feed_bytes.rs:39-41` is the right mitigation.)
4. **On failure print the seed and pin the case** — MET, demonstrated, not
   just claimed. During the reviewer's mutation runs (below) the panic output
   named seed and step
   (`seed=0x000000005050c0de step=16 action=CursorMove: cursor row in
   bounds`) and the sweep printed
   `SOAK DEFECT: seed=0x0000000000000000 ... reproduce with: SOAK_SEED=0x0
   cargo test --package noren-terminal --test soak_feed_bytes
   soak_feed_bytes_seed_sweep -- --nocapture`. The `SOAK_SEED` replay path
   runs with per-step tracing. No pinned `#[ignore]` regression exists, which
   is correct: 0 defects were found, and the spec only requires pinning when
   a defect is found.
5. **Zero new dependencies** — MET. `git diff --stat origin/main...HEAD`
   touches exactly 2 files (+492/−0); no `Cargo.toml`, `Cargo.lock`, or
   `rust-toolchain.toml` change. No `rand`, no nightly.
6. **Under 10 seconds; bound the iteration count and say what it is** — MET.
   Measured 2.00 s wall for both soak tests. Bounds are named constants:
   `STEPS_PER_SEED = 100`, `SEED_SWEEP_COUNT = 8192` → 819 300 randomised
   steps/run (`soak_feed_bytes.rs:308-315`), documented in file header and
   handoff.
7. **Do not duplicate the existing adversarial suites; value = randomised
   interleaving** — MET. The corpus and the interleaving driver are net-new;
   neither `adversarial.rs` (fixed single sequences) nor `adversarial_kimi.rs`
   randomises or interleaves. The file-header rationale
   (`soak_feed_bytes.rs:1-8`) matches what I verified in both suites.
8. **File lease** — MET. New file `crates/noren-terminal/tests/soak_feed_bytes.rs`
   plus the required `docs/coordination/handoffs/glm-q2.md` (which did not
   exist on `origin/main`; no shadowing). Nothing else touched.
9. **Commit `-s`, do not push** — MET. The sole commit carries
   `Signed-off-by: ta-061`; `git ls-remote origin
   refs/heads/agent/soak-feed-bytes` returns nothing (branch not pushed).

## Beyond the checklist — what I ran to try to break it

### Interaction the author never ran: the harness × current main

The author built against `91a0536`; `origin/main` has since gained 54 commits
including substantial `noren-terminal` changes (`src/state.rs` +237,
`src/parser.rs` +117, new mouse/bracketed-paste suites). A clean textual
merge can still be semantically wrong, so I ran the combination:

```
$ git checkout -b review/q2-soak-merge-trial && git merge origin/main --no-edit
(clean, no conflicts)
$ cargo test --package noren-terminal
(all targets ok, including: 2 passed; ... finished in 2.38s  <- the soak suite
 against the NEW parser/state implementation)
$ cargo test --workspace
workspace: passed=863 failed=0 ignored=4
```

The soak harness compiles unchanged and passes against the current-main
implementation (all APIs it uses — `new`, `feed_bytes`, `resize`,
`set_scroll_region`, `move_cursor`, `size`, `cursor`, `screen`,
`scroll_region`, `is_wrap_pending`, `CursorMove`, `MAX_SCREEN_CELLS` — exist
there, and main's `assert_invariants` is still identical to the copied
oracle). Merging this branch will not break CI. Scratch branch deleted after
the trial; the deliverable branch is unmodified.

### Do the tests actually test behaviour? (mutation testing)

Two independent one-line mutations to `crates/noren-terminal/src/state.rs`
(reverted afterwards with `git checkout --`; branch tree verified clean):

- **M1 — remove the cursor clamp** (`CursorMove::To { row, column } => {
  self.active.cursor.row = row; ... }`, was `.min(last_row)`):
  both soak tests FAIL. Default-seed test via the oracle:
  `seed=0x000000005050c0de step=16 action=CursorMove: cursor row in bounds`;
  sweep via `catch_unwind`: `SOAK DEFECT: seed=0x0000000000000000 ...`.
  Result: `test result: FAILED. 0 passed; 2 failed`.
- **M2 — let the scroll region escape the screen**
  (`ScrollRegion::checked`: `bottom.min(last_row + 1)`, was `.min(last_row)`):
  the sweep FAILS at seed 0 with the same SOAK DEFECT attribution.
  Result: `test result: FAILED. 1 passed; 1 failed` (the fixed default seed
  happens not to reach the region bound within its 100 steps; the sweep
  covers it — this is expected behaviour for a seeded suite, not a hole).

The oracle is live on at least two independent invariant classes; a broken
implementation cannot pass silently.

### Hostile/degenerate input, panics, leaks, unbounded growth

- The corpus already includes invalid UTF-8 (`0xff 0xc0 0xaf 0xed 0xa0 0x80`),
  aborted CSI/OSC, saturating parameters, combining/wide chars, and 1-column
  grids appear in trace output (`size=(5, 1)` for seed 0x2a). All 819 300
  steps held the invariants.
- The harness itself allocates nothing that grows: one fresh `TerminalState`
  per seed, dropped each iteration; no threads, files, or channels; the only
  per-step allocation is the temporary `format!` context strings.
- The product structures on the exercised paths are bounded: grid by
  `MAX_SCREEN_CELLS`, scrollback enforced at `state.rs:1042` against
  `MAX_SCROLLBACK_LINES = 10_000`. No new growth surface is introduced.
- Degenerate env handling: `SOAK_SEED=not-hex` fails loudly with a clear
  panic rather than silently skipping (`test result: FAILED`), which is the
  safe direction.

### Noren/Zellij boundary (ADR 0003)

No violation. The change adds one test file driving `TerminalState`'s public
byte/grid API. It introduces no pane, tab, layout tree, or split, and neither
reads nor persists anything Zellij-internal. ADR 0003 lives at
`docs/adr/0003-noren-zellij-responsibility-boundary.md` on this branch (main
has since moved to `docs/coordination/decisions/`); the boundary analysis is
the same either way.

### Unintended deletions

`git diff --stat origin/main...HEAD`:

```
 crates/noren-terminal/tests/soak_feed_bytes.rs | 359 +++++++++++++++++++++++++
 docs/coordination/handoffs/glm-q2.md           | 133 +++++++++
 2 files changed, 492 insertions(+)
```

Nothing deleted, nothing modified outside the lease.

## Findings

No BLOCKER. No MAJOR.

### MINOR-1 — handover overstates what the threat model says

`docs/coordination/handoffs/glm-q2.md:7-8` and
`soak_feed_bytes.rs:1-3` say the threat model "names
`TerminalState::feed_bytes` as the untrusted-input boundary". TM-04
(`docs/security/threat-model.md:26`) actually names the consequence class and
asks for a "fuzz/no-panic target"; the symbol `feed_bytes` appears nowhere in
the threat model (the task prompt used the same loose phrasing, so the author
inherited it). The *substance* is right — this harness is that fuzz/no-panic
target — only the phrasing overstates the evidence.
Fix: reword to "the boundary TM-04's fuzz/no-panic target applies to".

### MINOR-2 — `SOAK_SEED` parsing rejects the `0X` prefix

`soak_feed_bytes.rs:341`: `trim_start_matches("0x")` is case-sensitive, so
`SOAK_SEED=0XC0FFEE` panics with "not a hex u64" (clearly, at least).
Reproduction: `SOAK_SEED=0X2a cargo test -p noren-terminal --test
soak_feed_bytes soak_feed_bytes_seed_sweep`. Expected: replay seed 0x2a.
Actual: test fails on parse. Fix: `trim_start_matches(["0x", "0X"])` or parse
case-insensitively.

### MINOR-3 — `u16_below` silently truncates domains above u16::MAX

`soak_feed_bytes.rs:96-98, 112-114`: indices are taken `as u16`, so any
future corpus/table larger than 65 535 entries would silently draw only from
the first 65 535 — a coverage loss with no diagnostic, in a file future lanes
are explicitly invited to extend. Latent only; every current call site is a
small constant. Fix: `(self.next_u64() % n as u64) as usize` on the full
domain.

## Genuinely sound areas

The splitmix64→xorshift64 seeding (seed 0 handled via `| 1`), the
per-seed `catch_unwind` attribution, the documented iteration bounds, the
byte-at-a-time partition stress, the lease discipline, and the sign-off /
no-push protocol are all correct as delivered.

## Verdict

All acceptance criteria met with reviewer-reproduced evidence; gates green on
the branch head (390 passed / 0 failed / 1 pre-existing ignored); the suite is
not vacuous (two independent mutations caught); the one combination the author
could not have run (harness × current main) merges clean and passes (863/0/4);
no deletions, no dependency or toolchain changes, no ADR 0003 surface. The
three MINOR items are polish, not merge blockers.

`REVIEW_Q2-SOAK verdict=PASS blockers=0 majors=0 minors=3 tests=PASS total=390`
