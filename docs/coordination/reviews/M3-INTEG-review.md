# Review — M3 integration lane (branch `agent/m3-integration`, head `7c5e4a1c7997`)

- **Reviewer:** independent review lane (Qwen 3.8 Max, opencode), run from the
  `pool-qwen-rv1-integ` worktree; branch checked out via the `pool-integ`
  worktree (git refuses a second checkout of the same branch).
- **Date:** 2026-08-07
- **Reviewed commit:** `7c5e4a1c799728a2213ffa758d1a93f3bc6b26cc`
  ("feat(app): wire session domain module into crate root (M3 integration)")
- **Author handoff:** `docs/coordination/handoffs/glm-integration.md`

## Verdict

**PASS** — 0 blockers, 0 majors, 2 minors (both process notes, neither requires
a branch change before merge). All acceptance criteria verified against real
command output; the test suite demonstrably catches broken code; no ADR 0003
boundary violation; no unintended deletions.

## Task spec note (read this first)

The review prompt names `state/tasks/M3-INTEG.md` in the fleet repo as the
authority. **That file does not exist.** Actual fleet-repo contents of
`state/tasks/`:

```
M3-1a.md  M3-1b.md  M3-3.md  M3-4.md  M3-ADV-session.md  M3-EXP-zellij.md  TEMPLATE.md
```

The de-facto spec is the integration lane's prompt,
`prompts/glm-integration-m3.md` in the fleet repo. All acceptance criteria
below are taken from that prompt. See finding M-1.

## Gates — real output

Run on `agent/m3-integration` @ `7c5e4a1` (macOS arm64, rustc 1.88.0):

```
$ cargo fmt --all -- --check
(no output)                                   → exit 0

$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (...)           → exit 0, 0 warnings
(cache busted first via touch on session.rs/lib.rs/session_domain.rs
 so this is a genuine re-check, not a cached pass)

$ cargo test --workspace
test result: ok. ... (all targets)            → exit 0
totals (awk over every "test result" line):  passed=387 failed=0 ignored=1
```

`git grep -c '#\[test\]'` on the new files: 29 in `tests/session_domain.rs`,
5 in `src/session.rs` unit tests → the merged branch contributes exactly 34.

**Test-count arithmetic independently re-derived.** I ran
`cargo test --workspace` in a second worktree sitting on `origin/main`
(`1d329a5`):

```
baseline on origin/main: passed=353 failed=0 ignored=1
branch:                  passed=387 failed=0 ignored=1
353 + 34 = 387  ✓  (nothing lost, nothing duplicated)
```

The single ignored test is the pre-existing one in `noren_terminal`'s lib
tests (84 passed, 1 ignored there on both branches).

## Acceptance criteria (from `prompts/glm-integration-m3.md`)

| # | Criterion | Met? | Evidence |
|---|-----------|------|----------|
| 1 | Merge the named M3 branches that exist; say which were skipped | YES | `git merge-base origin/main HEAD` = `1d329a5` = `origin/main`; `git merge-base --is-ancestor a8526b6 HEAD` → yes (fast-forward of `agent/m3-session-domain`). `git ls-remote origin \| grep m3-` shows **only** `refs/heads/agent/m3-session-domain` (`a8526b6…`) — supervisor, sidebar-view, adv-fixes have never been pushed, so "skipped" was the only option and is stated in the handoff table. |
| 2 | Add module declarations + public exports for landed M3 modules | YES | `crates/noren-app/src/lib.rs:15` adds `pub mod session;`. No `#[path]` shim remains anywhere (`git grep '#\[path' -- crates/` → nothing), so the module compiles exactly once, inside the crate. |
| 3 | No module's public API changed to ease wiring | YES | The integration commit touches `lib.rs` (+1 line), the test file's **import root only**, and adds the handoff. `git show 7c5e4a1 -- tests/session_domain.rs` shows only the doc-header and import lines changed; all 29 test bodies byte-identical. `src/session.rs` untouched by the integration commit. |
| 4 | Resolve conflicts keeping both sides' tests | N/A | Fast-forward merge; `conflicts=0` claim is true — the merged branch adds only new files plus its own handoffs/review. |
| 5 | Full workspace builds and every test passes | YES | Gate output above: 387 passed, 0 failed, 1 pre-existing ignored. |
| 6 | Test count ≥ sum of landed branches minus duplicates | YES | 353 + 34 = 387, re-derived, not taken on faith. |
| 7 | ADR 0003: no pane/tab/layout/split types | YES | `git grep -iE 'zellij\|pane\|layout\|split\|\btab\b'` over the new files matches only doc comments disclaiming such types (`session.rs:8-9`). Model conform to D-M3-001 (below). |
| 8 | Handoff committed with `-s`, not pushed | YES | Commit `7c5e4a1` carries `Signed-off-by: ta-061 <…>`; branch not pushed (worktree-only). |

