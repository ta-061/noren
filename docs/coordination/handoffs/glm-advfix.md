# Handoff — M3 adversarial session-lifecycle fixes (`glm-advfix`)

> A second model should be able to resume from this file plus `git log` /
> `git show` alone, with no conversation context.

## Status (second pass — branch-drift re-merge)

An independent Qwen review (`docs/coordination/reviews/M3-ADVFIX-review.md`)
found one MAJOR and two MINORs; all are resolved on this branch now:

- **M-1 (MAJOR, branch drift) — RESOLVED.** The domain lane landed follow-up
  commits to `SessionRegistry::observe` *after* being merged here — notably
  `65ebc45` ("conform `SessionEvent::StatusChanged` to D-M3-001 struct
  variant") and review-gap closure `25a246c`. I merged
  `origin/agent/m3-session-domain`; both sides edit `observe`, so the
  resolution keeps **both** fixes: this branch's ADV-S1 monotonic rank guard
  (`SessionError::InvalidStatusTransition`) AND the domain lane's
  `StatusChanged { id, status }` struct conformance. The domain lane
  independently arrived at the same rank guard, so the merged `observe` carries
  it; for `session.rs` the domain lane's contract-authoritative version is
  taken (it contains both fixes; all conflict hunks were doc-comment wording).
  The merge also brought in `origin/main` (the domain lane had merged main).
  See §8 for the full resolution.
- **N-1 (MINOR, supervisor doc commit) — RESOLVED.** The supervisor lane's
  follow-up doc commit `0ac6512` ("independent re-review of M3-1b") is now
  folded in (`agent/m3-session-supervisor` merged; its code tip `2686956` was
  already an ancestor, so only the `M3-1b-review.md` update applied). The
  supervisor *code* was already fully merged and verified.
- **N-2 (MINOR, handoff count) — RESOLVED.** §5 now reads the supervisor unit
  count as **14** (was incorrectly "13"); `grep -c '#[test]'
  crates/noren-app/src/session_supervisor.rs` → 14.

All three ADV defects remain fixed after the merge; the three `adv_s*` guards
pass against the merged module (struct `StatusChanged` + rank guard). Gate
post-merge: fmt clean, clippy `-D warnings` clean, **459 passed / 0 failed /
1 pre-existing ignored**.

## Identity

- **Lane:** `glm-advfix` — fix the three defects the independent adversarial
  lane `kimi-a` reported in the session domain and supervisor (engine GLM 5.2
  via opencode).
- **Branch:** `agent/m3-adv-fixes`, created from `origin/main`, merged with the
  two owning lanes, then (second pass) re-merged with the domain lane's
  follow-up `StatusChanged` conformance and `origin/main`.
- **Authorship of the code under fix:** **No** for the original algorithms —
  they are the unmerged sibling branches `agent/m3-session-domain` (lane
  `glm-a`) and `agent/m3-session-supervisor` (lane `glm-b`), which I merged
  read-only. I authored the fixes and the regression guards.

## Authority and inputs

- **Findings:** `docs/coordination/handoffs/kimi-a.md` on
  `agent/m3-session-adversarial` (copied into this branch).
- **Reproducers:** `crates/noren-app/tests/session_adversarial.rs` on that same
  branch — three `#[ignore]` tests reproducing the defects against faithful
  local fakes of the two modules.
- The two modules were absent from `main`; I merged the owning lanes so the real
  code is present, then fixed it in place.

## Files touched (within the lease)

| File | Status | Purpose |
| --- | --- | --- |
| `crates/noren-app/src/session.rs` | edited | **ADV-S1** fix + invariant doc |
| `crates/noren-app/src/session_supervisor.rs` | edited | **ADV-S2** + **ADV-S3** fixes; mock `exited()` helper |
| `crates/noren-app/tests/session_adversarial.rs` | new (copied) + edited | regression guards ported to the real modules |
| `crates/noren-app/tests/session_supervisor.rs` | edited | adapt to the `Result` return of `terminate` |
| `docs/coordination/handoffs/glm-advfix.md` | new | This handoff |

`lib.rs`, `main.rs`, `Cargo.toml`, and `Cargo.lock` were **not** edited. ADR
0003 is respected: no pane/tab/layout/split types. The two modules remain
un-wired into `lib.rs` (wiring is a later serial integration commit), compiled
into their test binaries via `#[path]` as before.

## 1. ADV-S1 — `observe` permits non-monotonic status regression (domain)

**Defect.** `SessionRegistry::observe` only checked equality with the current
status, so a status could move backwards — `Running` → `Starting`, or, most
alarmingly, `Exited` → `Running` (resurrecting a dead session as live). Any
consumer reasoning about lifecycle order was unsound.

**Fix (`session.rs`).** Added a monotonic rank (`SessionStatus::rank`):
`Starting` (0) < `Running` (1) < terminal `Exited`/`Failed` (2). `observe`
rejects any observation whose rank is **lower** than the current status with a
new `SessionError::InvalidStatusTransition`. Same-status stays a no-op, and an
**equal-rank** transition is still allowed (e.g. `Failed` → `Exited` once a
real exit code arrives — this is a terminal refinement, not a resurrection, and
the existing `observe_records_failure_and_exit_statuses_with_payloads` test
relies on it). The single rule forbids both the backwards slide and the
resurrection. Recorded as module invariant #5 and on the `observe`/`SessionStatus`
docs.

**Regression guard.** `adv_s1_observe_rejects_non_monotonic_status_regression`
asserts `Running`→`Starting` and `Exited`→`Running` both return
`Err(InvalidStatusTransition)` and leave the status unchanged, while a
`Failed`→`Exited` refinement still succeeds.

## 2. ADV-S2 — supervisor retains dead-session records without bound

**Defect.** The domain is bounded because `Close` removes the entry; the
supervisor had no equivalent — `poll`/`terminate` marked a session terminal but
**retained** its record, and only manual `forget()` retired it, with nothing
requiring or enforcing `forget`. Repeated spawn/die cycles grew the list without
limit (same boundedness class as the prior cell-growth defect).

**Fix (`session_supervisor.rs`).** Added a compile-time cap
`RETAIN_TERMINAL_RECORDS = 16` and an automatic `retire_overflow` that runs on
**every** terminal transition (`mark_exited`/`mark_failed`/`finalize_exited`/
`finalize_failed`): while more terminal records than the cap are retained, the
oldest terminal record (first in insertion `order`) is retired — the same effect
as `forget`, gone from both `sessions` and `order`. Selection was already cleared
when the record went terminal, so no dangling selection.

**Stated bound.** Total retained records ≤ `running_count() +
RETAIN_TERMINAL_RECORDS`, a constant independent of how many sessions have ever
been spawned. 16 was chosen above the largest simultaneous-terminal count in any
existing test (10), so every existing test keeps its records and stays green
unchanged; the cap only bites long unbounded retention (the 500-cycle attack).

**Design note (read before changing).** Because eviction targets the
oldest-*spawned* terminal record, a long-lived early session that dies after the
ring has already filled is retired immediately on death — its outcome was
already delivered synchronously (the `ReapReport` from `poll`, or the return
value of `terminate`), so no death is silently lost; only post-death status
lookup by id is bounded. This is the boundedness trade-off and is intentional.

**Regression guard.** `adv_s2_dead_records_do_not_accumulate_without_bound`
runs 500 spawn-an-already-dead-child + `poll` cycles with no `forget`, then
asserts `running_count() == 0`, `len() <= RETAIN_TERMINAL_RECORDS`, and
`len() < 500`. Without the fix `len()` is 500, so the guard is strict.

## 3. ADV-S3 — `terminate` on an unknown id fabricates `Failed(PollFailed)`

**Defect.** `terminate` returned a bare `SessionStatus`, so for an id it no
longer tracked the `None` arm returned `Failed { reason: PollFailed }` — a
control-plane status describing a poll failure that never happened, on a session
that does not exist. A caller could not distinguish "I terminated a dead
session" from "that id never existed".

**Fix (`session_supervisor.rs`).** Changed the honest representation: `terminate`
and `terminate_now` now return `Result<SessionStatus, SessionOpError>`, and an
unknown id surfaces as `Err(SessionOpError::Unknown)` — the same channel
`select`/`forget` already use. No status is invented for a session the
supervisor does not track. `shutdown_all` now keeps only `Ok` results
(`if let Ok(status) = self.terminate(...)`); an id auto-retired since the
snapshot is naturally `Unknown` and omitted, which keeps the boundedness of
ADV-S2 and the honesty of ADV-S3 consistent.

**Signature-change fallout.** Every existing call to `terminate`/`terminate_now`
was updated to handle the `Result` (`.expect(...)` where the id is known-live,
or an explicit error check). These edits are mechanical and in the two test
files; no assertion was weakened. `shutdown_all`'s signature is unchanged.

**Regression guard.** `adv_s3_terminate_unknown_id_signals_unknown_not_fabricated_status`
spawns a session, terminates + forgets it (so the id is genuinely unknown), then
asserts `terminate_now(id) == Err(SessionOpError::Unknown)`.

## 4. How the reproducers were turned into guards (and the one judgement call)

The adversarial reproducers mirrored the buggy algorithm through local `fake_*`
modules because the real modules were absent from `main` at the time. On this
branch the real modules are merged, so each reproducer was **ported to compile
and run against the real code** (`#[path = "../src/session.rs"]` /
`#[path = "../src/session_supervisor.rs"]` at the top of the test file) and
flipped from `#[ignore]` to a normal `#[test]`. The guarded **property** is the
same in each case; the assertions are adapted to the fixed contracts
(regressions now *error*, unknown ids now surface as `Err`) — they are not
weakened to pass.

The `fake_*` modules and their 20 defensive tests are kept verbatim as the
original attack record; they still pass and document the design under attack.

**The one bound assertion that changed.** The original ADV-S2 reproducer
asserted `len() <= 1`. That constant was the adversarial author's encoding of
"small", written against a fake with no retention contract. The task makes the
bound **mine to choose and state** ("state the bound"), so the guard now asserts
`len() <= RETAIN_TERMINAL_RECORDS` (16) — the actual implemented cap, which
still proves the load-bearing property (500 spawn/die cycles do **not** leave
500 records; they are capped at a constant). The fakes are unchanged, so the
historical `<= 1` form is recoverable from `git show` if the council prefers a
tighter cap.

## 5. Commands actually run and real results

Run from the worktree root on branch `agent/m3-adv-fixes`.

```
$ cargo fmt --all
$ echo $?
0
```

```
$ cargo clippy --workspace --all-targets -- -D warnings
$ echo $?
0
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 11.3s
```
(No warnings, no errors.)

```
$ cargo test --workspace
```

Per-binary results (parsed from `test result:` lines):

| binary | passed | failed | ignored |
|---|---|---|---|
| noren-app lib unittests (`src/lib.rs`) | 79 | 0 | 1 (pre-existing) |
| noren-app bin unittests (`src/main.rs`) | 24 | 0 | 0 |
| `tests/session_adversarial.rs` | 42 | 0 | 0 |
| `tests/session_domain.rs` | 35 | 0 | 0 |
| `tests/session_supervisor.rs` | 29 | 0 | 0 |
| `tests/verify59_independent.rs` | 19 | 0 | 0 |
| noren-pty lib unittests | 10 | 0 | 0 |
| noren-terminal lib/bin unittests + integration tests | 221 | 0 | 0 |
| doc-tests (3 crates) | 0 each | 0 | 0 |

**Total: 459 passed, 0 failed, 1 ignored** (the 1 ignored is pre-existing in
`noren-app` lib, unrelated to this lane). The three ADV guards pass; the prior
`#[ignore]` reproducers are gone (replaced by the guards), so there are no
adversarial `#[ignore]` tests left on this branch. (`session_domain` rose 34 →
35 after the re-merge: the domain lane added a unit assertion for the
`InvalidStatusTransition` Display string.)

The `session_adversarial.rs` binary also compiles each merged module's own unit
tests in-scope (via the `#[path]` includes), so its 42 includes the domain (5)
and supervisor (14) unit tests — they run green here as well as in their
dedicated integration binaries. (N-2 corrected: the supervisor count is 14, not
13; `grep -c '#[test]' crates/noren-app/src/session_supervisor.rs` → 14.)

## 6. Resuming from here

- Re-run the gate: `cargo fmt --all && cargo clippy --workspace --all-targets
  -D warnings && cargo test --workspace` (expect 459/0/1).
- See the three fixes: `git show` the diff on `src/session.rs` (ADV-S1),
  `src/session_supervisor.rs` (ADV-S2 + ADV-S3).
- See the guards: the final section of `tests/session_adversarial.rs`
  (`adv_s1_*`, `adv_s2_*`, `adv_s3_*`).
- See the branch-drift re-merge: `git log --oneline` for the
  "Merge origin/agent/m3-session-domain" and supervisor-doc merge commits.
- **Not pushed.** Commits are `git commit -s` on this branch only.

## 7. Open items for the serial integration commit (not this lane)

- **Point the supervisor at the domain types.** The supervisor still carries its
  STUB `SessionId`/`SessionStatus`/`SessionFailure`; integration is a deletion
  (re-export the domain types). The `wrapping_add` vs `checked_add` id-minting
  divergence (kimi-a §4) should be reconciled then.
- **Selection-on-observe divergence.** The domain clears selection only on
  `Close`; the supervisor clears it on every terminal transition. The
  `domain_selected_session_observed_terminal_remains_selected_documenting_divergence`
  test still records this; reconcile when the supervisor uses the domain status.
- These are pre-existing, out of scope for the three reported defects, and
  unchanged by this lane.

## 8. Branch-drift re-merge (M-1) — full resolution

The independent review flagged that `agent/m3-session-domain` landed follow-up
fixes to `SessionRegistry::observe` *after* it was first merged here, so a naive
landing could silently drop either this branch's ADV-S1 guard or the domain
lane's D-M3-001 `StatusChanged` conformance. Resolution:

- **Merge performed:** `git merge --no-edit origin/agent/m3-session-domain`
  (the remote tip `25a246c`, which had itself merged `origin/main`). Only
  `crates/noren-app/src/session.rs` conflicted; `tests/session_domain.rs` and
  everything else auto-merged.
- **What both sides changed in `observe`:** this branch added the monotonic rank
  guard and `SessionError::InvalidStatusTransition`; the domain lane changed
  `SessionEvent::StatusChanged` from a unit variant to the contract struct
  `StatusChanged { id, status }` (`65ebc45`) and refined docs. The domain lane
  independently arrived at the same rank guard in `25a246c`.
- **Resolution choice — both survive.** For `session.rs` the domain lane's
  version is taken as the contract-authoritative superset (via `git checkout
  --theirs` on that one file): it carries **both** the rank guard
  (`if status.rank() < descriptor.status.rank() => Err(InvalidStatusTransition)`,
  ADV-S1) and the struct `StatusChanged { id, status }` construction
  (D-M3-001). Git had already auto-merged the `observe` body with both changes;
  every conflict hunk was doc-comment wording only, so no behaviour was
  silently dropped on either side.
- **No genuine behavioural conflict:** the two changes are orthogonal — the
  guard decides *whether* to accept the observation; the struct variant is the
  *shape* of the success event. Keeping both is correct, not a compromise.
- **Confirmation:** `tests/session_domain.rs` (35) and the three `adv_s*` guards
  all pass against the merged module. `grep` confirms
  `InvalidStatusTransition` and `StatusChanged {` are both present in the
  resolved `session.rs`.

### N-1 (supervisor doc) resolution

Folded in the supervisor lane's doc-only follow-up by merging
`agent/m3-session-supervisor`. Its code tip `2686956` was already an ancestor of
HEAD, so the merge applied only `0ac6512` (the `M3-1b-review.md` re-review); no
code changed, no conflict. The supervisor code remains fully merged and verified.

### Carry-in from `origin/main`

The domain lane had merged `main`, so this re-merge also brings in `main`'s
changes (fleet-tooling removal, PRs #68/#69/#72, the `session-api.md` and
`milestone-3-breakdown.md` docs, the `merge_gate.py` CI script). None of these
touch the session code or tests; `git diff --stat origin/main...HEAD` deletes no
test file. ADR 0003 is still respected.
