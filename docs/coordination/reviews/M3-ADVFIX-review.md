# Review — M3-ADVFIX adversarial session-lifecycle fixes (independent)

- Reviewed branch: `agent/m3-adv-fixes`
- Reviewed head SHA: `2775fb9f6dbb` (`fix(app): resolve three adversarial
  session-lifecycle defects (ADV-S1/S2/S3)`)
- Base: `origin/main` at `1d329a5`
- Task authority: `state/tasks/M3-ADVFIX.md` — **not found**. Not present on
  this branch, not on `origin/main`, and not anywhere in the fleet repo
  (`git log --all -- state/tasks/M3-ADVFIX.md` → empty; no `state/` directory
  exists). The review therefore reconstructed the acceptance criteria from the
  two authority-surrogate documents the branch itself carries:
  `docs/coordination/handoffs/kimi-a.md` (the adversarial findings §3) and
  `docs/coordination/handoffs/glm-advfix.md` (the fix plan). Where the two
  disagree, the defect descriptions in kimi-a.md were treated as the
  requirement.
- Author handoff reviewed: `docs/coordination/handoffs/glm-advfix.md` — treated
  as claims to verify, not as evidence.
- Reviewer: independent (did not author the code under review). The branch was
  already checked out in the `pool-advfix` worktree at exactly `2775fb9`; all
  commands below were run there on 2026-08-07. Toolchain: `rustc 1.88.0`,
  `cargo 1.88.0` (workspace `rust-toolchain.toml`).

## Verdict

**FINDINGS** — 0 blockers, 1 major, 2 minors. All three defects (ADV-S1/S2/S3)
are fixed, all three regression guards are mutation-sensitive (each fails when
its fix is removed), the gates pass (fmt clean, clippy `-D warnings` clean,
458 passed / 0 failed / 1 pre-existing ignored — exactly matching the handoff
claim), the diff against `origin/main` is purely additive (0 deletions), and
ADR 0003 is respected. The major finding is **branch drift**: the domain lane
landed a follow-up fix to the same function this branch edits
(`SessionEvent::StatusChanged` in `observe`) *after* being merged here, so a
naive landing order risks silently dropping either fix. The minors are the
supervisor lane's follow-up doc commit not being merged, and one off-by-one
count inaccuracy in the handoff. No panics, leaks, or unbounded growth found;
eight reviewer-authored combination probes beyond the author's own tests all
pass.

## Gates — commands actually run, real output

```
$ cargo fmt --all -- --check
$ echo $?
0
```
(No output — format clean.)

```
$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (/Users/yoshinagatatsuya/Documents/apps/noren-worktrees/pool-advfix/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.63s
$ echo $?
0
```

Note on cache honesty: the first clippy invocation in this review finished in
0.17s from cache. To rule out a stale cache from the author's run, the reviewer
`touch`ed `session.rs`, `session_supervisor.rs`, and all three session test
files and re-ran; clippy re-checked with exit 0 and no diagnostics. Content-
identical re-runs reuse fingerprints, so additionally the reviewer's own probe
binary (below) forced a genuine fresh `Compiling noren-app` of both modules via
the same `#[path]` mechanism, also clean.

```
$ cargo test --workspace
```
Per-binary `test result:` lines (all `ok`), in run order:

| binary | passed | failed | ignored |
|---|---|---|---|
| noren-app lib (`src/lib.rs`) | 79 | 0 | 1 (pre-existing) |
| noren-app bin (`src/main.rs`) | 24 | 0 | 0 |
| `tests/session_adversarial.rs` | 42 | 0 | 0 |
| `tests/session_domain.rs` | 34 | 0 | 0 |
| `tests/session_supervisor.rs` | 29 | 0 | 0 |
| `tests/verify59_independent.rs` | 19 | 0 | 0 |
| noren-pty lib | 10 | 0 | 0 |
| noren-terminal lib/bin + integration tests (17 binaries) | 221 | 0 | 0 |
| doc-tests (3 crates) | 0 | 0 | 0 |

**Total: 458 passed, 0 failed, 1 ignored** — independently recomputed from the
raw `test result:` lines (79+24+42+34+29+19+10+221 = 458), matching the
handoff claim exactly. The 1 ignored is
`crates/noren-app/src/clipboard.rs:228` (`touches the real macOS system
clipboard`), present on `origin/main` — verified pre-existing via
`git grep -n ignore origin/main -- crates/noren-app/src/clipboard.rs`.

Targeted guard run:

