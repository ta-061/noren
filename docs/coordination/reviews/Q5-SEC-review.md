# Independent review — Q5 TM-08 sentinel verification + IME drop count (`agent/security-no-leak`)

Independent review. I did not author this code and reviewed head from scratch
rather than trusting the handoff. Commands below were actually run; output is
quoted verbatim.

- Reviewed at: `7e75f19b78fa` on `agent/security-no-leak`, off `origin/main`
  @ `309c0b4` (macOS arm64, `cargo 1.88.0`).
- The lane prompt cites `state/tasks/Q5-SEC.md` in the fleet repo as the
  authority. **That file is absent from this branch** (`git ls-tree -r` has no
  `state/` tree). Acceptance criteria below are taken from the threat model
  TM-08 in `docs/security/threat-model.md` and the handoff
  `docs/coordination/handoffs/qwen-q5.md`, which together specify the two
  deliverables: (1) an executable sentinel test for TM-08, and (2) the IME
  drop counter.
- The branch was already checked out at the `pool-q5` worktree (worktree
  lock). All commands were run in that worktree; same commit, clean tree.
- Scope of diff (`git diff --stat origin/main...HEAD`): 4 files, +563 / −13.

  ```
  crates/noren-app/src/diagnostics.rs        |  75 +++++-
  crates/noren-app/src/main.rs               |   7 +-
  crates/noren-app/tests/security_no_leak.rs | 401 +++++++++++++++++++++++++++++
  docs/coordination/handoffs/qwen-q5.md      |  93 +++++++
  ```

  The 13 deletions are all intentional (old doc comments replaced, old format
  string replaced, old no-op Ime arm replaced). No unintended removals.

## Gate output (actually run at `7e75f19`)

```
$ cargo fmt --all -- --check
    exit 0 (no diff)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile → exit 0, 0 warnings

$ cargo test --workspace
    total passed=866 failed=0 ignored=4
```

The 4 ignored tests are pre-existing (system-clipboard, FIFO helper, two
font-stack defect specifications) and untouched by this lane. This matches
the handoff's claim of 866 passed / 0 failed / 4 ignored.

## Acceptance criteria — item by item

### 1. TM-08 executable sentinel verification — MET

TM-08 requires "a test logger that rejects known sentinel
terminal/input/environment values." The suite
(`tests/security_no_leak.rs`) injects one unique sentinel into each named
channel and scans every observable surface:

| Channel         | How the sentinel enters                                  | Surface scanned               |
| --------------- | -------------------------------------------------------- | ----------------------------- |
| Terminal input  | Typed through `KeyEncoder` (the real keystroke path)     | Diagnostics `report()` output |
| PTY output      | Live `/bin/zsh` PTY (or direct-feed fallback) into `TerminalState` | Diagnostics + child stdout/stderr |
| Working dir     | Child process runs inside a sentinel-named directory     | Child stdout/stderr           |
| Environment     | Sentinel variable name + value in child env             | Child stdout/stderr           |

The parent/child split captures the child's complete stdout+stderr. The
child asserts every injection actually took effect before trusting a clean
scan. Two anti-vacuous guards are present:

- The scanner is self-tested against a planted leak
  (`scanner_flags_planted_leaks_and_accepts_clean_output`).
- The parent asserts the child actually ran (`CHILD_COMPLETED_MARKER`),
  actually emitted the diagnostics line (`noren diagnostics:`), and actually
  recorded the IME drops (`ime_drops=3`).

The `CONFIG_ENV_VAR` override embeds the cwd sentinel in an env value, and
the child exercises both `AppConfig::load()` (success path) and
`AppConfig::load_from()` (failure path). `ConfigError` variants carry only
bounded, clipped text — never file paths (verified in `config.rs:283-311`),
so the "configuration file not found" message does not leak the sentinel
path.

### 2. IME drop counter — MET

`record_ime_drop()` / `ime_drop_count()` are added to `diagnostics.rs`:

- `record_ime_drop()` is **argument-free** — there is no code path by which
  dropped content can reach the counter. This is enforced by type, not
  convention.
- The counter is a process-wide `AtomicU64` with `Ordering::Relaxed`
  (correct for a standalone counter with no dependent memory operations).
- `report()` includes `ime_drops=N` in the fixed field sequence, before
  `state=`, preserving the invariant that `state=` is last (all existing
  `ends_with("state=…")` assertions still hold).
- The `main.rs` `WindowEvent::Ime` arm replaces the old no-op
  (`let _ = KeyDropReason::ImeOrDeadKey;`) with `diagnostics::record_ime_drop()`.

### 3. File lease — MET

Three code files touched: `diagnostics.rs` (counter + report field + unit
test), `main.rs` (Ime arm only — confirmed by `git diff`), and
`security_no_leak.rs` (new). No changes to `lib.rs`, `Cargo.toml`, or
`Cargo.lock`.

