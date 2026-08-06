# Review: M3-5 — Zellij pass-through policy

- Branch: `agent/m3-zellij-passthrough`
- Head SHA: `ad597d2cb1a2`
- Reviewer: GLM (independent; did not author this code)
- Lane handoff: `docs/coordination/handoffs/qwen-c.md`

## Authority note

`state/tasks/M3-5.md` is **not present** in the fleet repo
(`state/tasks/` holds M3-1a, M3-1b, M3-3, M3-4, M3-ADV-session, M3-EXP-zellij
only). The acceptance criteria below were therefore reconstructed from the
authoritative compatibility-matrix row "Noren Zellij Pass-through Mode"
(`docs/compatibility/zellij.md:271`), ADR 0003, and the lane handoff. If the
missing spec carries criteria beyond these, this review should be re-run
against it.

## Gate (real output, run on the branch worktree)

```
$ cargo fmt --all -- --check
(exit 0)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.31s
(exit 0)

$ cargo test --workspace
... 368 passed, 0 failed, 1 ignored (pre-existing) across all targets
```

fmt clean, clippy clean, 368 passed / 0 failed. Matches the handoff's claimed
368 (+15 over the 353 baseline). Toolchain `cargo 1.88.0 / rustc 1.88.0`,
`aarch64-apple-darwin`.

## Acceptance criteria — one by one

| Criterion (from the matrix row + ADR 0003) | Met? | Evidence |
| --- | --- | --- |
| Freeze the permitted interception manifest | yes | `PassthroughPolicy::try_new` is the single enforcement point (`crates/noren-app/src/passthrough.rs:797`) |
| Minimal accepted set intercepted | yes | default claims exactly one chord, `Super+Escape`; `try_new` rejects more than exit + one palette (`:797-859`) |
| Child forwarding continues byte-for-byte | yes | `unbound_input_is_forwarded_byte_for_byte` compares the gate output to a direct `KeyEncoder` encode over a wide corpus (`tests/passthrough.rs:544`) |
| Exit via configured leader / palette / GUI | partial-by-lease | keyboard exit leader required at construction (`:826`); `RecoveryRoute::PointerInvokedPalette` modeled unconditionally (`:887`). The pointer surface itself is out of lease (config/lib.rs excluded); handoff states this |
| Collisions asserted mechanically vs pinned corpus | yes | `default_manifest_has_zero_collisions_with_zellij_defaults` (`:235`); corpus pinned to v0.44.3 / `55a2121` (`:37,40`) |
| No trapped session (reject or always-reachable non-keyboard recovery) | yes | fail-closed `try_new` + `PointerInvokedPalette` always present (`:887`); anti-trap cases tested (`:322,811`) |
| File lease honored | yes | `git diff --numstat origin/main...HEAD` is purely additive on the three leased paths; `lib.rs`/`main.rs`/`input.rs`/`Cargo.*` untouched |
| ADR 0003 boundary (no pane/tab/layout/split, no Zellij-layout read/persist) | yes | see below |

Deferred items (configurable leader schema, command-palette surface,
entry/exit precedence) are explicitly outside this lane's lease and are
documented as such in the handoff. They are not counted as unmet criteria.

## Boundary (ADR 0003) — no violation

`grep` for pane/tab/split/layout in `passthrough.rs` returns only (a) comments
restating the boundary (`:5-6`) and (b) descriptive **labels** for Zellij's own
modes inside the collision corpus (`mode: "pane"`, `mode: "tab"` at `:409,442`).
The module carries no workspace-model state: it never introduces a Noren
pane/tab/layout/split type and never reads or persists Zellij's internal
layout. The corpus describes what Zellij binds so Noren can avoid it; it does
not model what Noren owns. Honored.

## Regressions / unintended deletions

`git diff --numstat origin/main...HEAD`:

```
981 0 crates/noren-app/src/passthrough.rs
826 0 crates/noren-app/tests/passthrough.rs
163 0 docs/coordination/handoffs/qwen-c.md
```

Purely additive (all `0` deletions). No forbidden file touched. No unintended
removals.

## Panics / resource leaks / unbounded growth

- `PassthroughGate::pending` is bounded by `MAX_LEADER_CHORDS` (8): a candidate
  can only stay `Pending` while it is a strict prefix of a claim, and claims are
  capped at 8 (`:267`). Verified by driving an 8-deep identical-chord leader:
  depth reaches 7 then completes; a hostile 10 000-press stream never exceeded
  the cap.
- No `unwrap`/`expect`/indexing on untrusted input in production paths; the
  only `.expect` is the corpus constant builder (`:317`), defended by the
  corpus-sanity test.
- `collisions()`, `zellij_default_bindings()`, `claims()`, `replay_timeout()`
  all return bounded structures (≤ ~120 corpus; replay ≤ 8).

No leaks or unbounded growth found.

## Combinations the author did not test

I exercised an untested interaction: **two simultaneous claims of unequal
length through the gate** (exit = 2-chord `[Super+e, Super+s]`, palette =
single `[Super+p]`). The author's `exit_plus_optional_palette_is_the_maximal_manifest`
constructs both but never drives the gate with both live. The gate behaved
correctly: a held `Super+e` is replayed when `Super+p` completes the palette,
and the palette intercepts standalone. Probe passed; no defect found here.

I also confirmed `replay_timeout()` is idempotent (second call yields nothing)
and that `press` after an `Intercepted` leaves the gate clean — both correct
and previously untested.

## Mutation testing (do the tests test the behavior?)

Three mutations applied to `passthrough.rs`, each reverted after.

1. Swap collision check order (prefix before exact) →
   `collision_detector_flags_documented_zellij_chords` **FAILED**. Exact
   detection is genuinely covered.
