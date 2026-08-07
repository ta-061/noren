# Handoff: qwen-c — M3-5 Zellij pass-through

Lane: `qwen-c`. Branch: `agent/m3-zellij-passthrough`, cut from `origin/main`
at `1d329a5` ("Merge PR #67: add optional configuration file and
diagnostics"). Signed commit, not pushed.

## Boundary (ADR 0003, owner-decided, restated)

Noren manages the workspace OUTSIDE the terminal; Zellij manages it INSIDE.
Noren has no tabs, no panes, no layout, no splits, and never reads or
persists Zellij's internal layout. This lane implemented nothing that crosses
that line: the delivered module carries no workspace-model state, only the
decision of which pressed chords Noren interprets and which bytes it
forwards untouched to the running session.

## Deliverables

File lease honored: only `crates/noren-app/src/passthrough.rs`,
`crates/noren-app/tests/passthrough.rs`, and this handoff were created.
`lib.rs`, `main.rs`, `actions.rs`, `Cargo.toml`, and `Cargo.lock` were not
touched.

1. `crates/noren-app/src/passthrough.rs` — self-contained pass-through
   policy module (no `crate::` imports, std only). Because the lease
   excludes `lib.rs`, the module is compiled and exercised through a
   `#[path]` include from the integration test; an integration lane later
   declares `pub mod passthrough;` in `lib.rs` and removes nothing.
2. `crates/noren-app/tests/passthrough.rs` — 15 tests binding the
   pass-through decision to the app byte contract through
   `noren_app::KeyEncoder`.

### API shape

- `Chord` / `KeyCode` / `Modifiers` / `ChordSeq`: normalized chord
  vocabulary (case-folded characters, no control/whitespace `Char`, F1-F24,
  non-empty leader sequences capped at 8).
- `zellij_default_bindings()`: curated corpus of the pinned Zellij v0.44.3
  default preset (commit `55a2121`), covering every mode, derived from the
  source-backed statements in
  [docs/compatibility/zellij.md](../../compatibility/zellij.md) plus a
  conservative reconstruction of `default.kdl`. Deliberately a superset
  within modes: extra entries tighten, never weaken, the collision check.
- `collisions()`: generic sequence-prefix overlap between claims and the
  corpus (exact, claim-prefixes-Zellij, Zellij-prefixes-claim).
- `PassthroughPolicy::try_new()`: fail-closed manifest validation — exit
  leader required, at most one optional palette chord, id whitelist,
  id/action pairing, non-empty justification per claim, no leader-prefix
  ambiguity between claims, zero corpus collisions. Rejection means
  pass-through is not enterable under that manifest; there is no silent
  fallback.
- `PassthroughGate`: streaming matcher returning `Forwarded` / `Pending` /
  `Intercepted(action)` with ordered replay of held leader chords on
  mismatch and a `replay_timeout()` path that forwards everything held.

## Claimed chords: 1

`noren.passthrough.exit` = **Super+Escape** (Super = Command on macOS).
Justification recorded on the claim itself: the Super/Command modifier
space has zero intersection with the pinned Zellij default corpus (which
binds no Super/Cmd chord — asserted by test) and zero intersection with
terminal-child convention (host window-layer chords do not reach terminal
children). Noren reads keys before the PTY, so the exit leader cannot be
shadowed by any child binding in any session state.

A second claim (`noren.palette.open`, command palette) is supported by the
validator but NOT enabled by default: pass-through claims as little as
possible, and the palette's pointer-invoked surface is the preferred
recovery route. The manifest cap is exit + palette; anything else is
rejected at construction.

## Collision posture

- Assertion is absence, not self-consistency:
  `default_manifest_has_zero_collisions_with_zellij_defaults` intersects the
  default manifest with the pinned corpus and requires the empty set.
- The detector is proven to detect: Ctrl+G (locked entry/unlock), Ctrl+P
  (pane mode), bare `d` (session detach) all report collisions; a two-chord
  leader beginning on Ctrl+G reports the prefix-shadow collision.
- Corpus sanity is pinned: tag v0.44.3, commit hash, presence of the
  documented anchors (Ctrl p/t/o/g, the shared Alt set, session `d`,
  Alt+f), shrink guard, and "no Super chord anywhere in the corpus".
- Zellij-bound chords Noren does not claim are asserted forwarded: the test
  walks the corpus itself through the gate and expects `Forwarded` for each
  (e.g. Alt+f reaches the PTY as `ESC f` unchanged).

## Recovery (no trapped user)

- Keyboard: the exit leader is required for policy construction and is a
  recovery route by construction.
- Non-keyboard: `RecoveryRoute::PointerInvokedPalette` is modeled as
  unconditionally present, matching the matrix obligation that an approved
  pointer-invoked recovery remains reachable regardless of binding state or
  configuration validity.
- Fail-closed: every invalid manifest (empty, missing exit, colliding
  leader, duplicates, unknown ids, empty justification, prefix ambiguity,
  over-cap sequence) is refused, so a disabled/invalid/shadowed leader
  never activates pass-through. This is the configuration-rejection arm of
  the matrix's "rejection or always-reachable non-keyboard recovery" rule.

## Gate output (real)

```text
$ cargo fmt --all
(exit 0, no changes after initial format)

$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (/Users/yoshinagatatsuya/Documents/apps/noren-worktrees/pool-pass/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.53s

$ cargo test --workspace
...
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out   (tests/passthrough.rs)
...
total: 368 passed, 0 failed, 1 ignored (pre-existing) across all targets
```

Baseline before this lane: 353 passed, same 1 ignored. Delta: +15, no
regressions. Clippy's coverage of the new test target was verified with a
planted-warning canary (removed afterwards) because warm caches made the
raw timing ambiguous. `python3 scripts/check_docs.py` reports OK with this
handoff present.

Toolchain: `cargo 1.88.0 (873a06493 2025-05-10)`,
`rustc 1.88.0 (6b00bc388 2025-06-23)`, `aarch64-apple-darwin`, per
`rust-toolchain.toml` pin.

## Boundary stops and out-of-scope (said so, not built)

- No configurable-leader config schema, no command-palette surface, no
  entry/exit precedence beyond the gate: the matrix lists these as awaiting
  approved requirements, and the lease excludes `config.rs`/`lib.rs` anyway.
- No reading or persisting of Zellij layout anywhere; no tab/pane/split
  concepts entered the module.
- `actions.rs` integration: the module emits its own decision tokens
  (`PassthroughAction`), not app actions. The actions lane maps
  `ExitToWorkspace` / `OpenCommandPalette` onto the registry it owns.

## Integration notes for the next lane

1. Declare `pub mod passthrough;` in `crates/noren-app/src/lib.rs` (the
   module has zero `crate::` dependencies, so this is additive).
2. Sit the gate between translated key presses and `KeyEncoder::encode_with`:
   Pressed/Repeat events map `KeyInput` -> `Chord` (the test file contains a
   working mapping), gate decision `Forwarded`/`Pending`/replay feed the
   encoder unchanged, `Intercepted` dispatches via the actions registry.
   Releases never enter the gate.
3. Wall-clock owner: call `replay_timeout()` when a pending leader expires;
   the module deliberately holds no timer.

## Known limitations (recorded, not hidden)

- The corpus is an advisory snapshot of the pinned preset, not byte-fixture
  evidence; the matrix assigns the `Z-PROTO`/`Z-SSH` byte-oracle runs to
  `codex-lab`. Over-inclusion makes the collision claim stronger, and the
  provenance is labeled in-code.
- Legacy encoding ambiguity (e.g. Ctrl+Shift+letter vs Ctrl+letter) and the
  encoder's existing Super drop are orthogonal to pass-through: the module
  decides interpret-vs-forward and never rewrites bytes. Super chords the
  encoder cannot yet encode forward to that documented encoder limitation,
  not to a pass-through transformation.
- Unlock-First preset behavior is covered only via the note that its shared
  Alt chords are corpus-representative and still Super-free; the hashed
  `Z-UF-*` fixtures are evidence targets, not inputs to this lane.

---

# Review round 1 response (M3-5-review, GLM, commit `f86ae61`)

Verdict was FINDINGS: 1 MAJOR, 3 MINOR. All four are addressed below; none
deferred. File lease unchanged.

## MAJOR-1 — replay-on-mismatch was not verified (fixed, mutation-proven)

The reviewer's mutation (forwarding branch of `PassthroughGate::press`
returning `replayed: Vec::new()`, silently dropping held leader chords) was
reproduced exactly as described before any fix was attempted:

