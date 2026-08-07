# Re-review — M3-1b session lifecycle supervisor (independent)

- Reviewed branch: `agent/m3-session-supervisor`
- Reviewed head SHA: `2686956b93d9024fadcd0f3dc3367ae7a26c17ce` (fix-up commit
  `2686956`; code commits `e4d6479`, review/handoff commits `79dff71`, `21d4b49`)
- Base: `origin/main` at `1d329a51582a937c37e5357e21d9a37eb49079bc`
  (`git merge-base origin/main HEAD` confirms)
- Task authority: `state/tasks/M3-1b.md` (fleet repo `noren-fleet-private`)
- Author handoff: `docs/coordination/handoffs/glm-b.md` (§8 documents the fixes)
- Reviewer: independent; did not author the code under review. This RE-review
  supersedes the earlier review of `79dff71` (which found 1 MAJOR + 5 MINORs);
  that review is void and every claim below was re-derived from the current head.

## Verdict

**FINDINGS** — 0 blockers, 0 majors, 1 minor. All three gates pass. All four
acceptance criteria and all four required tests are met. Every prior finding is
genuinely resolved: MAJOR 1 and MINORs 2/3 are fixed with regression-catching
tests (proven by mutation, below); MINORs 4/5/6 are documented exactly as the
agreed disposition stated. The single new finding is a non-blocking
documentation-clarity nit (mixed-pass ordering semantics of `ReapReport`).
No ADR 0003 violation, no unintended deletions, no panic/leak/unbounded-growth
surface beyond the retention policy that is now documented on the type.

## Gates (real output)

Toolchain: `rustc 1.88.0 (6b00bc388 2025-06-23)` via `rust-toolchain.toml`
(channel `1.88.0`, target `aarch64-apple-darwin`).

```
$ cargo fmt --all -- --check
$ echo $?
0
```
(No output — format clean.)

```
$ cargo clean -p noren-app          # force a fresh check, not cache
$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (.../pool-m3b/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.76s
$ echo $?
0
```
(Deliberately re-ran after `cargo clean -p noren-app` because the first run was
fully cached; no warnings, no errors on the fresh check.)

```
$ cargo test --workspace
```
All 25 `test result:` lines are `ok`. Per-binary breakdown:

| binary | passed | failed | ignored |
|---|---|---|---|
| noren-app lib (`src/lib.rs`) | 79 | 0 | 1 (pre-existing) |
| noren-app bin (`src/main.rs`) | 24 | 0 | 0 |
| `tests/session_supervisor.rs` (the lane's target) | 29 | 0 | 0 |
| `tests/verify59_independent.rs` | 19 | 0 | 0 |
| noren-pty lib | 10 | 0 | 0 |
| noren-terminal lib | 45 | 0 | 0 |
| noren-terminal integration (16 targets) | 176 | 0 | 0 |
| doc-tests (3 crates) | 0 | 0 | 0 |

**Total: 382 passed, 0 failed, 1 ignored.** The 1 ignored test is the
pre-existing `#[ignore = "touches the real macOS system clipboard"]` in
`crates/noren-app/src/clipboard.rs:228`, present identically on `origin/main`
(verified with `git show origin/main:crates/noren-app/src/clipboard.rs`).
The lane target's 29 = 14 inline unit tests + 15 integration tests (12
original + 3 added by the fix-up), matching the handoff §4/§8 claims exactly.
Baseline check: `origin/main` at `1d329a5` has 353 passing; 353 + 29 = 382,
so the lane adds exactly its tests and breaks nothing.

## Prior findings — genuinely resolved, verified by mutation

The fix-up's own claims were not trusted; each was re-verified by mutating the
current head and re-running the committed suite. All mutations were reverted
afterwards (`git status --short` clean).

| prior finding | fix claimed | mutation applied at head | committed suite result | verdict |
|---|---|---|---|---|
| MAJOR 1 — `poll` delivered HashMap order | iterate `self.order` + 10-session order test | revert `poll` to `self.sessions.iter()` | `poll_reports_multi_session_transitions_in_insertion_order` FAILS (28 passed; 1 failed) | genuinely fixed |
| MINOR 2 — shared deadline untested | mock records deadlines + all-equal assertion | move `deadline` computation inside the loop (`n × deadline` regression) | `shutdown_all_feeds_one_shared_deadline_to_every_child` FAILS (28 passed; 1 failed) | genuinely fixed |
| MINOR 3 — forget+shutdown_all untested | `forget_then_shutdown_all_omits_the_forgotten_id` | delete `order.retain` in `forget` | `forget_then_shutdown_all_omits_the_forgotten_id` FAILS (28 passed; 1 failed) | genuinely fixed |
| MINOR 4 — unknown-id `Failed(PollFailed)` | documented | n/a (documentation disposition) | rustdoc at `src/session_supervisor.rs:506-511` matches probe-verified behavior | resolved as agreed |
| MINOR 5 — deadline advisory vs `Drop` | documented | n/a (documentation disposition) | "Drop semantics (integration note)" at `src/session_supervisor.rs:235-246` | resolved as agreed |
| MINOR 6 — unbounded retention | documented | n/a (documentation disposition) | "Record retention" section at `src/session_supervisor.rs:345-352` | resolved as agreed |

