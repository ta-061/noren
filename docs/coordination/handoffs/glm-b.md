# Handoff — M3-1b session lifecycle supervisor (GLM-b lane)

- **Lane:** `agent/m3-session-supervisor` (lifecycle supervisor — spawn,
  terminate, select, process ownership, child reaping, stale/dead handling).
- **Branch:** `agent/m3-session-supervisor` (created from `origin/main` at
  `1d329a51582a937c37e5357e21d9a37eb49079bc`).
- **Code commit:** `e4d647962358c477ee834a7b088f444af3783857`
  ("feat(app): add session lifecycle supervisor (M3-1b)").
- **This handoff commit:** see `git log -1` on this file's commit (the handoff
  is committed separately so it can record the code commit's exact SHA).
- **Author of the code under review:** Yes — this lane both authored and
  self-verified the module. There is no second reviewer in this session.
- **TEMPLATE note:** `docs/coordination/handoffs/TEMPLATE.md` was **not present
  on `main` at `1d329a5`** (the `handoffs/` directory did not exist). This file
  follows the structure and tone of `docs/coordination/reviews/*.md` plus the
  handoff requirements stated in the task prompt. If a template lands later,
  reshape this file to match; no content here depends on the absent template.

A second model should be able to resume from this file plus `git log`/`git show`
alone, with no conversation context.

## 1. Task authority and what was actually available

The task prompt names two authority documents:

- `docs/coordination/tasks/M3-1b.md` — **not present** on `main` at `1d329a5`
  (verified: `ls docs/coordination/tasks/` → no such directory).
- `docs/coordination/decisions/D-M3-001-session-api.md` — **not present**
  (verified: `ls docs/coordination/decisions/` → no such directory; the sibling
  lane `agent/m3-session-domain` that authors D-M3-001 is checked out in a
  separate worktree `pool-m3a` but is also at `1d329a5` — i.e. its work had not
  landed when this lane started).

Per the prompt's explicit guidance ("the session-domain lane … may not exist on
`main` yet. Therefore: build against a fake/mock process model first, plus a
failure matrix"), I treated **the prompt itself as the authority** and built
against a mock process model. The prompt's bullet list of responsibilities
(spawn, terminate, select, process ownership, child reaping, stale/dead
handling) and its two hard invariants drove the design:

1. The supervisor owns child handles; the registry never does.
2. A dead child surfaces as `Exited`/`Failed`, not a stuck `Running`.

For broader M3 context I read `origin/docs/m3-breakdown:docs/roadmap/milestone-3-breakdown.md`
(branch `docs/m3-breakdown`, not merged) which decomposes M3-1 as the
pane/tab/workspace foundation. This lane is a slice of that: the lifecycle
supervisor that the workspace will own.

## 2. What this lane built

Two files (the entire file lease):

- `crates/noren-app/src/session_supervisor.rs` — the module.
- `crates/noren-app/tests/session_supervisor.rs` — independent verification.

**The module's public surface** (this lane's real, non-stub deliverable):

- `Child` trait — the process handle the supervisor owns. Two methods:
  `poll_exit() -> Result<PollOutcome, ChildError>` (non-blocking reap probe) and
  `shutdown(deadline: Instant) -> Result<(), ChildError>` (bounded, idempotent
  kill+reap, mirroring `noren-pty::PtySession::shutdown`).
- `PollOutcome { StillRunning, Exited { code } }` — the explicit reap outcome.
  This exists to kill the ambiguity where "exited with no code" could otherwise
  be read as "still alive" (see §5 — this was a real bug caught during
  verification).
- `ChildError { Poll, Shutdown(ShutdownError) }`, `ShutdownError { Failed, TimedOut }`.
- `Spawner` trait — the seam where real PTY spawn wiring drops in. The
  supervisor owns child *handles*; it does not own spawn policy.
- `SessionSupervisor` — owns every live child handle. Methods: `spawn`,
  `status`, `selected`, `select`, `clear_selection`, `poll` (non-blocking reap
  pass → `ReapReport`), `terminate(id, deadline)`, `terminate_now`,
  `shutdown_all` (one shared deadline), `forget` (retire a terminal record),
  plus `len`/`is_empty`/`running_count`.
- `ReapReport` — the set of sessions that left `Running` on one `poll` pass.
- `SessionOpError { Unknown, NotRunning, StillRunning }`.
- `SHUTDOWN_DEADLINE = 2s` — mirrors `noren-pty::SHUTDOWN_DEADLINE`.