```
$ cargo test --test session_adversarial adv_
running 3 tests
test adv_s1_observe_rejects_non_monotonic_status_regression ... ok
test adv_s3_terminate_unknown_id_signals_unknown_not_fabricated_status ... ok
test adv_s2_dead_records_do_not_accumulate_without_bound ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 39 filtered out
```

## 1. Acceptance criteria, one by one

Reconstructed from kimi-a.md §3 (defect statements) and glm-advfix.md (fix
contracts).

### ADV-S1 — `observe` permits non-monotonic status regression — **MET**

- **Requirement (kimi-a §3):** status must not move backwards
  (`Running → Created/Starting`), and most importantly no resurrection of a
  dead session (`Exited → Running`).
- **Implementation verified:** `SessionStatus::rank`
  (`crates/noren-app/src/session.rs:164` — Starting 0 < Running 1 < terminal 2)
  and the guard in `observe` (`session.rs:416-421`): equal status is a no-op,
  lower rank returns `Err(SessionError::InvalidStatusTransition)`, equal-or-
  higher rank advances. Documented as module invariant #5 (`session.rs:40-43`)
  and on `SessionStatus`/`observe` docs.
- **Evidence:** guard `adv_s1_observe_rejects_non_monotonic_status_regression`
  asserts both `Running→Starting` and `Exited→Running` are rejected and leave
  status unchanged, while `Failed→Exited` refinement succeeds (the existing
  `observe_records_failure_and_exit_statuses_with_payloads` test depends on
  that refinement — confirmed still passing).
- **Design note (accepted, not a finding):** the equal-rank rule also permits
  `Exited→Failed` and exit-code rewrites (`Exited{Some(0)}→Exited{Some(1)}`).
  Reviewer probed this explicitly
  (`probe_domain_terminal_to_terminal_cross_variant_is_permitted_by_design`) —
  it works as documented ("refine one terminal report into another"), and
  kimi-a's requirement only forbade regression and resurrection. If the council
  later wants first-terminal-report-is-truth, that is a conscious contract
  change, not a defect here.

### ADV-S2 — supervisor retains dead-session records without bound — **MET**

- **Requirement (kimi-a §3):** repeated spawn/die cycles must not grow the
  supervisor's record list without bound; the fix must "state the bound".
- **Implementation verified:** `RETAIN_TERMINAL_RECORDS: usize = 16`
  (`crates/noren-app/src/session_supervisor.rs:75`) and `retire_overflow`
  (`session_supervisor.rs:667-682`), invoked from **all four** terminal
  transition paths: `mark_exited` (693), `mark_failed` (705), `finalize_exited`
  (719), `finalize_failed` (verified in diff; grep confirms 4 call sites and no
  other terminal path exists). Stated bound
  (`session_supervisor.rs:64-75`, struct docs 358-369): retained records ≤
  `running_count() + RETAIN_TERMINAL_RECORDS`.
- **Bound induction checked by reviewer:** spawn adds only Running records and
  never changes the terminal count; every terminal transition re-enforces
  `terminal_count ≤ cap`. Hence total ≤ running + cap at all times.
  `retire_overflow`'s victim search can only select terminal records
  (`is_terminal_session`), so it can never retire a live session; the
  `None => break` arm is unreachable defense. `sessions` and `order` are always
  mutated in pairs (insert: 405/412; remove: 637/638 in `forget`, 676/677 in
  `retire_overflow`) — no divergence leak.
- **Evidence:** guard `adv_s2_dead_records_do_not_accumulate_without_bound`
  (500 spawn-already-dead + poll cycles, asserts `len() <= 16` and `< 500`);
  mutation-verified below. Reviewer probes additionally confirm the ring keeps
  exactly the 16 most recent records, evicted ids lose all queryability, and
  running sessions are never evicted (probes P2, P4 below).

### ADV-S3 — `terminate` on unknown id fabricates `Failed(PollFailed)` — **MET**

- **Requirement (kimi-a §3):** no invented status for a session that does not
  exist; caller must be able to distinguish "terminated a dead session" from
  "id never existed".
- **Implementation verified:** `terminate`/`terminate_now` now return
  `Result<SessionStatus, SessionOpError>` (`session_supervisor.rs:529-533`,
  593-596); the unknown arm is `None => return Err(SessionOpError::Unknown)`
  (`session_supervisor.rs:541`) — the fabricated
  `Failed { reason: PollFailed }` arm is gone (confirmed absent from the whole
  file). `shutdown_all` keeps only `Ok` results (610-616).
