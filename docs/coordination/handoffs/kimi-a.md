# Handoff — M3 adversarial session-lifecycle lane (`kimi-a`)

> A second model should be able to resume from this file plus `git log` /
> `git show` alone, with no conversation context.

## Identity

- **Lane:** `kimi-a` — adversarial session lifecycle (engine GLM 5.2 via
  opencode). Independent attacker: I did **not** author the code under review.
- **Branch:** `agent/m3-session-adversarial`, branched from `origin/main` @
  `1d329a51582a937c37e5357e21a9a37eb49079bc` (353 workspace tests passing at the
  branch point).
- **Code commit (authoritative):** `3c27804f39591443b396973cede1cd403ac41963`
  ("test(app): add adversarial session-lifecycle suite (M3-ADV-session)").
- **This handoff commit:** the commit that adds this file (separate, so this
  file can record the stable code SHA above).
- **Base SHA:** `1d329a5`.
- **Diff vs main:** `git diff --stat origin/main...HEAD` shows two files added
  only (the test file + this handoff); **no edits to `lib.rs`, `main.rs`,
  `Cargo.toml`, `Cargo.lock`, or `status.md`.**

## Files touched (within the lease)

| File | Status | Purpose |
| --- | --- | --- |
| `crates/noren-app/tests/session_adversarial.rs` | new | Adversarial suite + faithful local fakes of the domain registry and supervisor. |
| `docs/coordination/handoffs/kimi-a.md` | new | This handoff. |

Nothing else was created or edited. The test target is self-contained (the
fakes live inside the file as private modules) and is **not** wired into
`crates/noren-app/src/lib.rs` — nothing in `lib.rs` references it. It compiles
and runs as a standalone test binary.

## Authorship of the code under review

**No.** The code under attack is on two unmerged sibling branches, which I read
read-only via `git show <branch>:<path>`:

- Domain registry: `agent/m3-session-domain:crates/noren-app/src/session.rs`
  (code commit `d31e3ac`, lane `glm-a`) — the D-M3-001 "shared session API
  contract".
- Lifecycle supervisor: `agent/m3-session-supervisor:crates/noren-app/src/
  session_supervisor.rs` (code commit `e4d6479`, lane `glm-b`).

I did **not** write either. I **did** author the local fakes in the test file
that mirror them — see the caveat in §1.

## 1. Task authority and what was actually available

The task prompt names two authority documents:

- `docs/coordination/tasks/M3-ADV-session.md` — **not present** on `main` at
  `1d329a5` (verified: `ls docs/coordination/tasks/` → no such directory).
- `docs/coordination/decisions/D-M3-001-session-api.md` — **not present**
  (verified: `ls docs/coordination/decisions/` → no such directory).

