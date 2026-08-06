# Review — M3-1b session lifecycle supervisor (independent)

- Reviewed branch: `agent/m3-session-supervisor`
- Reviewed head SHA: `79dff71e3f26` (code commit `e4d6479`, handoff `79dff71`)
- Base: `origin/main` at `1d329a5`
- Task authority: `state/tasks/M3-1b.md` (fleet repo `noren-fleet-private`)
- Reviewer: independent (did not author the code under review), run from the
  `pool-rev-b` worktree checked out detached at `79dff71`.

## Verdict

**FINDINGS** — 0 blockers, 1 major, 5 minors. All three gates pass; all four
acceptance criteria and all four required tests are met. The major finding is a
public contract the docs promise and the code demonstrably does not deliver
(`ReapReport` ordering). The minors are test-coverage gaps proven by mutation
testing, a misleading error classification, and two integration notes. No
ADR 0003 boundary violation, no unintended deletions, no panic/leak surface.

## Gates (real output)

Toolchain: `rustc 1.88.0` via `rust-toolchain.toml`.

```
$ cargo fmt --all -- --check
$ echo $?
0
```
(No output — format clean.)

```
$ cargo clippy --workspace --all-targets -- -D warnings
...
    Checking noren-app v0.1.0 (...)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 19.79s
$ echo $?
0
```

```
$ cargo test --workspace
```
Per-binary `test result:` lines (all `ok`):

| binary | passed | failed | ignored |
|---|---|---|---|
| noren-app lib (`src/lib.rs`) | 79 | 0 | 1 (pre-existing) |
| noren-app bin (`src/main.rs`) | 24 | 0 | 0 |
| `tests/session_supervisor.rs` (new) | 26 | 0 | 0 |
| `tests/verify59_independent.rs` | 19 | 0 | 0 |
| noren-pty lib | 10 | 0 | 0 |
| noren-terminal lib | 45 | 0 | 0 |
| noren-terminal integration (16 targets) | 176 | 0 | 0 |
| doc-tests (3 crates) | 0 | 0 | 0 |

Total **379 passed, 0 failed, 1 ignored**. The 1 ignored test is
`crates/noren-app/src/clipboard.rs:228` (`#[ignore = "touches the real macOS
system clipboard"]`), present identically on `origin/main` — not introduced by
this lane. The handoff's count claims (353 baseline + 26 new = 379; 14 inline +
12 integration = 26) reproduce exactly.

## Acceptance criteria (one by one)

1. **Supervisor owns child handles; the registry never does. — MET.**
   `SupervisedSession.child` (`src/session_supervisor.rs:320-323`) is the only
   strong reference to `Box<dyn Child + Send>`; every terminal transition sets
   it to `None` (`mark_exited` line 593, `mark_failed` line 604,
   `finalize_exited` line 615, `finalize_failed` line 626). No registry exists
   on this branch (M3-1a unmerged), and the module is not declared in
   `lib.rs`, so nothing else can hold a handle. Structural, and enforced by
   type ownership.

2. **A dead child produces Exited/Failed rather than a stuck Running. — MET.**
   Verified by probe: unprompted exit (`ctrl.exit(Some(42))`) is surfaced by a
   single `poll()` as `Exited { code: Some(42) }`; poll backend error becomes
   `Failed(PollFailed)`; shutdown timeout becomes `Failed(ReapTimeout)`;
   elapsed deadline becomes `Failed(ReapTimeout)` without a backend call; even
   the defensive "handle missing while Running" case (line 452-453) resolves to
   `Failed`, never `Running`. Also verified a racing natural exit then
   `terminate_now` keeps the natural code `Exited { code: Some(42) }`.

3. **Termination reaps the child and is idempotent. — MET.** Verified by
   reading and mutating: with the fast path deleted, 3 committed tests fail
   (see mutation section). `shutdown_all` is idempotent at the backend level:
   my probe recorded per-child `shutdown` call counts `[1, 1, 1]` after the
   first pass and unchanged after a second pass.

4. **Until M3-1a lands, only a fake process model and a failure matrix exist.
   — MET.** `docs/coordination/decisions/D-M3-001-session-api.md` is absent on
   this branch (the `decisions/` directory does not exist); the STUB block
   (lines 64-149) is clearly marked; the mock + tests are the only executable
   surface; the module is not wired into `lib.rs` (forbidden by the lease).

## Required tests — present and behavior-bound

- spawn then Running via fake supervisor: `spawn_assigns_unique_ids_and_selects_newest`
  (inline) — mutates `fresh_id` → fails.
- child crash surfaces as Failed/Exited: `poll_surfaces_unprompted_exit...`,
  `unprompted_death_surfaces_as_exited_within_one_poll`, plus the poll-error
  pair.
- double terminate idempotent, reaps once: `terminate_reaps_a_running_child_and_is_idempotent`,
  `terminate_reaps_and_second_call_is_a_no_op` (asserts backend call counts).
- stale session detected: `poll` over a child that died with no status update
  is exactly the unprompted-death tests above.

## Interaction / adversarial tests beyond the author's suite