One extra mutation for the core invariant (independent of prior findings):
suppressing the `Exited` transition in `poll` (stuck `Running`) makes **9
committed tests fail** (20 passed; 9 failed) — the central contract is
behavior-bound, not documented into existence.

## Acceptance criteria (one by one)

1. **Supervisor owns child handles; the registry never does. — MET.**
   `SupervisedSession.child` (`src/session_supervisor.rs:333-336`) is the only
   strong reference to `Box<dyn Child + Send>`. Every terminal transition drops
   it: `mark_exited` (:627), `mark_failed` (:638), `finalize_exited` (:649),
   `finalize_failed` (:660). No registry exists on this branch (M3-1a
   unmerged), the module is not declared in `lib.rs` (grep: no reference in
   `crates/noren-app/src/lib.rs` or `main.rs`), so nothing else can hold a
   handle. Probe P5 (below) confirms a terminal session's handle is gone:
   `terminate` after poll-recorded failure performs zero backend calls.
2. **A dead child produces Exited/Failed rather than a stuck Running. — MET.**
   Verified by mutation (9 failures when the transition is suppressed) and by
   probes: unprompted exit, poll backend error, shutdown hard error, shutdown
   timeout, elapsed deadline, deadline boundary (`Instant::now()`), and even
   the defensive handle-missing branch (`src/session_supervisor.rs:479-481`).
   Every path lands terminal.
3. **Termination reaps the child and is idempotent. — MET.**
   `terminate_reaps_and_second_call_is_a_no_op` asserts `shutdown_count == 1`
   and stable poll counts after a second call; probe P4 extends this to
   `shutdown_all`'s second pass (zero additional backend calls per child).
4. **Until M3-1a lands, only a fake process model and a failure matrix exist.
   — MET.** `docs/coordination/decisions/` does not exist on this branch
   (D-M3-001 absent); the STUB block (`src/session_supervisor.rs:64-149`) is
   clearly marked; the mock + tests are the only executable surface; the module
   is not wired into `lib.rs` (forbidden by the lease).

## Required tests — present and behavior-bound

- spawn then Running via fake supervisor: `spawn_assigns_unique_ids_and_selects_newest` (inline).
- child crash surfaces as Failed/Exited: `poll_surfaces_unprompted_exit_as_exited_not_running`,
  `unprompted_death_surfaces_as_exited_within_one_poll`, plus the poll-error pair.
- double terminate idempotent, reaps once: `terminate_reaps_a_running_child_and_is_idempotent`,
  `terminate_reaps_and_second_call_is_a_no_op` (both assert backend call counts).
- stale session detected: the unprompted-death tests (child dies with no status
  update; one `poll` surfaces it).

## Interactions the author did not test (reviewer probes, temporary — removed)

A throwaway `tests/zz_review_probe.rs` (10 tests) was written, run, and deleted
before the review commit. Three probes failed on their first draft and all
three were flaws in the probes, not the code — recorded here for honesty:
selecting the victim instead of the keeper; expecting `shutdown_count == 0`
when `terminate` kills a child that died unobserved (the supervisor cannot know
without polling; the natural exit code still survives, which is the real
invariant); miscounting records after an extra `forget`. After fixing the
probes, **all 10 pass**:

- **P1 mixed pass**: 6 sessions, alternating exit/fail, one `poll` → `exited()`
  and `failed()` each in insertion order, all 6 terminal, `running_count == 0`.
- **P2 unrelated death preserves selection**: non-selected session dies →
  selection stays on the live session.
- **P3 spawn failure preserves state**: failed spawn leaves existing selection
  and records untouched.
- **P4 second `shutdown_all` pass does zero backend work** (per-child
  `shutdown_count` stays 1, poll counts unchanged).
- **P5 terminate after poll-recorded failure**: fast path, zero backend calls,
  recorded status returned.
- **P6 unobserved death then terminate**: natural code `Exited { code: Some(42) }`
  survives the kill path.
- **P7 terminate on a forgotten id** returns the documented defensive
  `Failed(PollFailed)` (matches the new rustdoc, MINOR 4 disposition).