```text
mutation applied, tests as shipped:
test result: ok. 15 passed; 0 failed; 0 ignored   <- defect: suite proves nothing
```

Root causes, both confirmed: the Harness asserted `decision.replayed`
against held chords drained by `decision.replayed.len()` — an empty replay
self-satisfies `[] == []`; and the mismatch test used Super chords, which
the encoder drops to zero bytes, so the byte stream could not witness lost
input either.

Fix, in `crates/noren-app/tests/passthrough.rs` only:

- The Harness now computes the expected replay from its own held-stream,
  independently of the decision: a `Forwarded` outcome must replay every
  held chord in order, and `Pending`/`Intercepted` must replay none. The
  drain-by-decision-length pattern is gone.
- `leader_completion_intercepts_and_mismatch_replays_in_order` now drives
  the mismatch through a printable, corpus-unbound leader (`[a, g]`, both
  absent from every pinned Zellij mode, so `try_new` accepts it): the child
  must receive `b"ax"` — replayed held chord before the mismatching chord.
- `leader_timeout_replays_held_chords_for_forwarding` uses the same
  printable leader, making the timeout replay byte-observable (`b"a"`)
  instead of empty-vs-empty.
- New regression `a_second_live_claim_does_not_swallow_a_held_leader_prefix`
  pins down the reviewer's own untested combination (two live claims of
  unequal length, exit `[a, g]` plus palette `q`): standalone palette chord
  intercepts; a palette chord after a held exit prefix replays the prefix
  and forwards — child bytes `b"aq"`.