The author tested each feature alone; this project has repeatedly shipped
combined-feature defects, so I probed combinations (temporary probes, since
removed):

- **forget + shutdown_all** (finding 3): spawn 3, terminate+forget the middle,
  `shutdown_all` reports only the two tracked ids. Current behavior correct;
  not covered by any committed test (proven by mutation).
- **racing natural exit + terminate**: `exit(Some(42))` then `terminate_now` →
  `Exited { code: Some(42) }`; the natural code survives the kill path.
- **terminate after poll-failure**: poll marks `Failed(PollFailed)`; a later
  `terminate_now` returns the recorded status and performs **zero** backend
  calls — correct fast-path behavior for `Failed`, not just `Exited`.
- **shared deadline proof**: a deadline-recording `Child` confirmed all three
  children in a batch receive the *same* `Instant` from `shutdown_all`.
- **terminate(unknown id)**: after forget, `terminate_now(id)` fabricates
  `Failed(PollFailed)` (finding 4).
- **Degenerate input**: no parse surface — the module is a state machine
  driven by app code. Panic surface is limited to `.expect("mock lock")` in
  test-only code. `fresh_id` uses `wrapping_add` (collision after 2^64 spawns;
  theoretical, noted, not a finding). `shutdown_all` clones `order` (O(n),
  bounded by session count). No unbounded buffers beyond the intentional
  terminal-record retention (finding 5 territory — see below).

## Mutation testing — do the tests test the behavior?

All mutations were applied, tested, and reverted; the tree is clean again.

| mutation | committed suite | verdict |
|---|---|---|
| suppress `Exited` transition in `poll` (stuck `Running`) | **8 failures** | caught |
| delete `terminate` terminal fast path | **3 failures** | caught |
| per-iteration deadline in `shutdown_all` (n*deadline bug) | **26 pass** | **NOT caught** — only a deadline-recording probe catches it |
| delete `order.retain` in `forget` (zombie ids re-terminated by `shutdown_all`) | **26 pass** | **NOT caught** — only the forget+shutdown_all probe catches it |

The first two columns show the load-bearing invariants are genuinely tested.
The last two are findings 2 and 3: real, documented contracts with zero
committed coverage.

## Unintended deletions / lease

`git diff --stat origin/main...HEAD`: 3 files, **1573 insertions(+), 0
deletions**. `git diff --diff-filter=D` lists nothing. Forbidden files
(`lib.rs`, `main.rs`, `Cargo.toml`, `Cargo.lock`, `status.md`) untouched —
verified by the diff stat above; `cargo build` therefore still excludes the
module, as intended. The handoff doc is outside the leased code paths but is
the workflow's required coordination artifact.

## ADR 0003 boundary

Clean. No pane, tab, layout tree, or split type anywhere in the diff; grep for
`pane|layout|split|zellij|tab` over both files matches only the word
"ownership split" in a comment (line 19). The module owns process lifecycle
only; `select` is session focus, which the task spec itself assigns to this
lane ("spawn/terminate/select"). No Zellij internal layout is read or
persisted.

## Findings

### MAJOR 1 — `ReapReport` promises insertion order; `poll` delivers HashMap order

