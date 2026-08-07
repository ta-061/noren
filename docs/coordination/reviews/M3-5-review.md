# Review: M3-5 — Zellij pass-through policy (round 2)

- Branch: `agent/m3-zellij-passthrough`
- Head SHA: `74dd035bf018`
- Reviewer: GLM (independent; did not author this code)
- Lane handoff: `docs/coordination/handoffs/qwen-c.md`
- Prior round: `f86ae61` (FINDINGS: 1 MAJOR, 3 MINOR — all addressed in `74dd035`)

## Authority note

`state/tasks/M3-5.md` is **not present** in the fleet repo (`state/tasks/`
holds M3-1a, M3-1b, M3-3, M3-4, M3-ADV-session, M3-EXP-zellij only).
Acceptance criteria were reconstructed from the compatibility-matrix row
"Noren Zellij Pass-through Mode" (`docs/compatibility/zellij.md:271`) and
the boundary stated in the module + handoff.

## Gate (real output, run on the branch worktree)

```
$ cargo fmt --all -- --check
(exit 0)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.32s
(exit 0)

$ cargo test --workspace
test result: ok. 79 passed; 0 failed; 1 ignored  (noren-app lib)
test result: ok. 24 passed; 0 failed; 0 ignored  (noren-app bin)
test result: ok. 16 passed; 0 failed; 0 ignored  (tests/passthrough.rs)
... (all targets green)
total: 369 passed, 0 failed, 1 ignored (pre-existing) across all targets
```

Toolchain `cargo 1.88.0 / rustc 1.88.0`, `aarch64-apple-darwin`.

## Round-1 findings — verification of fixes

### MAJOR-1 (replay-on-mismatch unverified) — FIXED, mutation-proven

Re-applied the exact round-1 mutation: forwarding branch of `press`
returns `replayed: Vec::new()`, silently dropping held chords.

```
test leader_completion_intercepts_and_mismatch_replays_in_order ... FAILED
test a_second_live_claim_does_not_swallow_a_held_leader_prefix ... FAILED
  left: []  right: [Chord { code: Char('a'), ... }]
```

The mutation is now caught on both the byte boundary (`b"ax"` instead of
`b"x"`) and the decision/chord level. The Harness computes
`expected_replayed` from its own held-stream independently of
`decision.replayed`, so the tautology is gone.

### MINOR-1 (dead `ClaimPrefixesZellij` branch) — DOCUMENTED

Docstring (`:663-667`) now states the corpus is single-chord, so only
`Exact` and `ZellijPrefixesClaim` are reachable today; the branch is kept
as defense for a future multi-chord corpus. Sound.

### MINOR-2 (`default_policy()` bypassed validator) — FIXED

`default_policy()` (`:785-788`) now builds through
`try_new(vec![default_exit_claim()])`. A mutation disabling the collision
check in `try_new` is caught by both `policy_rejects_manifests...` and
`an_invalid_manifest_means_pass_through_is_not_enterable` (see mutation 5
below), which would also fire if the default claim were edited into a
collision.

### MINOR-3 (per-keypress allocation in `press`) — FIXED

`press` (`:960-990`) now borrows claims through `iter_claims()` and matches
the pending prefix in place via `seq.starts_with(pending)`. No `Vec`
allocation on the hot path. `claims()` (`:879-881`) still returns an owned
`Vec` for callers wanting ownership; `press` never calls it.

## Acceptance criteria — one by one

| Criterion (matrix row + boundary) | Met? | Evidence |
| --- | --- | --- |
| Freeze the permitted interception manifest | yes | `PassthroughPolicy::try_new` (`:801-863`) is the single enforcement point |
| Minimal accepted set intercepted | yes | default claims exactly one chord (`Super+Escape`); `try_new` rejects more than exit + one palette (`:805-808`) |
| Child forwarding continues byte-for-byte | yes | `unbound_input_is_forwarded_byte_for_byte` compares gate output to a direct `KeyEncoder` encode over a wide corpus (`tests:564-589`) |
| Exit via configured leader / palette / GUI | yes (within lease) | keyboard exit leader required at construction (`:830-832`); `RecoveryRoute::PointerInvokedPalette` modeled unconditionally (`:895-900`). Pointer surface itself is out of lease (config/lib.rs excluded); handoff documents this |
| Collisions asserted mechanically vs pinned corpus | yes | `default_manifest_has_zero_collisions_with_zellij_defaults` (`tests:255-263`); corpus pinned to v0.44.3 / `55a2121` (`:37,40`) |
| No trapped session | yes | fail-closed `try_new` + `PointerInvokedPalette` always present; anti-trap cases tested (`tests:342-449, 910-926`) |
| File lease honored | yes | purely additive diff (see below) |
| ADR 0003 boundary | yes | see below |

## Boundary (ADR 0003) — no violation

