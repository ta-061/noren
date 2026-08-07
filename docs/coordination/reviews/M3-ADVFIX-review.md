# Review — M3-ADVFIX adversarial session-lifecycle fixes (second round, independent)

- Reviewed branch: `agent/m3-adv-fixes`
- Reviewed head SHA: `8c5a04b5699d` (`docs(coordination): update glm-advfix
  handoff for branch-drift re-merge`)
- Three-dot base (merge-base with `origin/main`): `25a246c` (the
  session-domain lane tip "fix(app): close session-domain review gaps")
- **This is the second-round review.** It supersedes the first-round review
  (preserved in git history at commit `33898db`, which reviewed head
  `2775fb9` and found 1 MAJOR + 2 MINORs). The author's re-merge commits
  (`5b0bb1e`, `7d0a853`, `8c5a04b5`) claim all three first-round findings
  resolved; every claim was re-verified from scratch here, not taken on faith.
- Task authority: `state/tasks/M3-ADVFIX.md` — **does not exist**. It is not
  on this branch, not on `origin/main`, and not anywhere in the fleet repo
  (`git log --all -- state/tasks/M3-ADVFIX.md` in
  `noren-fleet-private` → empty output; no such path in any history). See
  finding N-2. Acceptance criteria were reconstructed from the three
  authority-surrogates that do exist:
  1. `docs/coordination/handoffs/kimi-a.md` §3 (the defect statements — the
     original adversarial lane's findings),
  2. the `glm-advfix` dispatch prompt constraints (fix-in-place contract), and
  3. `state/tasks/M3-ADV-session.md` (the adversarial task's acceptance
     criteria, where applicable).
- Author handoff reviewed: `docs/coordination/handoffs/glm-advfix.md` —
  treated as claims to verify, not as evidence.
- Reviewer: independent (did not author the reviewed code, the fixes, the
  guards, or the first-round review). All commands below were run on
  2026-08-07 in the `pool-advfix` worktree at `8c5a04b5699d` (the branch was
  already checked out there; the designated `pool-qwen-rv2-advfix` worktree
  could not check the branch out because git refuses one branch in two
  worktrees). Toolchain: workspace `rust-toolchain.toml`, cargo 1.88-class
  stable.

## Verdict

**FINDINGS** — 0 blockers, 0 majors, 2 minors.

All three defects (ADV-S1/S2/S3) are fixed; all three regression guards are
mutation-sensitive (each fails when its fix is disabled); the gates pass
(fmt clean, clippy `-D warnings` clean from a cold target dir, **459 passed /
0 failed / 1 pre-existing ignored** — exactly matching the handoff's claim);
the three-dot diff against `origin/main` is purely additive (4609 insertions,
0 deletions); a simulated landing merge into *current* `origin/main` is fully
automatic and preserves main's `pub mod session;` wiring; and ADR 0003 is
respected. Eleven reviewer-authored probes beyond the author's own tests —
including interactions the author did not test (eviction ring × `shutdown_all`,
mid-`poll`-pass ring churn, post-eviction error-channel consistency) — all
pass. The two minors are: (1) a mutation-testing gap — reversing the eviction
order passes the entire shipped suite, so no shipped test pins eviction
order/ring membership; (2) a fleet-governance gap — the authoritative task
spec is missing from the fleet repo and its lease-conflict decision packet was
never resolved, though the branch empirically honored the prompt's lease.

All three first-round findings (M-1, N-1, N-2) are confirmed resolved —
verified independently, evidence below.

## Gates — commands actually run, real output

```
$ cargo fmt --all -- --check
$ echo $?
0
```
(No output — format clean.)

```
$ touch crates/noren-app/src/{session.rs,session_supervisor.rs} \
        crates/noren-app/tests/{session_adversarial.rs,session_supervisor.rs}
$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (/Users/yoshinagatatsuya/Documents/apps/noren-worktrees/pool-advfix/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.55s
$ echo $?
0
```

Cache honesty: warm-cache runs were not trusted. A fully cold run in a fresh
target dir was also executed:

```
$ CARGO_TARGET_DIR=/var/folders/.../advfix-clippy cargo clippy --workspace --all-targets -- -D warnings
    Checking objc2-app-kit v0.2.2
    ...
    Checking noren-app v0.1.0 (/Users/yoshinagatatsuya/Documents/apps/noren-worktrees/pool-advfix/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 31.37s
CLIPPY_COLD_EXIT=0
```

```
$ cargo test --workspace
```

Per-binary `test result:` lines (all `ok`), independently recomputed:

| binary | passed | failed | ignored |
|---|---|---|---|
| noren-app lib (`src/lib.rs`) | 79 | 0 | 1 (pre-existing) |
| noren-app bin (`src/main.rs`) | 24 | 0 | 0 |
| `tests/session_adversarial.rs` | 42 | 0 | 0 |
| `tests/session_domain.rs` | 35 | 0 | 0 |
| `tests/session_supervisor.rs` | 29 | 0 | 0 |
| `tests/verify59_independent.rs` | 19 | 0 | 0 |
| noren-pty lib | 10 | 0 | 0 |
| noren-terminal lib + 16 integration binaries | 221 | 0 | 0 |
| doc-tests (3 crates) | 0 | 0 | 0 |

**Total: 79+24+42+35+29+19+10+45+23+20+7+6+3+9+6+6+9+6+17+25+6+7+22+4 = 459
passed, 0 failed, 1 ignored** — matching the handoff's claim exactly (458 at
first review; +1 is the domain lane's `InvalidStatusTransition` Display
assertion brought in by the re-merge). The 1 ignored is the pre-existing
macOS clipboard test (`crates/noren-app/src/clipboard.rs:228`, present on
`origin/main`: `git grep -n "#\[ignore" origin/main -- crates/` matches only
it). No `#[ignore]` remains in the adversarial file — its matches are doc
comments only (verified by grep).

Targeted guard run:

```
$ cargo test --test session_adversarial adv_
running 3 tests
test adv_s1_observe_rejects_non_monotonic_status_regression ... ok
test adv_s3_terminate_unknown_id_signals_unknown_not_fabricated_status ... ok
test adv_s2_dead_records_do_not_accumulate_without_bound ... ok
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 39 filtered out
```

## First-round findings — resolution verified

- **M-1 (MAJOR, branch drift) — RESOLVED.**
  `git merge-base --is-ancestor 25a246c HEAD` → 0 (true) and likewise for
  `65ebc45` (the D-M3-001 `StatusChanged` struct conformance). The
  merge-base of `origin/main` and HEAD is exactly `25a246c`, and HEAD's
  `crates/noren-app/src/session.rs` is **byte-identical** to both
  `origin/main`'s copy and the domain lane tip's copy (`git diff` empty in
  both directions). Content verified directly: rank guard at
  `session.rs:398` (`if status.rank() < descriptor.status.rank() { return
  Err(SessionError::InvalidStatusTransition) }`), `SessionStatus::rank` at
  `session.rs:151`, struct `StatusChanged { id, status }` at `session.rs:239`
  conforming to `docs/coordination/session-api.md:56`. Both fixes coexist;
  none was dropped.
- **N-1 (MINOR, supervisor doc commit) — RESOLVED.**
  `git merge-base --is-ancestor 0ac6512 HEAD` → 0; the updated
  `M3-1b-review.md` is present in the branch diff.
- **N-2 (MINOR, count inaccuracy) — RESOLVED.** Handoff now states 14;
  `grep -c '#[test]' crates/noren-app/src/session_supervisor.rs` → `14`.
  Binary arithmetic also closes: 5 domain + 14 supervisor + 20 fake
  defensive + 3 guards = 42 = the actual `session_adversarial` total.

## 1. Acceptance criteria, one by one

Reconstructed from kimi-a.md §3 (defect statements) plus the fix-lane prompt
contracts.

### ADV-S1 — `observe` permits non-monotonic status regression — **MET**

- **Requirement:** no backwards status move (`Running → Starting`) and no
  resurrection of a dead session (`Exited/Failed → Running`).
- **Implementation (verified at HEAD):** monotonic rank
  (`session.rs:144-157`: Starting 0 < Running 1 < terminal 2) enforced in
  `observe` (`session.rs:386-406`); equal status = no-op `Ok(None)`;
  equal-or-higher rank advances; lower rank →
  `Err(SessionError::InvalidStatusTransition)` without mutation. Documented
  as invariant #5 (`session.rs:39-41`).
- **Guard:** `adv_s1_observe_rejects_non_monotonic_status_regression`
  (`tests/session_adversarial.rs:1406`) asserts `Running→Starting` and
  `Exited→Running` are rejected with status unchanged, and that the
  equal-rank `Failed→Exited` refinement still succeeds.
- **Design note (accepted, same as first review):** the equal-rank rule also
  permits `Exited→Failed` and terminal payload rewrites — documented
  "terminal refinement"; kimi-a required only forbidding regression and
  resurrection. My probe `rv2_p10` additionally verified `Failed→Running` and
  `Failed→Starting` are rejected and a forward rank jump
  (`Starting→Exited`) is permitted.

### ADV-S2 — supervisor retains dead-session records without bound — **MET**

- **Requirement:** repeated spawn/die cycles must not grow the record list
  without limit; the fix must state the bound.
- **Implementation (verified at HEAD):** `RETAIN_TERMINAL_RECORDS: usize = 16`
  (`session_supervisor.rs:75`), `retire_overflow` (`session_supervisor.rs:667-
  682`) invoked from **all four** terminal-transition paths:
  `mark_exited` (693), `mark_failed` (705), `finalize_exited` (716),
  `finalize_failed` (728) — grep confirms exactly 4 call sites; no other
  terminal path exists. Stated bound (`session_supervisor.rs:64-75`,
  358-367): retained records ≤ `running_count() + RETAIN_TERMINAL_RECORDS`.
- **Bound induction re-checked:** `spawn` adds only Running records; every
  terminal transition re-enforces `terminal_count ≤ cap`; victim search can
  only select terminal records (`is_terminal_session`), so a live session can
  never be retired; `sessions`/`order` are always mutated in pairs on every
  insert/remove path (insert 405/412; `forget` 637/638; `retire_overflow`
  676/677). The `None => break` arm is unreachable defense.
- **Guard:** `adv_s2_dead_records_do_not_accumulate_without_bound`
  (`tests/session_adversarial.rs:1464`): 500 spawn-already-dead + `poll`
  cycles, asserts `running_count() == 0`, `len() <= RETAIN_TERMINAL_RECORDS`,
  `len() < 500`. Mutation-verified below.
- The eviction victim is oldest-*spawned*, which is a documented, intentional
  trade-off (handoff §2 design note; rv1 concurred): the death outcome is
  always delivered synchronously (`ReapReport`/return value), only
  post-death lookup by id is bounded.

### ADV-S3 — `terminate` on unknown id fabricates `Failed(PollFailed)` — **MET**

- **Requirement:** no invented status for a session that does not exist;
  "terminated a dead session" must be distinguishable from "id never existed".
- **Implementation (verified at HEAD):** `terminate`/`terminate_now` return
  `Result<SessionStatus, SessionOpError>` (`session_supervisor.rs:529-533`,
  593-596); the unknown arm is `None => return Err(SessionOpError::Unknown)`
  (`session_supervisor.rs:541`). The fabricated
  `Failed { reason: PollFailed }` arm is gone — grep shows `PollFailed`
  remains only in the enum definition, its Display, the *genuine*
  poll-error path (`mark_failed` at line 504), and the unit test asserting
  that genuine path. `shutdown_all` keeps only `Ok` results (610-616).
- **Guard:**
  `adv_s3_terminate_unknown_id_signals_unknown_not_fabricated_status`
  (`tests/session_adversarial.rs:1495`): spawn → terminate → forget →
  re-terminate ⇒ `Err(SessionOpError::Unknown)`. Mutation-verified below.

### Supporting criteria

- **Guards run against the real modules, not the fakes:**
  `tests/session_adversarial.rs:71-75` includes
  `#[path = "../src/session.rs"]` and
  `#[path = "../src/session_supervisor.rs"]`; the guards import from those
  (`use session::...` at 1412, `use session_supervisor::...` at 1471/1502).
  The fakes remain quarantined under `mod fake_domain` / `mod fake_supervisor`
  as the historical attack record, clearly labeled in the file header.
- **No assertion weakened by the `Result` signature change:** read the full
  diff of `tests/session_supervisor.rs` in `2775fb9` line-by-line — every
  change is a mechanical `.expect("...")` at known-valid call sites; zero
  assertion edits.
- **Lease honored:** `git log --oneline origin/main..HEAD --
  crates/noren-app/src/lib.rs crates/noren-app/src/main.rs Cargo.toml
  Cargo.lock` → empty. The fix commit `2775fb9` touches only the leased
  files (`session.rs`, `session_supervisor.rs`, the two session test files)
  plus coordination handoffs. Modules remain un-wired in HEAD's `lib.rs`
  (wiring is main's, restored at landing — see §4).

## 2. Regressions, boundaries, and combinations (reviewer-authored probes)

The author's guards cover single-defect properties. The reviewer wrote a
scratch integration binary (same `#[path]` mechanism, deleted after the run;
`git status` clean at write time) with eleven probes targeting interactions
**the author did not test**:

| # | probe | interaction/boundary under attack | result |
|---|---|---|---|
| P1 | `rv2_p1_shutdown_all_with_full_ring_reports_every_tracked_session` | **eviction ring × `shutdown_all`**: 16 terminal + 1 running; all 17 ids must appear exactly once in the report; ends bounded; idempotent second call | ok |
| P2 | `rv2_p2_shutdown_all_over_cap_with_only_running_sessions` | **cap overflow during one `shutdown_all`**: 20 running, ring fills and churns mid-call; every session still reported (the `if let Ok` skip arm stays unreachable in practice) | ok |
| P3 | `rv2_p3_eviction_never_retires_running_sessions` | ring at cap + one more death: evicted victim is the oldest *terminal*; all 4 running sessions remain queryable as `Running`; `len()` drops to 20 (16 + 4, the bound tight) | ok |
| P4 | `rv2_p4_poll_reports_every_death_even_when_ring_churns_mid_pass` | **17 deaths in one `poll` pass** over a full ring: `ReapReport` carries all 17 with correct codes even though records are evicted during the same pass | ok |
| P5 | `rv2_p5_evicted_id_is_unknown_for_every_followup_operation` | **ADV-S2 × ADV-S3 combination**: an id evicted by the cap is `None` for `status`, `Err(Unknown)` for `terminate_now`/`forget`/`select`; the retained 16 remain `Exited` | ok |
| P6 | `rv2_p6_error_channels_stay_distinct_under_ring_pressure` | full ring + live session: `forget(live)`=StillRunning, `select(dead)`=NotRunning, `select(live)`=Ok, selection cleared exactly when the selected session goes terminal | ok |
| P7 | `rv2_p7_spawn_exhaustion_records_nothing_and_preserves_ring` | hostile input: 50 spawns against an exhausted spawner ⇒ all `Err(SpawnFailed)`, `len()` unchanged, no selection side effects | ok |
| P8 | `rv2_p8_selection_never_points_at_a_dead_or_missing_session` | 30 spawn/poll steps with staggered deaths: after every step, any selection resolves to `Running`; after all deaths, selection `None`, `len() ≤ 16` | ok |
| P9 | `rv2_p9_domain_observe_after_close_is_unknown_not_transition_error` | error-channel precedence: closed id ⇒ `UnknownSession`, never `InvalidStatusTransition` | ok |
| P10 | `rv2_p10_domain_every_resurrection_form_is_rejected` | `Exited→Starting`, `Exited→Running`, `Failed→Running`, `Failed→Starting` all rejected; forward rank jump `Starting→Exited` allowed; equal-rank payload rewrite allowed | ok |
| P11 | `rv2_p11_domain_mass_create_close_leaves_no_residue` | 1000 create/close cycles ⇒ empty registry, no selection | ok |

```
test result: ok. 30 passed; 0 failed; 0 ignored; 0 measured  (11 probes + in-scope module unit tests)
```

The one case the handoff hedges (`shutdown_all`: "ids auto-retired since the
snapshot are omitted") was re-analyzed and probed (P1/P2): the eviction victim
is the oldest terminal record, and because `terminate` processes the snapshot
in insertion order, any victim is either already reported earlier in the same
call or is the session being reported itself (finalize makes it terminal
before `retire_overflow` runs, so it can be its own victim). No tracked
session is ever lost from a single `shutdown_all` result — 17/17 and 20/20
reported empirically. The `Err`-omitting arm is unreachable defense.

## 3. Panics, resource leaks, unbounded growth

- **No `todo!`/`unimplemented!`/`panic!`/`unreachable!`** in either module
  (grep). The domain's single `.expect("session id space exhausted")` on
  `checked_add` is a documented, unreachable u64-exhaustion guard.
- **Unbounded growth:** the ADV-S2 cap closes the reported class; probes
  P3/P4/P8 pin the bound empirically under churn; `sessions`/`order` shrink
  together on every retirement path (no orphaned entries either side);
  `ReapReport` is per-`poll` and returned by value; child handles are dropped
  (`child = None`) on every terminal path. Domain registry growth is
  caller-bounded by contract (`Close` removes; probe P11: 1000 cycles leave
  nothing).
- **`retire_overflow` cost:** O(n) `terminal_count` scan inside a `while`,
  each iteration O(n) find + O(n) retain — worst case O(n²), but n ≤
  running + 17 is cap-bounded, so effectively constant. Not a defect.
- **Loop termination:** each iteration removes exactly one record or `break`s;
  no infinite loop possible.
- **Hostile input probed:** exhausted spawner ×50 (P7), cap+ deaths in one
  pass (P4), cap+ running in one `shutdown_all` (P2), observe-after-close
  (P9). No panic, no leak, no stuck `Running`, no dangling selection found.
- **Known deferred divergence (not a defect of this branch):** the supervisor
  mints ids with `wrapping_add` vs the domain's panicking `checked_add`
  (kimi-a §4; handoff §7) — unreachable below 2^64 spawns, documented for the
  serial integration commit.

## 4. Unintended deletions

```
$ git diff --stat origin/main...HEAD
 crates/noren-app/src/session_supervisor.rs    | 1191 +++++++++++++++++++
 crates/noren-app/tests/session_adversarial.rs | 1518 +++++++++++++++++++++++++
 crates/noren-app/tests/session_supervisor.rs  |  376 ++++++
 docs/coordination/handoffs/glm-advfix.md      |  303 +++++
 docs/coordination/handoffs/glm-b.md           |  303 +++++
 docs/coordination/handoffs/kimi-a.md          |  300 +++++
 docs/coordination/reviews/M3-1b-review.md     |  240 ++++
 docs/coordination/reviews/M3-ADVFIX-review.md |  378 ++++++
 8 files changed, 4609 insertions(+)
```

**Purely additive** (`git diff --numstat` shows 0 deletions in all 8 files).
The internal deletions of the fix commit `2775fb9` (2217+/44−) were read
line-by-line: they remove only the fabricated `Failed(PollFailed)` arm, the
superseded no-cap retention docs, and adapt three test call sites to the
`Result` signature — every deletion intentional, no assertion weakened.

**Landing-order verification (beyond the three-dot diff):** the branch is 6
commits behind `origin/main` (PR #74 domain merge, PR #75 crate-root wiring,
integration docs). A dry-run landing merge was executed and aborted:

```
$ git merge --no-commit --no-ff origin/main
Automatic merge went well; stopped before committing as requested
$ grep -n "pub mod session" crates/noren-app/src/lib.rs
15:pub mod session;
$ git merge --abort   # clean, tree restored
```

Zero conflicts; main's `pub mod session;` wiring and main's
`tests/session_domain.rs` survive intact (the branch touches neither since
the merge-base, so main's versions win automatically). Note for the
integrator: land by merging (branch→main or main→branch) — do **not**
tree-replace or squash-by-file-copy, which would drop main's `lib.rs` wiring
(the direct `git diff origin/main HEAD` shows that 1-line delta because HEAD
predates PR #75). Standard practice; no action beyond a normal merge.

## 5. Noren/Zellij boundary (ADR 0003) — **clean**

`git diff origin/main...HEAD | grep -inE '^\+.*(zellij|\bpane\b|\btab\b|\bsplit\b|layout)'`
matches only documentation prose (module docs stating the boundary, the phrase
"ownership split" meaning registry/supervisor ownership divide, and review
prose). The introduced types are exclusively process-lifecycle state: opaque
`SessionId(u64)`, `SessionStatus`, `SessionFailure`, `SessionDescriptor`,
`SessionEvent`, `SessionError`, in-memory `SessionRegistry`, and
`SessionSupervisor` with `Child`/`Spawner` seams plus a test-only mock
(`cfg(test)`). No pane, tab, layout tree, or split type is introduced; no
Zellij internal layout is read or persisted; nothing renders.

## 6. Do the tests actually test the behavior? (mutation testing)

Four mutations, each reverted immediately after the run
(`git checkout -- <file>`; tree verified clean):

| mutation | expected | real result |
|---|---|---|
| M1: `session.rs:398` guard → `if false && …` | `adv_s1` fails | **FAILED** — panicked at `tests/session_adversarial.rs:1423` ("observe(Starting) must not regress a Running session") |
| M2: all four `self.retire_overflow();` call sites disabled (grep confirmed 4) | `adv_s2` fails | **FAILED** — panicked at `tests/session_adversarial.rs:1482` |
| M3: unknown arm → `Ok(SessionStatus::Failed { reason: SessionFailure::PollFailed })` | `adv_s3` fails | **FAILED** — panicked at `tests/session_adversarial.rs:1513` |
| M4: eviction victim search `order.iter()` → `order.iter().rev()` (evict newest terminal instead of oldest) | should fail | **PASSES — no shipped test detects it** (see finding N-1) |

M1–M3: all three regression guards are load-bearing; each fails with a message
naming the exact guarded property. M4 (a reviewer-invented mutation): reversing
eviction order — which would defeat the ring's stated purpose (a just-died
session's record vanishes instantly while ancient dead records linger) even
though the cap itself still holds — passes the entire shipped suite
(`session_adversarial` 42/42, `session_supervisor` 29/29). The reviewer's own
probe `rv2_p5` fails against it immediately, confirming the gap is in the
suite, not in the behavior.

## Findings

### N-1 (MINOR) — eviction order / ring membership is not pinned by any shipped test

- **Where:** `crates/noren-app/src/session_supervisor.rs:669-673` (victim
  selection); coverage gap in `crates/noren-app/tests/session_adversarial.rs`
  (`adv_s2_*` only asserts the `len()` bound, 1482-1492).
- **Reproduction:** apply mutation M4 above; `cargo test --workspace` still
  reports 459 passed / 0 failed. A newest-first eviction would silently break
  the retention contract's purpose (recent outcomes become unreadable by id
  immediately after death) while every shipped assertion stays green.
- **Expected vs actual:** expected — a mutation contradicting the documented
  retention purpose ("kept long enough for a caller to read the outcome by
  id", `session_supervisor.rs:361-363`) fails the suite; actual — nothing
  fails.
- **Suggested fix (minimal):** adopt two tests from the reviewer's probes
  (deleted after the run, recoverable from this review): the P5 shape
  (oldest id evicted, evicted id `Unknown` for every op, newest 16 retained
  and `Exited`) and the P4 shape (a full pass of cap+ deaths reports every
  transition). Both are small and compile against the real modules via the
  existing `#[path]` includes. The first-round review made the same
  suggestion (its P1/P2); it remains the only shipped-suite blind spot found.

### N-2 (MINOR) — the authoritative task spec does not exist; its lease-conflict decision was never resolved (fleet governance)

- **Where:** fleet repo `noren-fleet-private`: no `state/tasks/M3-ADVFIX.md`
  in any history (`git log --all -- state/tasks/M3-ADVFIX.md` → empty);
  `state/decision-packets/M3-ADVFIX-lease_conflict.md` says "Cannot dispatch:
  task declares no file lease" and its **State section is empty** — no
  recorded decision, yet the lane ran.
- **Impact:** this review (like the first) had to reconstruct acceptance
  criteria from surrogates. The branch itself is not at fault: it honored the
  lease declared in its dispatch prompt (verified in §1, no forbidden file
  touched), and the surrogate criteria are all met. The gap is that a task was
  dispatched and merged while its own decision packet records an unresolved
  dispatch blocker.
- **Suggested fix (minimal, for the orchestrator, not this branch):** record
  the lease decision in the packet's State section and file the spec for
  auditability; or note in `decisions.md` that M3-ADVFIX proceeded under the
  prompt-declared lease.

## Areas verified sound (no findings invented)

- All four terminal-transition paths call `retire_overflow` (exactly 4 call
  sites; no fifth terminal path exists); `sessions`/`order` co-mutation on
  every insert/remove path; the victim can never be a running or selected
  session (selection is cleared on every terminal transition — probe P8 over
  30 steps).
- `shutdown_all` never loses a tracked session from a single call's result
  (analysis in §2 + probes P1/P2); its `Err`-omitting arm is unreachable
  defense. The second call is idempotent and reports the retained terminal
  records (fast path, no backend work).
- Error-channel precedence: domain observe-after-close ⇒ `UnknownSession`
  (probe P9); supervisor distinguishes `Unknown`/`NotRunning`/`StillRunning`
  under ring pressure (probe P6).
- Spawner exhaustion records no session and leaves selection untouched
  (probe P7; matches the merged-lane `spawn_failure_records_no_session`).
- First-round M-1/N-1/N-2 all resolved (evidence at top of this review).
- The two deferred items (id-minting `wrapping_add` divergence;
  selection-on-observe domain/supervisor divergence) are documented in both
  handoffs as integration-time work, unreachable or by-design pending the
  D-M3-001 type unification; the reviewer concurs they are not defects of
  this branch.

## Test-balance note

The shipped suite is property-focused (3 guards + 20 fake defenses + the
merged-lane 35 + 29). It does not pin eviction order, ring membership, or
multi-death-per-pass report completeness; reviewer probes P4/P5 (and P1/P2)
cover those and caught mutation M4 where the suite did not. Adopting them is
finding N-1's suggested fix.

---
*Second-round reviewer ran every command quoted above on 2026-08-07 in the
`pool-advfix` worktree at `8c5a04b5699d`. All four mutations and the probe
file were reverted/deleted and the dry-run merge aborted before this commit
(`git status --short` clean at write time). The first-round review remains in
history at `33898db`.*