Re-applied the same mutation after the fix; it is now caught on both the
original and the MINOR-3-refactored `press`:

```text
mutation applied, tests after fix:
test leader_completion_intercepts_and_mismatch_replays_in_order ... FAILED
test a_second_live_claim_does_not_swallow_a_held_leader_prefix ... FAILED
  assertion `left == right` failed: a mismatch must replay every held leader chord, in order
    left: []
    right: [Chord { code: Char('a'), ... }]
test result: FAILED. 14 passed; 2 failed; 0 ignored

mutation reverted:
test result: ok. 16 passed; 0 failed; 0 ignored   (tests/passthrough.rs)
```

## MINOR-1 — dead `ClaimPrefixesZellij` branch (documented)

Variant docstring now states that the pinned corpus is single-chord, so only
`Exact` and `ZellijPrefixesClaim` are reachable with it today, and the
branch is retained as defense for a future multi-chord corpus. Kept, not
deleted: removing a collision shape the generic algorithm must support
would weaken the contract the moment the corpus grows a sequence.

## MINOR-2 — `default_policy()` bypassed its own validator (fixed)

`default_policy()` now builds through
`try_new(vec![default_exit_claim()])`, so the frozen default passes the
same collision/ambiguity/justification validation as any configured
manifest. A future edit of the default claim into a colliding chord fails
at construction and in the existing collision test, not only in the test.

## MINOR-3 — per-keypress allocation in `press` (fixed)

`press` no longer allocates: it borrows claims through a private
`iter_claims()` (no collected `Vec`) and matches the pending prefix in
place instead of cloning it into a candidate. The public `claims()`
accessor keeps its `Vec` shape for callers that want ownership; the hot
path never touches it. (The same refactor switched prefix matching to
slice `starts_with`, so the now-unused `ChordSeq::starts_with` helper was
removed rather than left dead.)

## Round 1 gate output (real, post-fix)

```text
$ cargo fmt --all
(exit 0)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.97s
(exit 0)

$ cargo test --workspace
...
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out   (tests/passthrough.rs)
...
total: 369 passed, 0 failed, 1 ignored (pre-existing) across all targets
```

Test-count delta vs round 0: +1 (`a_second_live_claim...`); the other 15
strengthened in place. `python3 scripts/check_docs.py` re-run: OK.
