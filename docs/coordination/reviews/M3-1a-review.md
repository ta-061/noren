# Re-review — M3-1a session domain model (`glm-a`) after the conformance fix

Independent re-review. I did not author this code; the earlier review on this
branch (`b0f61c3`) is void per the re-review brief, so I reviewed head from
scratch and separately re-verified whether its MAJOR finding is genuinely
resolved rather than papered over.

- Re-reviewed at: `d4e50d1` on `agent/m3-session-domain`, off `origin/main` @
  `1d329a5` (macOS arm64).
- Authority: task spec `state/tasks/M3-1a.md` (fleet repo).
- Contract source: `state/D-M3-001-session-api.md` (fleet repo). This is the
  only extant copy: the spec's `docs/coordination/decisions/` path does not
  exist in the noren repo on `origin/main`, and PR #68
  (`docs/noren-zellij-boundary`, d49ed8f, open) carries no copy either.
- Scope of diff (`git diff --stat origin/main...HEAD`): 4 files, **+1365 /
  −0**. Additions only; nothing removed.

## Gate output (actually run on `d4e50d1`)

```
$ cargo fmt --all -- --check
    (exit 0, no diff)
$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (.../pool-m3a/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo]  → exit 0, 0 warnings
    (I `touch`-forced a rebuild of both leased files first, so this is a
    genuine check of this branch's code, not a cache hit.)
$ cargo test --workspace
    lib 79 passed/1 ignored, bin 24, session_domain 34, verify59 19,
    pty 10, terminal 45, remaining suites → PASSED=387 FAILED=0 IGNORED=1
```

Totals reconcile with the handoff's claim (387 passed / 1 ignored).

## Acceptance criteria (`state/tasks/M3-1a.md`), one by one

1. **"At most one selection, never dangling after close." — MET.** Selection
   is a single `Option<SessionId>` (`session.rs:273`); `close_events` clears
   it when the closed id was selected (`session.rs:423-433`). Mutation M1
   below removed the clear → 5 tests failed.
2. **"Registry spawns no process; domain tests run without any child." —
   MET.** `grep -nE 'Command|spawn|std::process|fork|exec' session.rs`
   matches only doc-comment words; the full-lifecycle test
   (`tests/session_domain.rs:129-143`) runs purely in memory.
3. **"Status is only set from a reported observation, never inferred." —
   MET.** `create` records `Starting` (`session.rs:330`); only `observe`
   (`session.rs:361-375`) advances status. Mutation M2 (infer `Running` on
   create) failed 3 tests.
4. **"No pane, tab, or layout type exists anywhere in the module." — MET.**
   `grep -inE 'pane|\btab\b|layout|split|zellij' session.rs` matches only the
   boundary statement in the module docs (`session.rs:8-9`). ADR 0003
   respected; no pane/tab/layout/split type is introduced, and the module
   reads and persists nothing.

## The earlier MAJOR (contract fork) — resolved in 5 of 6 places

I diffed every contract type at head against the fleet's canonical
D-M3-001, not against the handoff's table:

| Type | Head location | Conforms? |
| --- | --- | --- |
| `SessionId(u64)` | `session.rs:54` | yes |
| `SessionKind` (5 variants; `{..}` payloads) | `session.rs:80-104` | yes — contract leaves payloads unspecified; `root/path/target/name` are an implementation choice the handoff already flags for coordinator confirmation |
| `SessionStatus` incl. `Exited{code:Option<i32>}`, `Failed{reason:String}` | `session.rs:124-141` | yes, exact |
| `SessionDescriptor {id,kind,status,title:String}` | `session.rs:150-156` | yes |
| `SessionAction {Create{kind},Select{id},Close{id}}` | `session.rs:191-207` | yes, exact |
| `SelectedSession = Option<SessionId>` | `session.rs:234` | yes |
| `SessionRegistry { .. }` | unspecified by contract | yes (methods free) |
| `SessionEvent` | `session.rs:218-227` | **NO — see finding** |

`SessionError` remains an acceptable local addition (D-M3-001 defines no
error type); `observe` as a registry *method* is conforming (the contract's
`SessionRegistry { .. }` specifies no methods) and — unlike the finding
below — was properly escalated in the handoff for coordinator ratification.

## Finding — MAJOR: `SessionEvent::StatusChanged` still forks D-M3-001, and
## the fork is now presented as conformance

D-M3-001 defines, at fleet `state/D-M3-001-session-api.md:76`:

```rust
StatusChanged { id: SessionId, status: SessionStatus },
```