- **Evidence:** guard
  `adv_s3_terminate_unknown_id_signals_unknown_not_fabricated_status`
  (spawn → terminate → forget → terminate again ⇒ `Err(Unknown)`); mutation-
  verified below. Reviewer probe P1 extends this to the ADV-S2×ADV-S3
  combination: an id **auto-retired by eviction** is also honestly `Unknown`
  for `terminate_now`, `status`, `forget`, and `select`.

### Supporting criteria

- **Guards compile against the real modules, not the fakes:** verified —
  `tests/session_adversarial.rs:71-75` includes
  `#[path = "../src/session.rs"]` and
  `#[path = "../src/session_supervisor.rs"]`; the three `adv_s*` guards import
  from those modules (e.g. `use session::{SessionError, ...}` at line 1412,
  `use session_supervisor::{RETAIN_TERMINAL_RECORDS, ...}` at line 1471). The
  fakes remain only under `mod fake_domain` / `mod fake_supervisor` with their
  20 defensive tests.
- **No adversarial `#[ignore]` tests left:** verified — the only `#[ignore]` in
  the workspace is the pre-existing macOS clipboard test; all matches in
  `session_adversarial.rs` are doc comments.
- **`terminate` call-site fallout:** verified in the commit diff — every
  existing caller gained `.expect("...")` with unchanged assertions;
  `shutdown_all`'s signature is unchanged; no assertion was weakened (read
  line-by-line in `git show 2775fb9`).
- **Unchanged files claim:** `lib.rs`, `main.rs`, `Cargo.toml`, `Cargo.lock`
  do not appear in `git diff --name-only origin/main...HEAD` — verified. The
  modules remain un-wired from `lib.rs` (confirmed: no `session` reference in
  `lib.rs`/`main.rs`).

## 2. Regressions, boundaries, and combinations (reviewer-authored probes)

The author's guards cover single-defect properties. The reviewer wrote a
scratch integration test (same `#[path]` mechanism, deleted after the run) with
eight probes targeting interactions and boundaries the author did not test:

| # | probe | interaction/boundary under attack | result |
|---|---|---|---|
| P1 | `probe_evicted_id_is_unknown_for_all_followup_ops` | **ADV-S2 × ADV-S3**: fill ring to cap+1; the evicted id must be `None` for `status`, `Err(Unknown)` for `terminate_now`/`forget`/`select` — no state invented for an evicted session | ok |
| P2 | `probe_ring_keeps_exactly_the_most_recent_cap_records` | cap+4 deaths ⇒ `len()==16`, exactly the 4 oldest evicted, remaining 16 queryable and `Exited` | ok |
| P3 | `probe_shutdown_all_over_cap_reports_every_tracked_session` | **ADV-S2 × shutdown_all**: 16 terminal + 5 running ⇒ `shutdown_all()` reports all 21 ids exactly once, all terminal, ends bounded (`len() ≤ 16`), selection cleared, idempotent second call | ok |
| P4 | `probe_eviction_never_retires_a_running_session` | ring at cap + 5 running ⇒ `poll` keeps all 5 `Running`; `len() == 21 == running + cap` (the stated bound, tight) | ok |
| P5 | `probe_reap_report_still_delivers_evicted_deaths_synchronously` | the handoff's design note: a death whose record is immediately evicted is still in the same `poll`'s `ReapReport` — all 17 deaths reported | ok |
| P6 | `probe_spawn_exhaustion_records_nothing_and_is_bounded` | hostile input: 50 spawns against an exhausted spawner ⇒ all `Err(SpawnFailed)`, `len()` unchanged | ok |
| P7 | `probe_domain_terminal_to_terminal_cross_variant_is_permitted_by_design` | documents the equal-rank rule's extent (`Exited→Failed` allowed, resurrection still rejected) | ok |
| P8 | `probe_domain_observe_after_close_is_unknown_not_transition_error` | closed id ⇒ `UnknownSession`, not `InvalidStatusTransition` (error-channel ordering correct) | ok |

```
test result: ok. 27 passed; 0 failed; 0 ignored  (8 probes + in-scope unit tests)
```

Additionally, the reviewer analyzed the one case the author's `shutdown_all`
doc hedges ("ids auto-retired since the snapshot are omitted"): the eviction
victim is always the *oldest* terminal record, iteration is in insertion order,
and a transitioning session is itself terminal at its own position — so the
victim is always already reported (or the very session being reported). The
`if let Ok` arm is unreachable defense, and **no tracked session can be lost
from the results**; probe P3 confirms empirically (21/21 reported).