2. Make `try_new` synthesize a default exit when none is supplied →
   `policy_rejects_manifests_that_could_trap_or_overreach` and
   `an_invalid_manifest_means_pass_through_is_not_enterable` **FAILED**.
   The missing-exit / anti-trap requirement is genuinely covered.
3. Make `press` return `replayed: Vec::new()` on the forwarding/mismatch path
   (silently drop held leader chords) → **all 15 tests PASSED.** See MAJOR-1.

## Findings

### MAJOR-1 — replay-on-mismatch is not verified; a losing mutation passes clean

- `crates/noren-app/tests/passthrough.rs:112-120` (tautological Harness
  assertion) and `:706-738` (`leader_completion_intercepts_and_mismatch_replays_in_order`).
- **Reproduction:** in `PassthroughGate::press`, change the forwarding branch to
  `replayed: Vec::new()` (dropping `std::mem::take(&mut self.pending)`). Run
  `cargo test --test passthrough`.
- **Actual:** `15 passed; 0 failed`. The held leader chord (`Super+e`) is
  silently lost — exactly the "trapped/lost input" class the matrix forbids.
- **Expected:** at least one test must fail, asserting the held prefix is
  replayed byte-for-byte before the mismatching chord.
- **Root cause (two parts):**
  1. The Harness replay check is self-fulfilling: it drains
     `decision.replayed.len()` items from `self.held` and compares the result
     to `decision.replayed`. An empty replay therefore self-satisfies as
     `[] == []`; it can never detect a dropped replay.
  2. The mismatch test uses `Super` chords, which the app encoder drops to
     zero bytes, so `harness.bytes()` cannot witness the lost input. The
     author's own comment (`:726-727`) concedes "the ordering assertion is on
     the decision/held-stream, not the bytes" — but that stream assertion is
     the tautological one in (1).
- **Contrast:** the timeout replay path *is* byte-tested (`:769-773`), proving
  the gap is specific to the `press()` mismatch path.
- **Minimal suggested fix (do not apply — report only):** assert
  `decision.replayed` against the *expected* held chords computed independently
  of `decision.replayed.len()`; assert `self.held.is_empty()` after a mismatch;
  and/or drive a mismatch with a non-Super, non-colliding leader (e.g. a plain
  multi-chord `ChordSeq` of printable chords fed straight to the gate) so the
  replayed bytes are non-empty and `harness.bytes()` can assert ordering.

### MINOR-1 — `CollisionKind::ClaimPrefixesZellij` is a dead branch

- `crates/noren-app/src/passthrough.rs:720-722` (and docstring `:669-671`).
- The corpus is all single-chord (`ChordSeq::single`). A claim can be a strict
  prefix of a Zellij sequence only if it is shorter than one chord, which is
  impossible (`ChordSeq` is non-empty). A probe over all 1..8-chord claims
  reported `ClaimPrefixesZellij` firing for **none** of them; only `Exact` and
  `ZellijPrefixesClaim` are reachable today.
- Not a correctness defect — the collision check is still correct for this
  corpus — but the variant's docstring describes a state that cannot occur.
- **Minimal fix:** note in the docstring that the corpus is single-chord, so
  only `Exact`/`ZellijPrefixesClaim` are reachable; keep the branch as defense
  for a future multi-chord corpus.

### MINOR-2 — `default_policy()` bypasses its own validator

- `crates/noren-app/src/passthrough.rs:769-784`.
- `default_policy()` / `Default::default` construct directly without calling
  `try_new`, so the collision/ambiguity/justification checks do not run for the
  default. The default is collision-free only because
  `default_manifest_has_zero_collisions_with_zellij_defaults` tests it, not
  because construction guarantees it. A future edit to `default_exit_claim()`
  to a colliding chord would not be caught at construction.
- **Minimal fix:** build the default through `try_new(vec![default_exit_claim()])`
  (it is infallible for the known-good default) for defense-in-depth.

### MINOR-3 — per-keypress allocation on the future hot path

- `crates/noren-app/src/passthrough.rs:948-968` (`press` calls
  `policy.claims()` twice per press) and `:874-879` (`claims()` allocates a
  `Vec`).
- Each keypress allocates two small `Vec`s. Acceptable for an unwired policy
  module (integration is deferred per the handoff), but worth resolving before
  the gate is placed between platform key events and the PTY.
- **Minimal fix:** iterate `exit` / `palette` directly in `press`, or have
  `claims()` return a small fixed iterator rather than a collected `Vec`.

## Sound areas (stated briefly)

- Fail-closed validation order in `try_new` matches its docstring and is
  mutation-tested (unknown id → wrong action → duplicate → empty justification
  → missing exit → ambiguity → collision).
- `Chord`/`ChordSeq` normalization and bounds (case-fold, control/whitespace
  rejection, F1-F24, length cap) are correct and tested.
- The default `Super+Escape` claim is genuinely disjoint from the pinned corpus
  (asserted "no Super chord anywhere" `:224-229`) and from terminal-child
  convention; the "Noren reads keys before the PTY" anti-shadowing argument is
  consistent with the design.
- The implementation itself is correct on the mismatch path — the defect is
  test coverage, not shipped behavior.

## Verdict

FINDINGS. No blockers. The lane is within lease and boundary, the gate is
green, the implementation is sound, and acceptance criteria (within the
reconstructable scope) are met. The single MAJOR is a coverage gap on the
critical no-lost-input replay path, demonstrated by a mutation that ships
broken and passes every test — exactly the failure mode this project has
shipped before. The MINORs are documentation/defense-in-depth notes.

`REVIEW_M3-5 verdict=FINDINGS blockers=0 majors=1 minors=3 tests=PASS total=368`