- **P8 forget mid-lifecycle then new spawn**: `order`/`poll`/selection stay
  coherent.
- **P9 shutdown_all reports a pre-terminal status verbatim** (`Exited { code:
  Some(7) }`) in insertion order, then reaps the rest.
- **P10 deadline boundary** `Instant::now()`: `Failed(ReapTimeout)`, no panic;
  generous deadline → `Exited`.

## Panics, leaks, unbounded growth

- No panic surface in production code: the only `.expect()`/`unwrap()` calls
  are inside `#[cfg(test)]` regions (mock lock, test bodies). No indexing, no
  arithmetic that can trap (`fresh_id` uses `wrapping_add`; the 2^64 id
  collision is theoretical and was already noted by the prior review).
- No `unsafe` anywhere; the integration target carries `#![forbid(unsafe_code)]`.
- No unbounded buffers beyond the intentional record retention, which is now
  documented on the type ("Record retention", `src/session_supervisor.rs:345-352`).
  `order`/`sessions` grow only with spawn count; `ReapReport` is per-pass and
  bounded by tracked sessions; the mock's `deadlines` vec is test-only and
  bounded by backend call count. Probe suite fed degenerate inputs (elapsed
  deadline, unknown/forgotten ids, empty spawner, alternating fault patterns);
  no panic, no stuck state.

## Unintended deletions / lease

```
$ git diff --name-status origin/main...HEAD
A   crates/noren-app/src/session_supervisor.rs
A   crates/noren-app/tests/session_supervisor.rs
A   docs/coordination/handoffs/glm-b.md
A   docs/coordination/reviews/M3-1b-review.md
$ git diff --diff-filter=D --name-only origin/main...HEAD | wc -l
0
```

Additions only; nothing deleted. Forbidden files (`lib.rs`, `main.rs`,
`Cargo.toml`, `Cargo.lock`, `status.md`) untouched — the module still does not
compile into the library/binary (`cargo build` excludes it; only `cargo test`
exercises it via the `#[path]` include), exactly as the lease requires. The
handoff and this review are coordination artifacts outside the two leased code
paths, required by the workflow. The fix-up commit's own diff touches only the
two leased code files plus the handoff.

## ADR 0003 boundary

Clean. `grep -niE "pane|layout|zellij|\btab\b|split"` across both code files
matches nothing except the phrase "ownership split" in a comment (excluded).
The module owns process lifecycle only; `select` is session focus, which the
task spec assigns to this lane. No Zellij internal layout is introduced, read,
or persisted.

## Findings (new at this head)

### MINOR R1 — `ReapReport`'s "in insertion order" is per-list on mixed passes; docs may promise more

- Location: contract at `crates/noren-app/src/session_supervisor.rs:293-297`
  ("Lists exactly the sessions that left `Running` this pass, **in insertion
  order**"); implementation `mark_exited`/`mark_failed` push into two separate
  `Vec`s (`src/session_supervisor.rs:300-301`).
- Reproduction: 6 sessions, alternating exit/fail, one `poll` (reviewer probe
  P1): `exited() == [id0, id2, id4]`, `failed() == [id1, id3, id5]`.
- Expected vs actual: under a strict reading of the contract sentence a caller
  could expect to reconstruct the single global order `[id0..id5]`; the actual
  API exposes two parallel lists, each internally in insertion order, with no
  accessor for the combined order. Behavior is correct and matches the most
  natural reading of the next sentence ("Exited and Failed are reported
  separately"); this is a doc-clarity gap, not a behavioral defect — a consumer
  that needs global order can still query `status(id)` per id.
- Severity: MINOR, non-blocking. Raised only because the MAJOR fixed in this
  re-review was itself an ordering promise, so the remaining ambiguity is worth
  pinning down at integration time.
- Minimal fix: clarify the rustdoc — "insertion order is preserved *within each
  of the two lists*" — or add a merged-view accessor at D-M3-001 integration.

## Areas checked and found sound

- Idempotency of `terminate` and `shutdown_all` (committed tests + probe P4).
- Selection safety: selection cleared exactly when the selected session
  transitions (all four finalizers) and on `forget`; unrelated deaths leave it
  alone (probe P2); `select` refuses dead/unknown ids.
- The `Ok(())`-shutdown path (`poll_after_shutdown`,
  `src/session_supervisor.rs:561-570`) never leaves a session `Running`, even
  on backend inconsistency.
- Handoff §4 numbers reproduce exactly (382/0/1; 29 lane tests); §8 fix claims
  all hold under mutation.
- No regressions elsewhere in the workspace: all pre-existing targets pass
  unchanged at the lane's head.