## 3. Panics, resource leaks, unbounded growth

- **Unbounded growth:** the ADV-S2 cap closes the reported defect; probes P2/P4
  pin the bound empirically. `order` and `sessions` shrink together on every
  retirement path (see §1 evidence) — no orphaned entries either side.
- **`retire_overflow` cost:** `terminal_count()` is an O(n) scan inside a
  `while`, each iteration adds O(n) find + O(n) retain — worst case O(n²).
  Because n ≤ running + 17 is itself bounded by the cap, this is O(1) in
  effect (tiny constant); not a defect, noted for completeness.
- **Loop termination:** `retire_overflow` strictly removes one record per
  iteration or `break`s; no infinite loop possible.
- **Panics under hostile input:** probes P6 (exhausted spawner ×50) and the
  over-cap shutdown (P3) found no panic path. `MockSpawner::failing()` and
  deadline-elapsed paths are covered by the merged lane tests (29/29 in
  `tests/session_supervisor.rs`).

## 4. Unintended deletions

```
$ git diff --shortstat origin/main...HEAD
 11 files changed, 5604 insertions(+)
$ git diff --numstat origin/main...HEAD | awk '{add+=$1; del+=$2} END {print add, del}'
5604 0
```

Purely additive against `origin/main`. The fix commit itself (`git show
--stat 2775fb9`: 2217 insertions, 44 deletions) deletes only the buggy arms it
replaces: the fabricated `Failed(PollFailed)` branch, the old "no cap, grows
without bound" retention docs, and signature adaptation in two test files —
each deletion verified line-by-line in the diff and intentional. The only
deletion in `session.rs` relative to the domain lane's merged snapshot
(`df3afcc`) is one doc sentence rewritten ("unknown-session cases" →
"unknown-session and invalid-transition cases"); no behavioral code removed.

Merge integrity: both owning lanes are fully contained —
`git merge-base --is-ancestor <lane-tip-at-merge> HEAD` holds for the merged
snapshots, and `agent/m3-session-supervisor`'s code tip (`2686956`) is an
exact ancestor. See finding M-1 for the post-merge lane drift.

## 5. Noren/Zellij boundary (ADR 0003) — **clean**

`git diff origin/main...HEAD | grep -inE 'zellij|\bpane\b|\btab\b|\bsplit\b|layout tree'`
matches only documentation prose (e.g. `session.rs:8-9` "carries no pane, tab,
layout, or split notion"; the phrase "ownership split" in
`session_supervisor.rs:19` refers to the registry/supervisor ownership divide,
not a terminal split). The introduced types are exclusively process-lifecycle
state: opaque `SessionId(u64)`, `SessionStatus`, `SessionDescriptor`,
`SessionEvent`, `SessionError`, in-memory `SessionRegistry` (HashMap),
`SessionSupervisor` with `Child`/`Spawner` seams and mock. No pane, tab, layout
tree, or split is introduced; no Zellij internal layout is read or persisted;
nothing renders inside or outside the terminal. The modules are not wired into
`lib.rs`, so the app binary surface is unchanged.

## 6. Do the tests actually test the behavior? (mutation testing)

Three mutations, each reverted after the run (`git checkout -- <file>`):

| mutation | expected | real output |
|---|---|---|
| M1: `if status.rank() < descriptor.status.rank()` → `if false && …` in `session.rs` `observe` | `adv_s1` fails | FAILED — `observe(Starting) must not regress a Running session / left: Ok(Some(StatusChanged)) right: Err(InvalidStatusTransition)` |
| M2: all four `self.retire_overflow();` call sites disabled in `session_supervisor.rs` | `adv_s2` fails | FAILED — `supervisor retained 500 records after 500 spawn/crash cycles; bounded by RETAIN_TERMINAL_RECORDS (16)` |
| M3: unknown arm changed to `Ok(SessionStatus::Failed { reason: SessionFailure::PollFailed })` | `adv_s3` fails | FAILED — `terminate on an unknown id must signal Unknown, not fabricate a status / left: Ok(Failed { reason: PollFailed }) right: Err(Unknown)` |

All three guards are load-bearing: each fails with a message naming the exact
property when its fix is removed. The pre-existing `session_domain` /
`session_supervisor` binaries additionally keep the lane invariants honest
(34 + 29 tests). The fakes' 20 defensive tests still pass against the mirrored
algorithm, and nothing passes against broken code in any configuration the
reviewer constructed.

## Findings

