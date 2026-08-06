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