## Contract conformance against canonical D-M3-001

The fleet repo **does** contain the canonical contract at
`state/D-M3-001-session-api.md` (the handoff's claim "the full D-M3-001 file
is not in this repo" refers to the public noren repo and is correct there —
but the coordinator can ratify field names from the fleet copy). Checked
against it:

- `SessionId(u64)` opaque/private ✓; `SessionKind` five variants ✓;
  `SessionStatus` four variants with exact payloads
  (`Exited { code: Option<i32> }`, `Failed { reason: String }`) ✓;
  `SessionDescriptor { id, kind, status, title }` ✓;
  `SessionAction` exactly {Create, Select, Close} ✓;
  `SessionEvent` Created/Selected tuple variants and
  `StatusChanged { id, status }` struct variant ✓ (this was the earlier
  D-M3-001 conformance fix, commit `65ebc45`);
  `SelectedSession = Option<SessionId>` ✓; registry holds no process handles ✓.
- Contract struct-variant fields are written `{ .. }` in D-M3-001, i.e. the
  field names are genuinely **not fixed** by the contract. The implementation's
  `root`/`path`/`target`/`name` choices therefore cannot conflict with it; the
  handoff's escalation item 1 is moot — but the coordinator should still record
  the chosen names into the contract before downstream lanes hard-code them.
- Contract invariant 2: "No session count cap is implied, but the registry must
  not grow without bound from repeated create/close cycles; ids may be reused
  only if that cannot alias a live session." Satisfied: `close_events` removes
  the map entry (`session.rs:429-439`), ids are monotonically minted and never
  reused (`session.rs:321-338`, stronger than required), no event history kept.

## Attempts to break it

**Mutation testing (3 mutants, all reverted after):**

1. `create` records `Running` instead of `Starting` (`session.rs:333`):
   `a_newly_created_session_is_starting_not_running … FAILED`
   ("create must not infer a running status"). **Caught.**
2. `close_events` never clears the selection (`session.rs:429-439`):
   `closing_the_selected_session_clears_the_selection`,
   `closing_the_only_session_leaves_no_selection`, and
   `a_full_session_lifecycle_runs_without_any_child_process` all `FAILED`.
   **Caught.**
3. `observe` drops the equal-status no-op check (`session.rs:373`):
   `observe_emits_status_changed_only_when_it_differs` and
   `observing_the_current_status_is_a_no_op` `FAILED`. **Caught.**

(Reviewer process note: my first run of mutant 3 used the name filter
`session`, which only selects 13 of the 29 integration tests and missed the
`observe_*` tests; rerunning on the full `--test session_domain` target caught
it. The suite itself is fine.)

**Adversarial/combination probes the author did not test** (scratch test file,
run, then deleted; tree verified clean afterwards):

- select → observe(Failed) on a `Project` session + observe(Running) on an
  `Ssh` session → close the selected one → recreate: emits
  `[Closed(id), Selected(None)]` in order, selection cleared, stale id errors
  on both `observe` and `select` with `UnknownSession`, fresh id distinct.