### M-1 (MAJOR) — branch drift: domain lane's follow-up fix to `observe` is not in this branch

- **Where:** `crates/noren-app/src/session.rs` (both branches edit
  `SessionRegistry::observe`); commits `65ebc45` ("fix(app): conform
  SessionEvent::StatusChanged to D-M3-001 struct variant"), `6fc1e39`,
  `0718f56` on `agent/m3-session-domain`.
- **Reproduction:**
  ```
  $ git merge-base --is-ancestor 65ebc45 HEAD   # exits non-zero (NOT an ancestor)
  $ git diff HEAD agent/m3-session-domain -- crates/noren-app/src/session.rs
  # diverges only in SessionEvent::StatusChanged shape (unit vs struct variant)
  # plus this branch's ADV-S1 additions
  ```
  The lane tip is not an ancestor of HEAD. `65ebc45` was committed 03:52, this
  branch's fix at 03:40 — the lane moved *after* being merged here. Neither
  branch is in `origin/main` yet (`git merge-base --is-ancestor 65ebc45
  origin/main` → non-zero), so the hazard lives in landing order, not in this
  branch alone.
- **Expected vs actual:** expected — the branch contains the current content of
  the lane it merged; actual — it contains `agent/m3-session-domain` as of
  `df3afcc`, missing the D-M3-001 `StatusChanged { id, status }` conformance
  fix. Both changes touch the same function (`observe`'s returned event).
  Whichever branch merges into `main` second faces a semantic conflict there;
  a careless or automatic resolution would silently drop either the ADV-S1
  rank guard or the D-M3-001 conformance fix. kimi-a.md §1 records that this
  project already shipped one "fix conforming to a misquoted contract" — the
  same failure class.
- **Suggested fix (minimal):** before landing, merge the current
  `agent/m3-session-domain` tip into this branch, adapt `observe` to return
  `SessionEvent::StatusChanged { id, status }` per `65ebc45`, and re-run the
  three `adv_s*` guards plus `tests/session_domain.rs` (34 tests) to confirm
  both fixes coexist.

### N-1 (MINOR) — supervisor lane's follow-up doc commit not merged

- **Where:** `agent/m3-session-supervisor` tip `0ac6512` ("docs(coordination):
  independent re-review of M3-1b session supervisor") is not an ancestor of
  HEAD.
- **Impact:** documentation only; the supervisor *code* tip (`2686956`) is
  fully merged and verified. No behavioral effect.
- **Suggested fix:** fold in when performing the M-1 re-merge, or note for the
  serial integration commit.

### N-2 (MINOR) — handoff count inaccuracy

- **Where:** `docs/coordination/handoffs/glm-advfix.md:195` claims the
  adversarial binary count includes "the domain (5) and supervisor (13) unit
  tests".
- **Actual:** `grep -c '#\[test\]' crates/noren-app/src/session_supervisor.rs`
  → 14 (5 + 14 + 20 fake + 3 guards = 42, the actual binary total). Off by one;
  no behavioral effect; noted because handoff numbers are otherwise exact and
  auditors rely on them.

## Areas verified sound (no findings invented)

- All four terminal-transition paths call `retire_overflow` (grep: exactly 4
  call sites; no fifth terminal path exists).
- `sessions`/`order` co-mutation on every insert/remove path.
- `shutdown_all` cannot lose a tracked session's result (analysis in §2 +
  probe P3); its `Err`-omitting arm is unreachable defense.
- Error-channel precedence in `observe`: unknown-session beats
  invalid-transition (probe P8).
- Spawner exhaustion leaves no partial record (probe P6; matches merged-lane
  `spawn_failure_records_no_session`).
- The two explicitly deferred items (wrapping_add vs checked_add id minting;
  selection-on-observe divergence) are documented in both handoffs as out of
  scope for this task, unreachable (2^64 spawns) or by-design pending the
  D-M3-001 type integration; the reviewer concurs they are not defects of this
  branch.

## Test-balance note

The guard suite is property-focused (3 guards + 20 fake defenses + merged-lane
suites). It does not pin eviction *order* (oldest-first) or ring *membership*
(the most recent 16) directly; the reviewer's probes P1/P2 covered those and
passed. Adopting two of them (P1, P2) into `session_adversarial.rs` would
harden the suite at low cost. Suggestion only, not a finding.

---
*Reviewer ran every command quoted above on 2026-08-07 from the `pool-advfix`
worktree at `2775fb9`; mutation edits and the probe file were reverted/deleted
before this commit (`git status --short` clean at write time).*
