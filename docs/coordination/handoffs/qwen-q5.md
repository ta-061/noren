# Handoff — Q5: TM-08 sentinel verification + IME drop count

Status: merge candidate. Updated 2026-08-14. Lane: `agent/security-no-leak`.

## Purpose

Two related security gaps, closed together:

1. **TM-08 had no executable verification.** The threat model names a control
   ("a test logger that rejects known sentinel terminal/input/environment
   values") and `diagnostics` claims it never emits PTY content — there is
   deliberately no opt-in for content — but the claim was untested. An
   untested security property is an assumption, not a guarantee.
2. **The IME drop reason was computed and discarded** (`main.rs`,
   `WindowEvent::Ime`): `let _ = KeyDropReason::ImeOrDeadKey;` constructed the
   reason and threw it away, so an IME or dead-key drop was invisible — not
   counted, not logged, not surfaced. A user whose input silently disappears
   had no way to see that it happened.

## What landed

File lease honored exactly. Three files:

1. `crates/noren-app/src/diagnostics.rs` — payload-free IME/dead-key drop
   counter (`record_ime_drop` / `ime_drop_count`) and a new `ime_drops=` field
   in the report, plus one unit test.
2. `crates/noren-app/tests/security_no_leak.rs` (new) — the TM-08 sentinel
   suite: 4 tests.
3. `crates/noren-app/src/main.rs` — **the `WindowEvent::Ime` arm only**.
   `main.rs` is an integration path; the edit is confined to that one arm
   (replacing the discarded reason with `diagnostics::record_ime_drop()`).
   No other line of `main.rs` changed; `git diff` confirms it.

No changes to `lib.rs`, `Cargo.toml`, or `Cargo.lock`; no new dependencies.

## Design: the drop counter cannot carry content

The constraint was: diagnostics must still never emit PTY content, and the
new counter must not be able to carry the dropped character. This is enforced
by type, not by convention: `record_ime_drop()` takes **no arguments**, so
only the *fact* of a drop crosses into diagnostics — never the composed or
dead-key text. The `main.rs` Ime arm matches `WindowEvent::Ime(_)` and never
binds the payload. The counter is a process-wide `AtomicU64` read by
[`report`]; it renders as a bare integer in the fixed field sequence, keeping
the line bounded ASCII. The privacy-rule module docs now state this.

IME itself is **not** implemented — that is deferred and depends on a font
stack that does not exist yet. This lane only stops discarding the drop.

## Design: the sentinel suite

`tests/security_no_leak.rs` injects one unique sentinel into each channel
TM-08 names and scans every observable log/diagnostics surface for them:

- **terminal input** — typed through [`KeyEncoder`], the real keystroke path;
- **PTY output** — routed through a live `/bin/zsh` PTY when one can spawn
  (with a direct-feed fallback), into the `TerminalState` the snapshot reads;
- **working directory** — the child runs inside a directory whose *name* is
  the sentinel, and the `NOREN_CONFIG` override value embeds it;
- **environment** — a sentinel variable name and value, both asserted present
  in the child before the scan is trusted.

The scan is a parent/child split: the parent re-spawns the test binary for
the child scenario and captures its **complete stdout+stderr** — the whole
log surface — then asserts no sentinel appears. The suite never mutates its
own process environment or cwd in-process (the workspace denies hand-written
`unsafe`, which both `set_var` and fd redirection would require). Diagnostics
is also exercised in-process so a leak into `report` fails directly.

Two anti-vacuous guards: the scanner is self-tested against a planted leak,
and the parent asserts the child *actually ran* and *actually emitted* the
diagnostics line (including `ime_drops=3`) before it trusts a clean scan.
During development a planted `eprintln!` of a sentinel was confirmed to make
the suite fail — which is the point of writing it.

## Verification

Gate output (macOS, `cargo 1.88.0`):

```text
cargo fmt --all                                            # clean
cargo clippy --workspace --all-targets -- -D warnings      # clean
cargo test --workspace                                     # 866 passed, 0 failed, 4 pre-existing ignored
```

The 4 ignored tests are pre-existing (system-clipboard, FIFO helper, and the
two font-stack defect specifications) and untouched by this lane.

This lane adds 5 test bodies (1 in `diagnostics.rs`, 4 in
`security_no_leak.rs`); the child scenario additionally runs in the
parent-spawned process as part of the end-to-end test. No pre-existing test
changed behavior; the `report` format gained one field before `state=`, and
every existing assertion (`contains` / `ends_with("state=…")`) still holds.