`grep` for pane/tab/split/layout in `passthrough.rs` returns only (a)
comments restating the boundary (`:5-6`) and (b) descriptive **labels** for
Zellij's own modes inside the collision corpus (`mode: "pane"`, `mode:
"tab"` at `:403,436`). The module introduces no Noren pane/tab/layout/split
type and never reads or persists Zellij's internal layout. The corpus
describes what Zellij binds so Noren can avoid it; it does not model what
Noren owns.

## Regressions / unintended deletions

```
$ git diff --numstat origin/main...HEAD
997  0  crates/noren-app/src/passthrough.rs
926  0  crates/noren-app/tests/passthrough.rs
268  0  docs/coordination/handoffs/qwen-c.md
210  0  docs/coordination/reviews/M3-5-review.md
```

Purely additive (all `0` deletions). `lib.rs`, `main.rs`, `Cargo.toml`,
`Cargo.lock` untouched (empty diff confirmed).

## Panics / resource leaks / unbounded growth

- `PassthroughGate::pending` is bounded by `MAX_LEADER_CHORDS` (8): a
  candidate can only stay `Pending` while it extends a claim prefix, and
  claims are capped at 8. Probed with an 8-deep leader: depth reaches 7
  then completes; a hostile 100 000-press stream of mismatching chords
  never grew `pending` beyond 0; 50 000 pending→mismatch cycles never
  leaked.
- No `unwrap`/`expect`/indexing on untrusted input in production paths.
  The only `.expect` is the corpus constant builder (`:311`), defended by
  the corpus-sanity test. `default_policy()`'s `.expect` (`:787`) is
  guarded by the collision test and is defense-in-depth per its docstring.
- `collisions()`, `zellij_default_bindings()`, `claims()`,
  `replay_timeout()` all return bounded structures (≤ ~131 corpus; replay
  ≤ 7).

No leaks or unbounded growth found.

## Combinations the author did not test

I exercised interactions beyond the author's suite via a temporary probe
file (removed before commit):

1. **Divergent multi-chord claims with shared prefix** (exit `[a,g]`,
   palette `[a,x]`): the gate correctly holds `a` as Pending, then
   intercepts the correct claim on the diverging chord. Not in the test
   suite; correct.
2. **Prefix-chord ambiguity rejection** (exit `[q]`, palette `[q,x]`):
   correctly rejected as `AmbiguousLeader`. The ambiguity check uses
   `is_prefix_of` which covers the equal case too, so this is caught.
3. **8-deep leader full completion**: all 8 chords consumed, Intercepted
   returned, gate clean afterwards. Probed; correct.
4. **Held prefix + standalone claim chord** (exit `[a,g]`, palette `[q]`,
   press `a` then `q`): the palette chord is **forwarded** (not
   re-evaluated for interception). This is the same design decision the
   author tests in `a_second_live_claim_does_not_swallow_a_held_leader_prefix`
   (`tests:784-836`): a held prefix commits to its claim; on mismatch,
   everything flushes forward. Consistent and tested — not a defect.
5. **`replay_timeout()` idempotency**: second call returns empty. Probed;
   correct.

## Mutation testing (do the tests test the behavior?)

Five mutations applied to `passthrough.rs`, each reverted after.

| # | Mutation | Result |
| --- | --- | --- |
| 1 | Forwarding branch returns `replayed: Vec::new()` (drop held chords) | **2 tests FAIL** — round-1 MAJOR fix verified |
| 2 | Remove completion loop entirely (never Intercepted) | **3 tests FAIL** |
| 3 | `is_prefix_of` always returns `true` (corrupts collisions + ambiguity) | **12 tests FAIL** |
| 4 | Completion check uses `seq[0]` instead of `seq[pending.len()]` | **1 test FAIL** (multi-chord completion) |
| 5 | Disable collision check in `try_new` | **2 tests FAIL** |

Every mutation is caught. The test suite genuinely tests the behavior.

## Cosmetic note (not ranked as a finding)

The corpus mode label `"shared_except locked"` (`:368`) uses a space where
every other label is a single word or underscore-joined (`"locked"`,
`"pane"`, `"tmux"`). This is descriptive-only with zero functional impact,
but `"shared_except_locked"` would be consistent.

## Sound areas (stated briefly)

- Fail-closed validation order in `try_new` is correct and mutation-tested.
- `Chord`/`ChordSeq` normalization and bounds (case-fold, control/whitespace
  rejection, F1-F24, length cap 8) are correct and tested.
- The `Super+Escape` default claim is genuinely disjoint from the pinned
  corpus (asserted "no Super chord anywhere" `tests:244-249`).
- The `press` implementation is allocation-free on the hot path and
  logically correct for single-chord, multi-chord, divergent, and
  mismatching inputs.
- All 15 adversarial probes (100k-press flood, 8-deep leader, rapid cycles,
  hostile chars, empty/too-long sequences, empty-collisions edge) passed.

## Verdict

PASS. No blockers, no majors, no minors. All four round-1 findings are
addressed with mutation-proven fixes. The gate is green, the implementation
is within lease and boundary, acceptance criteria (within the reconstructable
scope) are met, and every mutation I applied was caught by the test suite.

`REVIEW_M3-5 verdict=PASS blockers=0 majors=0 minors=0 tests=PASS total=369`