The domain and supervisor modules are likewise **absent from `main`** — they
live only on the unmerged sibling branches above. Per the lane brief ("If the
domain module is not on `main` yet, build against a local fake that matches
D-M3-001's shape and say so — do not block"), I treated **the prompt itself as
the authority**, read the two branch modules to recover D-M3-001's shape, and
built faithful local fakes. `docs/coordination/handoffs/TEMPLATE.md` was also
absent on `main`; this file follows the structure of the sibling handoffs
(`glm-a.md`, `glm-b.md`) plus the prompt's stated handoff requirements.

### The independence caveat (read before trusting a finding)

The two `fake_*` modules in `tests/session_adversarial.rs` are
behaviour-for-behaviour mirrors of the branch code: the public types, the
public API, and — critically — the reduction algorithm (`observe_entry`,
`close_entry`, `select_entry`) and the reaping/termination state machine
(`poll`, `terminate`, `finalize_*`, `fresh_id`) are copied line-for-line where
the algorithm is load-bearing for a finding. Consequences:

- A **failing** reproducer is valid to the extent the fake mirrors the branch.
  Every finding below is an *algorithmic* property violation (boundedness,
  status monotonicity, unknown-id handling), so it reproduces in the real code
  regardless of who typed the fake — the shared algorithm is the defect.
- A **passing** test confirms the design holds under that attack *conditional on
  the real code matching the mirror*, which I copied carefully but could not
  compile-link against (neither module is wired into `lib.rs` on `main`).
- I could not, by construction, judge whether the real modules compile cleanly
  together, because they are on separate branches with conflicting STUB types
  (the supervisor carries its own `SessionId`/`SessionStatus` STUBs pending
  integration with D-M3-001). That integration question is out of scope for an
  adversarial lane and belongs to the serial integration commit.

## 2. What this lane built

One test file: `crates/noren-app/tests/session_adversarial.rs`. It contains:

- **`fake_domain`** — mirror of `session.rs`: `SessionId`, `SessionKind`,
  `SessionStatus`, `SessionDescriptor`, `SessionAction`, `SessionEvent`,
  `SelectedSession`, `SessionError`, and `SessionRegistry` (the pure state
  machine) with `apply`/`create`/`close`/`select`/`observe` and queries.
- **`fake_supervisor`** — mirror of `session_supervisor.rs`: `SessionSupervisor`
  with `spawn`/`poll`/`terminate`/`terminate_now`/`shutdown_all`/`select`/
  `clear_selection`/`forget`, the `Child`/`Spawner` traits, `ReapReport`, and a
  `MockChild`/`MockController`/`MockSpawner` process model (the mock additionally
  records the `Instant` deadlines passed to `shutdown`, which the branch mock
  does not — this lets the `shutdown_all` test verify the *one shared deadline*
  contract empirically rather than by assertion).
- **23 tests**: 20 defensive (passing) + 3 reproduced-defect reproducers
  (`#[ignore]`, failing only under `--ignored`).

## 3. Reported defects (reproducers; reported, NOT fixed)

Each is `#[ignore = "reproduces <id>"]`, fails under `cargo test --test
session_adversarial -- --ignored`, and is left for the owning lanes / design
council.

### ADV-S1 — `observe` permits non-monotonic status regression (domain)

The module doc states `observe` "advances a session past `Created`", implying
forward-only transitions. `observe_entry` has **no transition-direction guard**:
its only check is equality with the current status. So a status can move
backwards. The reproducer shows two forms:

1. `Running -> Created`: a successfully-observed session is un-observed.
2. `Exited -> Running`: a **dead session is resurrected as live** — the most
   alarming form for a lifecycle model, since a caller would address a corpse
   as if it were running.

The handoff of the domain lane (decision #4: "Observe only mutates the status
field") confirms there is no direction guard by design. This is a real
correctness gap if "advances" is a contract; if regressions are intended (e.g.
process restart), the doc should say so. Either way the `Exited -> Running`
resurrection should not be possible.

```
panicked: assertion `left == right` failed: observe(Created) regressed a Running session; status must be monotonic
  left: Created
 right: Running
```

### ADV-S2 — supervisor retains dead-session records without bound (boundedness)

**This is the boundedness finding the brief explicitly warns about** (same
class as the prior cell-growth defect). The domain is bounded because `Close`
removes the entry entirely (invariant #4). The supervisor has **no equivalent**:
`poll`/`terminate` mark a session terminal (`Exited`/`Failed`) and release the
child handle, but **retain the record** in `sessions` and `order`; only
`forget()` retires a record, and nothing requires or enforces `forget`. So the
realistic app pattern — spawn sessions, let them die (reaped via `poll`), open
more — grows the supervisor's session list without limit. There is no
high-water mark and no automatic retirement.

The reproducer runs 500 `spawn` + `poll` cycles (each child pre-exited) with no
`forget`; after it, `running_count() == 0` but `len() == 500`. A contrasting
test (`supervisor_repeated_spawn_terminate_forget_cycle_stays_bounded`) shows
the loop *is* bounded when `forget` is called each iteration — proving the
unboundedness is specifically the missing auto-retirement / cap.

```
panicked: supervisor retained 500 dead records after 500 spawn/crash cycles; list grows without bound (no auto-retirement, no cap)
```

### ADV-S3 — `terminate` on unknown id fabricates `Failed(PollFailed)` (supervisor)

`terminate` returns `SessionStatus`, not `Result`, so there is no `Unknown`
channel. For an id it no longer tracks, the `None` arm returns
`Failed { reason: PollFailed }`: a control-plane status describing a poll
failure that never happened, on a session that does not exist. A caller cannot
distinguish "I terminated a dead session" from "that id never existed", and a
bogus status leaks out of the API. Reached via public API: `spawn -> terminate
-> forget -> terminate` (the forgotten id is unknown). Compare `forget`, which
correctly returns `SessionOpError::Unknown` for the same id.

```
panicked: assertion `left != right` failed: terminate on an unknown id fabricated Failed(PollFailed) instead of signaling that the id is unknown
  left: Failed { reason: PollFailed }
 right: Failed { reason: PollFailed }
```

## 4. Unresolved concerns (no reproducer — not reachable via public API)

- **id-minting divergence.** The domain mints ids with `checked_add` and panics
  on the (unreachable) `u64` overflow; the supervisor mints with `wrapping_add`,
  which silently wraps. After 2^64 spawns the supervisor's `fresh_id` would
  return an id colliding with a still-live record. This is a real inconsistency
  between the two modules but is not reachable through the public API in any
  realistic run, so it has no reproducer. Noted inline in the fake
  (`fresh_id`) for the owning lane. Filing it as a divergence, not a defect.
- **`shutdown_all` does not retire records.** Even the orderly shutdown path
  (`shutdown_all`) leaves every session terminal-but-retained; the caller must
  still `forget` each. This is the same root cause as ADV-S2 and would be
  resolved by the same fix.
- **Selection-on-observe divergence (documented, not a defect).** The domain
  clears selection only on `Close` (invariant #1), so a selected session
  observed to a terminal status stays selected. The supervisor enforces the
  stronger "selection implies `Running`" (clears selection on every terminal
  transition). The test
  `domain_selected_session_observed_terminal_remains_selected_documenting_divergence`
  PASSES and records this divergence; it must be reconciled when the supervisor
  is pointed at the domain types.

## 5. Commands actually run and real results

Run from the worktree root
`/Users/yoshinagatatsuya/Documents/apps/noren-worktrees/pool-m3f` on branch
`agent/m3-session-adversarial`, toolchain `1.88.0` (workspace `rust-version`).

```
$ cargo fmt --all
$ echo $?
0
```

```
$ cargo clippy --workspace --all-targets -- -D warnings
$ echo $?
0
    Checking noren-app v0.1.0 (...)
    Finished `dev` profile [unoptimized + debuginfo] target(s)
```
(No warnings, no errors. The `fake_*` mirrors expose the full published API
surface, so several faithful methods/variants go unused by tests; a file-level
`#![allow(dead_code)]` silences those — it cannot mask a missing `#[test]`,
since test functions are never "dead".)

```
$ cargo test --workspace
$ echo $?
0
```

Per-binary results (parsed from `test result:` lines):

| binary | passed | failed | ignored |
|---|---|---|---|
| noren-app lib unittests (`src/lib.rs`) | 79 | 0 | 1 (pre-existing) |
| noren-app bin unittests (`src/main.rs`) | 24 | 0 | 0 |
| **`tests/session_adversarial.rs` (NEW)** | **20** | **0** | **3 (reproducers)** |
| `tests/verify59_independent.rs` | 19 | 0 | 0 |
| noren-pty lib unittests | 10 | 0 | 0 |
| noren-terminal lib/bin unittests + integration tests | 221 | 0 | 0 |
| doc-tests (3 crates) | 0 each | 0 | 0 |

**Total: 373 passed, 0 failed, 4 ignored** (3 are this lane's reproducers; 1 is
pre-existing in `noren-app` lib). The `main` baseline was 353 passing; 353 + 20
= 373, confirming this lane added exactly its 20 defensive tests and broke
nothing. The 3 reproducers do **not** run under the default `cargo test
--workspace` (they are `#[ignore]`), so the gate stays green.

### Reproducer verification (the defects are real, not asserted)

```
$ cargo test --test session_adversarial -- --ignored
running 3 tests
test adv_s1_observe_can_regress_and_resurrect_status ... FAILED
test adv_s2_dead_records_accumulate_without_bound ... FAILED
test adv_s3_terminate_unknown_id_fabricates_status ... FAILED
test result: FAILED. 0 passed; 3 failed
```

All three fail with the messages quoted in §3, confirming the defects reproduce
against the mirrored algorithm. (The default `--workspace` gate remains green
because the reproducers are `#[ignore]`.)

## 6. Attack surface coverage

Brief's required surface → how it was attacked:

| Attack surface | Coverage |
| --- | --- |
| mass create/close cycles | `domain_mass_create_close_cycle_stays_bounded` (bounded, passes); supervisor attach/detach with forget (bounded, passes). |
| rapid selection switching | `domain_rapid_selection_switching_keeps_single_selection`; `supervisor_rapid_selection_amid_crashes_never_dangles` (invariant checker after every step). |
| child crash | `supervisor_child_crash_via_poll_is_exited_not_stuck_running`; `supervisor_signal_like_exit_reports_no_code_but_still_exited`; `supervisor_poll_error_surfaces_as_failed_not_running`. |
| stale selected session | domain divergence documented (§4); supervisor clears selection on death (verified). |
| duplicate ids | `domain_create_mints_unique_monotonic_ids`; supervisor `wrapping_add` divergence noted (§4, unreachable). |
| invalid actions | `domain_actions_against_closed_id_are_unknown_session`; `supervisor_select_refuses_dead_and_unknown_and_forget_requires_terminal`; `supervisor_spawn_failure_records_no_session`. |
| shutdown races | single-threaded model — no data races; logical interleavings covered by terminate-then-poll / poll-then-terminate ordering; `supervisor_terminate_is_idempotent_and_skips_backend_for_terminal`. |
| resource cleanup | child handle released on every terminal path (`mark_*`/`finalize_*`); `supervisor_terminate_elapsed_deadline_is_reap_timeout_without_backend_call`. |
| unbounded list growth | **ADV-S2** (the boundedness finding). |
| malformed future-persistence fixture | `domain_malformed_persistence_replay_rejects_dangling_refs` (dangling refs rejected; duplicate-id injection impossible via the action API). |
| repeated attach/detach | `supervisor_repeated_spawn_terminate_forget_cycle_stays_bounded` (bounded with forget); ADV-S2 (unbounded without it). |

## 7. Assumptions

- The fakes faithfully mirror the branch modules as read at branch point
  `1d329a5`; if either sibling rebases its module before integration, the fakes
  must be re-synced (the findings are algorithmic and will still apply unless
  the algorithm changes).
- "advances" in the domain doc is read as a forward-only contract. If the
  council decides regressions are valid, ADV-S1 is downgraded to a doc fix
  (state the intent) but the `Exited -> Running` resurrection should still be
  forbidden.
- The supervisor is single-threaded (per its doc), so "shutdown races" was
  read as *logical* interleaving races, not data races.

## 8. Resuming from here

- Re-run the gate: `cargo fmt --all && cargo clippy --workspace --all-targets
  -- -D warnings && cargo test --workspace` (expect 373/0/4).
- See the defects: `cargo test --test session_adversarial -- --ignored`
  (expect 3 FAILED).
- The three findings are owned by lanes `glm-a` (ADV-S1) and `glm-b` (ADV-S2,
  ADV-S3). They are reported, not fixed; do not delete the `#[ignore]`
  reproducers until the owning lane confirms a fix, then flip them to normal
  `#[test]`.