- Location: contract at `crates/noren-app/src/session_supervisor.rs:282-284`
  ("Lists exactly the sessions that left `Running` this pass, **in insertion
  order**") vs implementation at `crates/noren-app/src/session_supervisor.rs:440-445`
  (ids collected from `self.sessions.iter()` — a `HashMap` with randomized
  iteration order). The insertion order is kept in `self.order` (line 333) but
  `poll` never consults it.
- Reproduction (reviewer probe, since removed): spawn 10 sessions, exit all
  via their controllers, one `poll`:
  ```
  left:  [SessionId(6), SessionId(1), SessionId(7), SessionId(0), SessionId(4),
          SessionId(8), SessionId(2), SessionId(3), SessionId(9), SessionId(5)]
  right: [SessionId(0), SessionId(1), ..., SessionId(9)]
  assertion `left == right` failed: ReapReport promises insertion order
  ```
- Expected: `report.exited()` in spawn order `[0..9]`. Actual: arbitrary
  `RandomState` order, nondeterministic supervisor-to-supervisor.
- Impact: no committed test transitions more than one session per `poll`, so
  the suite cannot see this. A future UI/event loop that renders or applies a
  pass's transitions in reported order gets shuffled, nondeterministic output.
- Minimal fix: in `poll`, collect ids from `self.order.iter().filter(...)`
  (filtering to `Running`) instead of `self.sessions.iter()`, and add one test
  that transitions ≥2 sessions in a single pass and asserts report order.

### MINOR 2 — shared-deadline contract of `shutdown_all` has no test that can catch a regression

- Location: `crates/noren-app/src/session_supervisor.rs:551-561`; the only
  timing assertion is `tests/session_supervisor.rs:123`
  (`elapsed <= SHUTDOWN_DEADLINE`).
- Evidence: with the deadline moved inside the loop (regressing to
  `n * SHUTDOWN_DEADLINE` — the exact bug the module doc and commit message
  claim to prevent), `cargo test -p noren-app --test session_supervisor` still
  reports `test result: ok. 26 passed; 0 failed`. An instant mock can never
  make a wall-clock assertion catch this.
- Current behavior is correct (probe confirmed one shared `Instant` reaches
  every child); the contract is simply unguarded.
- Minimal fix: add a `Child` impl that records the `deadline` argument of each
  `shutdown` call, and assert all recorded values are identical after
  `shutdown_all`.

### MINOR 3 — `forget`'s `order` maintenance and the forget+`shutdown_all` interaction are untested

- Location: `crates/noren-app/src/session_supervisor.rs:577`
  (`self.order.retain(...)`).
- Evidence: deleting the `retain` line leaves committed tests green
  (`test result: ok. 26 passed`), while `shutdown_all` then resurrects the
  forgotten id and reports it as `Failed(PollFailed)`. No committed test
  combines `forget` with `shutdown_all`.
- Current behavior is correct; the interaction is unguarded.
- Minimal fix: test spawn-3 → terminate+forget one → `shutdown_all` reports
  exactly the remaining ids (this is precisely the reviewer probe that caught
  the mutation).

### MINOR 4 — `terminate` on an unknown id fabricates `Failed(PollFailed)`

- Location: `crates/noren-app/src/session_supervisor.rs:485-489`.
- Reproduction: spawn → terminate → `forget(id)` → `terminate_now(id)` returns
  `SessionStatus::Failed { reason: SessionFailure::PollFailed }` (probe-verified).
- Expected vs actual: `select`/`forget` correctly report
  `SessionOpError::Unknown` for such ids, but `terminate` has no error channel
  and invents a "poll failed" status for a session that never existed; a UI
  rendering statuses cannot tell "unknown id" from a genuine backend poll
  fault. The reason recorded has no relation to what happened.
- Minimal fix: at minimum use a dedicated classification (or `Unknown` if one
  is added to `SessionFailure` at D-M3-001 integration); preferably give
  `terminate` a `Result<SessionStatus, SessionOpError>` so unknown ids surface
  as `Unknown`. If kept as-is, document the chosen semantics on the method.

### MINOR 5 — `Child::shutdown` deadline parameter cannot be honored by the known production backend; abandon path may still block in `Drop`

- Location: contract `crates/noren-app/src/session_supervisor.rs:238-239`
  ("Kill, reap, and join before `deadline`"); pre-check at lines 493-495;
  production backend `crates/noren-pty/src/lib.rs:376-407` (`shutdown` takes
  no deadline, uses its own internal `SHUTDOWN_DEADLINE`) and
  `crates/noren-pty/src/lib.rs:420-424` (`impl Drop for PtySession` calls
  `shutdown()`).
- Consequences once the real adapter is wired: (a) the deadline argument is
  dead weight for `PtySession` (tight caller budgets can be exceeded by the
  internal 2 s); (b) `terminate`'s already-elapsed-deadline path marks
  `Failed(ReapTimeout)` and drops the handle *without* calling `shutdown` —
  but the adapter's `Drop` will then run a full shutdown attempt anyway, so
  "no backend call" (asserted by the mock-based test at lines 967-980) will not
  hold in production.
- The handoff §5.2 flags half of this; the Drop interaction seems new. No code
  change is required in this lane — but the serial integration commit must
  decide (deadline-parameterised adapter vs concurrent kill) and the `Child`
  doc should note Drop semantics. Recorded so the integrator cannot miss it.

### MINOR 6 — terminal records are retained without bound until cooperative `forget`

- Location: `crates/noren-app/src/session_supervisor.rs:315-323` (record
  kept), 563-582 (`forget` is the only removal path).
- Behavior: every spawned session's record stays in `sessions` (and `order`)
  until someone calls `forget`; nothing reaps records automatically and there
  is no cap. A long-running supervisor whose owner never forgets grows without
  bound.
- Assessment: this is a deliberate, documented design ("The status remains so
  callers can observe the outcome by id", lines 317-319), and the retirement
  API exists. Ranking MINOR because the retention policy is delegated to a
  registry that does not exist yet and nothing enforces it. Suggested fix:
  at integration, the registry/domain must own an explicit retirement policy
  (e.g., forget after the UI observes the terminal status, or a record cap);
  alternatively document on the type that the map grows until `forget`.

## Areas checked and found sound

- Idempotency of `terminate` and `shutdown_all` (mutation-verified, §above).
- Selection safety: selecting a dead session is refused; selection is cleared
  on every terminal transition of the selected id (all four `mark_*`/`finalize_*`
  paths, lines 596-598, 607-609, 617-619, 627-629); `forget` also clears it
  defensively.
- The `Ok(())`-shutdown path reads the code via a second `poll_exit` and never
  leaves the session `Running`, even on backend inconsistency (lines 507-517).
- `#[path]` inclusion mechanics: the module compiles once per test binary only;
  `cfg(test)` gating keeps `mock` out of production; `#![forbid(unsafe_code)]`
  in the integration target; no `unsafe` in the module.
- File lease and forbidden files (§above); handoff factual claims
  (§Gates — all reproduce).