**The STUB block** (clearly marked in the source; delete on integration):
`SessionId`, `SessionStatus`, `SessionFailure`. These reference D-M3-001 and
stand in for the domain's types so this lane compiles/tests in isolation.
Integration is a **deletion, not a merge**: drop the STUB block, re-export the
domain's types, point `SessionSupervisor` at them.

**The mock** (`#[cfg(test)] pub mod mock`): `MockChild` + `MockController`
(shared-state machine driven by the test) and `MockSpawner`. It is `pub` under
`cfg(test)` so both the inline unit tests and the integration test (which
includes the module via `#[path]`, and for which `cfg(test)` is also enabled)
share one definition.

## 3. How the invariants are enforced

- **Supervisor owns handles; registry does not.** `SupervisedSession.child` is
  the only strong reference to a `Box<dyn Child + Send>`. The registry (when it
  lands) observes `SessionStatus` by `SessionId`; it has no path to the handle.
  On any terminal transition `child` is set to `None`.
- **Dead ⇒ Exited/Failed, never Running.** Every transition path
  (`poll`/`terminate`/`shutdown_all`) writes a terminal `SessionStatus` and
  drops the handle. `select` refuses a non-`Running` id with `NotRunning`. The
  selected id is cleared when it transitions, so a caller never addresses a dead
  session as live.
- **Termination reaps, is idempotent, under one deadline.** `terminate` has a
  fast path for already-terminal ids (no backend call). `shutdown_all` computes
  one `now + SHUTDOWN_DEADLINE` and feeds that same absolute `Instant` to each
  `terminate`, so the batch is bounded by one deadline, not `n * deadline`. An
  already-elapsed deadline marks `Failed(ReapTimeout)` without calling the
  backend.

## 4. Commands actually run and real results

Run from the worktree root `/Users/yoshinagatatsuya/Documents/apps/noren-worktrees/pool-m3b`
on branch `agent/m3-session-supervisor`, toolchain `1.88.0` (`rust-toolchain.toml`).

```
$ cargo fmt --all
$ echo $?
0
```

```
$ cargo clippy --workspace --all-targets -- -D warnings
$ echo $?
0
    Checking noren-app v0.1.0
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```
(No warnings, no errors.)

```
$ cargo test --workspace
$ echo $?
0
```

Per-binary results (`cargo test --workspace`, parsed from the `Running`/`test
result:` lines):

| binary | passed | failed | ignored |
|---|---|---|---|
| noren-app lib unittests (`src/lib.rs`) | 79 | 0 | 1 (pre-existing) |
| noren-app bin unittests (`src/main.rs`) | 24 | 0 | 0 |
| **`tests/session_supervisor.rs` (NEW)** | **26** | **0** | **0** |
| `tests/verify59_independent.rs` | 19 | 0 | 0 |
| noren-pty lib unittests | 10 | 0 | 0 |
| noren-terminal lib unittests | 45 | 0 | 0 |
| noren-terminal integration tests (`adversarial`, `adversarial_kimi`, `alternate_screen`, `application_modes`, `bracketed_paste`, `control_sequences`, `embedded_c0`, `erase_operations`, `scroll_regions`, `scrollback`, `selection`, `sgr_attributes`, `string_states`, `terminal_state`, `unicode_width`, `vt_compat`) | 23+20+7+6+3+9+6+6+9+6+17+25+6+7+22+4 = 176 | 0 | 0 |
| doc-tests (3 crates) | 0 each | 0 | 0 |

**Total: 379 passed, 0 failed, 1 ignored.** The 1 ignored test is pre-existing
(in `noren-app` lib), not introduced by this lane. The new
`tests/session_supervisor.rs` target contributes **26 tests** = 14 inline unit
tests (`session_supervisor::tests::*`, compiled into the test binary via the
`#[path]` include) + 12 independent integration tests, all green. The mainline
baseline on `origin/main` was 353 passing; 353 + 26 = 379, confirming this lane
added exactly its 26 tests and broke nothing.

**Commands run during development that surfaced bugs** (kept for the record):
two earlier iterations of `cargo test --workspace` caught (a) the private-stub
constructor access from the integration test, and (b) the `Ok(None)` ambiguity
documented in §5. Both were fixed before the final green run above.

## 5. Unresolved findings / design notes for the integrator