- Status regression `Running → Exited{Some(0)} → Running → Exited{None} →
  Failed{""}`: all accepted via `observe` and reflected exactly. This is
  contract-correct ("status is reported, never inferred"); no hidden state.
- Degenerate: ops on a closed id all return `UnknownSession` (no panic);
  **50,000 create/close cycles**: `len()` tracks exactly, `sessions()` stays
  sorted by id, registry ends empty with no selection, no panic, no retained
  growth. Memory is bounded by the live set since nothing else is stored.
- 32-session interleaved select/close churn: single-selection invariant holds
  throughout; closing never leaves a dangling selection.

**Boundary / misc checks:**

- No production code consumes the module yet (`main.rs`'s `session` hits are a
  local PTY variable, unrelated) — consistent with the handoff's
  "what could not be verified"; not a defect of this branch.
- `cargo doc -p noren-app` emits 2 warnings, but both (`clipboard.rs:49`,
  `config.rs:281`) reproduce identically on `origin/main` in the baseline
  worktree — pre-existing, not introduced here. `session.rs` itself, despite
  heavy intra-doc linking, adds zero doc warnings.
- Unintended deletions: `git diff --stat origin/main...HEAD` = 6 files,
  **1586 insertions(+), 0 deletions(-)**. Nothing removed; the integration
  commit's only removals are the 3 lines of the `#[path]` shim it replaced.

## Findings

### BLOCKER — none

### MAJOR — none

### MINOR

**M-1 (process): the review's authority file does not exist.**
The review prompt points at `state/tasks/M3-INTEG.md` in the fleet repo; only
`M3-1a/1b/3/4`, `M3-ADV-session`, `M3-EXP-zellij`, and `TEMPLATE` exist there.
Expected: an authoritative task spec for the integration lane.
Actual: criteria had to be taken from `prompts/glm-integration-m3.md`.
Suggested fix: the coordinator creates `state/tasks/M3-INTEG.md` (or amends
review prompts to cite the prompt file). No branch change needed.

**M-2 (process): test file edited outside the literal integration lease.**
`crates/noren-app/tests/session_domain.rs:15-18` now imports
`noren_app::session::{…}` instead of the `#[path]` shim. The lane spec grants
the lease on `lib.rs` ("You MAY edit `crates/noren-app/src/lib.rs`") and the
edit is disclosed in the handoff and commit message; it is also *correct* —
with `pub mod session;` live, the shim would compile the module twice and the
integration target would silently test a private duplicate rather than the
wired crate. Reproduction of the hazard: revert the import to the shim, keep
`pub mod session;` — everything still compiles and passes while testing the
wrong copy. Expected: lease wording covering the import-root switch.
Actual: edit justified but outside the lease's literal scope.
Suggested fix: future integration prompts should extend the lease to
"the crate-root module file and the import roots of the merged branches' test
files". No branch change needed.

## Areas verified sound

- Fast-forward merge bookkeeping (merge-base, ancestry, skipped-branch claims
  checked against `git ls-remote`, not the handoff).
- Contract conformance to canonical D-M3-001, field-by-field.
- Selection invariant, observed-status invariant, and bounded-state invariant
  all mutation-resistant and probe-resistant.
- No panic on any degenerate input reachable through the public API; the only
  `expect` (`session.rs:326-329`, id-space exhaustion behind `checked_add`)
  is unreachable in practice and correctly treated as fatal.
- ADR 0003 fully respected: the module is exactly the outside-the-terminal
  session bookkeeping the ADR assigns to Noren.

## Outstanding for the coordinator (not findings against this branch)

- Re-run an integration pass when `m3-session-supervisor`, `m3-sidebar-view`,
  and `m3-adv-fixes` reach `origin` (none is pushed today).
- Ratify the `SessionKind` struct-variant field names into D-M3-001 and decide
  the `SessionRegistry::observe` ratification question the handoff escalated.