### 4. Privacy rule preserved — MET

`DiagnosticsInput` has no text fields (all `u16`, `usize`, `bool`, and
`PtyChildStatus` enum). The report is bounded ASCII with a fixed field
sequence. The sentinel test proves this end-to-end: sentinel text is placed
into terminal content (screen + scrollback + alternate screen), and the
report demonstrably does not contain it.

## Mutation testing (tests actually test the behavior)

Three mutations were applied, each confirmed to make a test fail, then
reverted:

1. **`record_ime_drop` → no-op** (`diagnostics.rs`): both the unit test
   (`ime_drops_are_counted_in_the_report_as_payload_free_numbers`) and the
   sentinel suite (`sentinels_in_…` via the `ime_drops=3` assertion) fail.
2. **Scanner → always returns `None`** (`security_no_leak.rs`): the
   self-test `scanner_flags_planted_leaks_and_accepts_clean_output` fails
   immediately (`left: None, right: Some("NOREN-TM08-IN-…")`).
3. **`ime_drops` field moved after `state=`** (`diagnostics.rs`): three
   existing tests fail (`ends_with("state=…")` breaks), proving the field
   ordering invariant is enforced.

All mutations were reverted; `git diff --stat` is clean and the full gate
passes (866 / 0 / 4).

## Interaction tested beyond the author's suite

The existing diagnostics tests cover the interaction between `ime_drops` and
all `PtyChildStatus` variants, persistence conflict/unverified flags, and
extreme grid sizes (1024×1024). The sentinel test covers the interaction
between IME drops and the live PTY path. I additionally verified that the
field ordering invariant (`state=` last) is enforced by mutating the field
position — three tests fail, confirming the interaction between the new
field and the existing format contract is tested.

## Noren/Zellij boundary (ADR 0003) — no violation

The changes are entirely within the `noren-app` crate (diagnostics module
and the `WindowEvent::Ime` event handler). No tabs, panes, splits, layout
trees, or Zellij internal state are introduced, read, or persisted.

## Panics, resource leaks, unbounded growth

- **No panics introduced.** `fetch_add`/`load` on `AtomicU64` cannot panic.
  The sentinel test's PTY interaction has a 5-second deadline and graceful
  fallback. `shutdown()` is called on all code paths in
  `route_sentinels_through_live_pty`.
- **No resource leaks.** The scratch directory is cleaned up with
  `remove_dir_all`. The PTY session is shut down on every path.
- **No unbounded growth.** The counter is a fixed-size `AtomicU64` (no
  allocation). The report is a bounded `String` with `with_capacity(128)`.
  The sentinel test's `seen` buffer is bounded by the test's own input and
  the 5-second deadline.

## Findings

### MINOR-1: Dead-key drops from the keyboard path are not counted

- **File:** `crates/noren-app/src/main.rs:2223` and `main.rs:2254`
  (`translate_logical_key` returns `Err(KeyDropReason::ImeOrDeadKey)` for
  multi-character and dead-key events). Consumed silently at `main.rs:869`
  (`let Ok(bytes) = encoded else { return; }`).
- **Expected:** The counter is documented as "IME and dead-key drops"
  (`diagnostics.rs` module docs and `record_ime_drop` doc comment), implying
  all IME/dead-key drops are counted.
- **Actual:** Only the `WindowEvent::Ime` arm calls `record_ime_drop()`.
  Dead keys and multi-character key events arriving through
  `WindowEvent::KeyboardInput` produce `Err(KeyDropReason::ImeOrDeadKey)`
  which is silently discarded by `handle_passthrough_key`. A user pressing
  dead keys on a physical keyboard sees `ime_drops=0` despite experiencing
  input loss.
- **Reproduction:** On macOS, press Option+e (dead acute) then a vowel in
  the Noren window. The dead-key event goes through `KeyboardInput`, not
  `Ime`, and is not counted.
- **Suggested fix:** In `handle_passthrough_key`, match on
  `Err(KeyDropReason::ImeOrDeadKey)` before the `let Ok(bytes) = encoded`
  line and call `diagnostics::record_ime_drop()`.
- **Note:** The handoff explicitly scopes the change to "the edit is
  confined to that one arm." This is an incompleteness in the new feature,
  not a regression. There is also no test asserting that dead keys or
  multi-char input produce `Err(KeyDropReason::ImeOrDeadKey)` — a
  pre-existing coverage gap.

## Verdict

The lane is sound. The TM-08 sentinel suite is well-designed with
anti-vacuous guards, the IME drop counter is type-safe by construction, the
privacy rule is preserved, the file lease is honored, and mutation testing
confirms the tests catch regressions. One MINOR finding: the keyboard-path
dead-key drops are not counted, which is a known scope limitation per the
handoff.