1. **`poll_exit` outcome semantics (resolved during this lane).** The first
   design returned `Result<Option<u32>, _>`, which conflated "still running"
   with "exited with no code" — a signal-style death read as `Running`. This is
   exactly the stuck-`Running` failure the lane exists to prevent, so the trait
   now returns an explicit `PollOutcome { StillRunning, Exited { code } }`. The
   production PTY adapter **must** map `PtyEvent::Exited { code }` to
   `PollOutcome::Exited { code }` and event-absence to `StillRunning` (never
   collapse them). This is the single most important contract note.

2. **`shutdown_all` shared-deadline caveat.** The supervisor passes one absolute
   `Instant` to each child's `shutdown`. In production each `PtySession` enforces
   its **own** internal `SHUTDOWN_DEADLINE` (2s) and does not accept a tighter
   caller deadline. With `n` sessions, a strict reading of "one shared deadline"
   is therefore only achievable if (a) the adapter exposes a
   deadline-parameterised shutdown, or (b) sessions are killed concurrently then
   reaped. This lane's contract is correct for any backend that honours the
   passed deadline (the mock does); the production wiring commit must decide
   between (a) and (b) and is the right place to revisit. No code change is
   needed in this module for either choice.

3. **Kill-vs-exit code modelling.** In the mock, a supervisor kill yields
   `Exited { code: None }` (no clean code), matching the `poll_exit` contract.
   In production, `noren-pty`'s `PtyEvent::Exited { code }` carries a code even
   after kill (the reaped wait status). The adapter surfaces whatever the PTY
   reports; the supervisor is code-agnostic. Both are `Exited`, never `Running`.

4. **Not wired into the crate.** As required by the file lease, the module is
   **not** declared in `crates/noren-app/src/lib.rs`, so it does **not**
   compile into the `noren-app` library or binary yet. It is verified only via
   the `tests/session_supervisor.rs` target, which includes it with
   `#[path = "../src/session_supervisor.rs"]`. `cargo build` of the workspace
   does **not** exercise this module; only `cargo test` does. This is the
   intended state pending the serial integration commit.

5. **Boundary (ADR 0003, owner-decided).** This lane touches only process
   *lifecycle*. It introduces no pane, tab, layout, split, or any Noren-side
   workspace geometry. There is no Zellij pane/layout type here, and none is
   wanted. If a future change to this module feels like it needs a layout type,
   that is the boundary: stop and flag it rather than adding one.

## 6. What I could NOT judge about my own work

- **No independent reviewer ran.** I authored the module and wrote the
  verification suite. The 12 integration tests were written to exercise the
  public contract "the way the task describes" (mirroring the approach in
  `tests/verify59_independent.rs`), but they are my own tests, not a second
  model's. A true independent review is still owed.
- **The stubs may not match the domain's final shapes.** `SessionId`
  (locally `u64`), `SessionStatus`, and `SessionFailure` are my stand-ins. If
  D-M3-001 settles different variant names or a different id type, the STUB
  block and the supervisor's signatures change at integration time. I could not
  verify against a contract that does not exist yet.
- **Production performance under load.** All tests use an instant mock backend.
  Real `PtySession::shutdown` does blocking I/O on a supervisor thread; with
  many sessions the `shutdown_all` ordering (sequential reap under one deadline)
  is only as good as the caveat in §5.2. I did not measure real-process
  teardown because wiring is pending.
- **`PollOutcome` as a public type.** I introduced it because the lane's central
  invariant demanded it, but whether the domain lane also models reap outcomes
  this way is unknown. If D-M3-001 defines its own, the `Child` trait's return
  type should adopt that instead — a one-line change at integration.

## 7. Resume instructions

1. `git checkout agent/m3-session-supervisor`.
2. `git show e4d6479` for the full diff (the two code files).
3. `cargo test --workspace` → expect 0 failures (26 in the new target).
4. To integrate once D-M3-001 lands: delete the STUB block in
   `session_supervisor.rs`, re-export the domain's `SessionId`/`SessionStatus`/
   `SessionFailure`, add `mod session_supervisor;` (or `pub mod`) to
   `crates/noren-app/src/lib.rs`, and write the `PtySession`-backed `Child` +
   `Spawner` adapters (per the contract notes in §5.1 and §5.2). The test file's
   `#[path]` include can then be replaced with `use noren_app::session_supervisor`.