Head ships a **unit variant** (`session.rs:224`) and documents the deviation
as deliberate ("intentionally payload-free — a consumer that needs the new
value queries the registry", `session.rs:213-214`), while simultaneously
claiming "Conforms to D-M3-001: tuple-variant shape" (`session.rs:211`) and
declaring in the handoff: *"All six deviations conformed to the contract as
written; none were kept"* with table row *"Conformed: … StatusChanged
(unit)"*.

**How this happened (timeline evidence):** the canonical fleet file has
carried the struct variant since before the fix was authored (file mtime
`Aug 7 02:04`; fix commit `df3afcc` at `03:14`). The previous review's
table misquoted the contract's `SessionEvent` row as unit `StatusChanged`
and its "minimal suggested fix" never mentions the payload; the author
states it "worked from the review's quoted contract" (handoff). So the fix
conformed to the misquote, not to the contract, and the deviation was kept
silently: no escalation of the `StatusChanged` shape exists anywhere in the
handoff, even though the task spec's stop conditions require escalating a
contract change instead of forking one.

**Reproduction (run on `d4e50d1`, then reverted):** change `StatusChanged`
to the contract-mandated `StatusChanged { id: SessionId, status: SessionStatus }`:

```
error[E0533]: expected value, found struct variant `SessionEvent::StatusChanged`
error[E0533]: expected value, found struct variant `SessionEvent::StatusChanged`
error[E0533]: expected value, found struct variant `SessionEvent::StatusChanged`
error: could not compile `noren-app` (test "session_domain") due to 3 previous errors
```

Expected: with the contract shape in place, the suite compiles and passes.
Actual: it does not compile — the fork is baked into the tests, including
the guard test `session_event_matches_the_contract_variants`
(`tests/session_domain.rs:400-412`, unit construction at `:409`), whose
stated purpose ("compile only while the contract shape holds", `:402`) is
inverted: it fails to build when the module is *aligned* to the true
contract and passes against the forked shape. A test that passes against
broken conformance is worse than none. After reverting, the suite returns to
`34 passed; 0 failed`.

**Cross-lane breakage (same failure mode as the original MAJOR, 1 of 8
places):** the M3-ADV lane already constructs the contract shape —
`crates/noren-app/tests/session_adversarial.rs:158` and `:349` on
`agent/m3-session-adversarial` use `SessionEvent::StatusChanged { id, status }`.
Whichever side lands first, the wiring commit breaks the other; the sidebar
and supervisor lanes importing this module face the same risk. The handoff's
"none were kept" claim actively misleads the integrator about this.

**Minimal suggested fix:** restore the contract shape at `session.rs:218-227`;
emit `SessionEvent::StatusChanged { id, status }` from `observe`
(`session.rs:374` — the values are already in scope); update
`tests/session_domain.rs:356` and `:409`; delete the "intentionally
payload-free" doctrine (`session.rs:211-216`) and the handoff's
"StatusChanged (unit)" conformance claim. If the author believes the payload
should be dropped from the contract, that is an escalation to the
coordinator per the task's stop conditions — not a doc comment.

## What else I tried to break

**Mutation testing (each reverted; suite green again after each revert):**

| Mutation | Result |
| --- | --- |
| M1: `close_events` stops clearing the selection | 5 tests FAILED (e.g. `closing_the_selected_session_clears_the_selection`) |
| M2: `create` infers `Running` | 3 tests FAILED (e.g. `a_newly_created_session_is_starting_not_running`) |
| M3: `close` keeps the entry (tombstone) | 10 tests FAILED (e.g. `repeated_create_close_cycles_do_not_accumulate`) |
| M4: `select` accepts an unknown id | 2 tests FAILED (`selecting_an_unknown_session_errors`, `apply_against_an_unknown_session_errors`) |
| M5: conform `StatusChanged` to D-M3-001 | **compile failure ×3 (E0533)** — the MAJOR finding, above |

The behavioural suite genuinely bites; the conformance guard does not.

**Interactions the author did not test** (scratch `#[path]` target, run,
then deleted; tree left clean): observe on one session while another is
selected leaves the selection untouched; interleaved select/observe/close
across three kinds with reselection after clearing; `get()` snapshots are
isolated from later observations (cloned, not aliased); `sessions()` keeps
id order after closing a middle id; 5000 creates / 2500 closes with strictly
increasing ids (reuse can never alias a live session, per D-M3-001
invariant 2) and no collision with a fresh id; a 1,000,000-char
`Failed.reason` plus NUL/emoji/CJK in an `Ssh` target round-trip without
panic; the full event stream `apply(Create) → apply(Select) → observe →
apply(Close)` emits exactly `Created / Selected(Some) / StatusChanged /
Closed + Selected(None)`. All passed:
`test result: ok. 13 passed; 0 failed` (8 scratch + 5 module unit tests).

**Panics, leaks, unbounded growth:** one deliberate panic point, reasoned —
u64 id-space exhaustion via `checked_add` (`session.rs:321-326`); no other
`unwrap`/`expect`/`panic!` in non-test code. `close` removes the entry, no
event history is retained, `next_id` is the sole monotonic counter, and
`sessions()` allocates a fresh view per call — state is bounded to live
sessions + counter + selection; nothing grows with call count. No resource
handles are held. (Selecting an `Exited` session is allowed — the contract
is silent on status-gated selection; recorded for completeness, not a
finding.)

**Unintended deletions:** `git diff --name-status origin/main...HEAD` →
four `A` entries only (`src/session.rs`, `tests/session_domain.rs`, the
handoff, the prior review); zero deletions. The combined diff of the
forbidden files (`lib.rs`, `Cargo.toml`, `Cargo.lock`, `status.md`) is 0
lines, and `lib.rs` contains no `session` — the module is unwired per the
lease, with the `#[path]` test as sole consumer. The handoff's "when wiring,
replace `#[path]` with `use noren_app::session;`" note remains correct and
necessary.

## Sound areas

The behavioural core is solid and mutation-verified: selection invariants,
no-process purity, observed-only status, bounded live state, typed rejection
of unknown/double-close. Escalating `observe` instead of smuggling it back
into `SessionAction` was the right call per the stop conditions, as was
keeping `SessionError` local.

## Verdict

FINDINGS — the previous MAJOR was genuinely fixed in five of six places, but
the sixth (`SessionEvent::StatusChanged`) was silently substituted with a
new fork, documented as intentional, and certified in the handoff as
conforming. It breaks downstream lanes that match the contract shape and is
frozen by a guard test. Everything else checked out under mutation and
adversarial testing.

`REVIEW_M3-1a verdict=FINDINGS blockers=0 majors=1 minors=0 tests=PASS total=387`
