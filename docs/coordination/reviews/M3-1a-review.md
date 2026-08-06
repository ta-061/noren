# Review — M3-1a session domain model (`glm-a`)

Independent review. I did not author this code and did not take the handoff at
its word: every claim below is backed by a command I ran on this branch.

- Reviewed at: `c2d5963` on `agent/m3-session-domain` (code commit `d31e3ac`),
  off `origin/main` @ `1d329a5`.
- Authority: `state/tasks/M3-1a.md` (fleet repo); contract source
  `state/D-M3-001-session-api.md` (fleet repo — see the MAJOR finding).
- Scope of diff (`git diff --stat origin/main...HEAD`): 3 files, **+1164 / -0**.
  Only additions; nothing removed, nothing outside the lease edited.

## Gate output (actually run)

```
$ cargo fmt --all -- --check        → exit 0 (clean, no diff)
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s)  (exit 0, 0 warnings)
$ cargo test --workspace            → exit 0
    PASSED=385 FAILED=0 IGNORED=1
    (lib 79+1ignored, bin 24, session_domain 32, verify59 19, pty 10,
     terminal 45 + adversarial/feature suites = 385 total)
```

Totals reconcile with the handoff's claimed 385 passed / 1 ignored.

## Acceptance criteria (from `state/tasks/M3-1a.md`), one by one

1. **"Registry holds sessions with at most one selection, never dangling after
   close." — MET.** Selection is `Option<SessionId>` (`session.rs:257`);
   `close_entry` clears it when the closed id was selected
   (`session.rs:395-405`). Tests `closing_the_selected_session_clears_the_selection`,
   `closing_a_non_selected_session_keeps_the_selection`,
   `closing_the_only_session_leaves_no_selection` cover it. I additionally broke
   this (Mutation 1 below) and a test failed, so the coverage is real.
2. **"Registry spawns no process; domain tests run without any child." — MET.**
   No `Command`/`spawn`/process API anywhere in `session.rs`; it is pure
   `HashMap`/`Option`/`u64` state. The full lifecycle test runs in memory.
3. **"Status is only set from a reported observation, never inferred." — MET.**
   New entries start `Created` (`session.rs:388`); only `observe_entry`
   (`session.rs:418-432`) advances status. Mutation 2 (infer `Running` on
   create) failed 4 tests.
4. **"No pane, tab, or layout type exists anywhere in the module." — MET.**
   `grep -inE 'pane|\btab\b|layout|split|zellij'` matches only the module doc
   comment (`session.rs:9-10`) that states the boundary; no such types exist.
   ADR 0003 boundary respected.

## Required tests — present, and they actually bite

All four required behaviours have tests, and I confirmed they are not vacuous by
mutating the implementation and watching tests fail (then reverted):

| Mutation (reverted after) | Result |
| --- | --- |
| `close_entry` stops clearing selection (dangling id) | `apply_close_of_selected_emits_closed_then_selection_cleared` **FAILED** |
| `create_entry` infers `Running` instead of `Created` | 4 tests **FAILED** incl. `a_newly_created_session_is_created_not_running` |
| `close_entry` retains the entry (tombstone leak) | 10 tests **FAILED** incl. `repeated_create_close_cycles_do_not_accumulate` |

After reverting, `session_domain` returns to `32 passed; 0 failed`. The suite
genuinely encodes the invariants.

## Interactions the author did not test

I wrote a scratch `#[path]` test target (deleted after running; tree left clean)
covering combinations not in the suite. All passed:

- select → observe → read `selected()`: the descriptor is a **fresh** snapshot
  (`Running` reflected), not stale.
- interleaved select/observe/close across 3 sessions, reselect, close former
  selection: selection and statuses stay consistent; `len()==1` at the end.
- hostile labels: empty string, 1_000_000-char label, emoji/CJK/NUL — no panic,
  values round-trip intact.
- full kind×status transition matrix (`Local`/`Ssh`/`Agent` × every status) via
  `apply`; no panic, `close` leaves `is_empty()`.
- stale id after close is rejected by `select`/`observe`/`close`/`get`.

```
test result: ok. 10 passed; 0 failed; 0 ignored  (scratch, then removed)
```

## Panics, leaks, unbounded growth

- Exactly one panic point, deliberate and reasoned: id-space exhaustion via
  `.expect("session id space exhausted")` with `checked_add`
  (`session.rs:381-384`). No other `unwrap`/`expect`/`panic!` in non-test code
  (`grep` confirmed).
- `close` **removes** the entry (`session.rs:396`), so repeated create/close
  cannot accumulate; verified by the 1000-cycle test and by Mutation 3. No
  event/history buffer is retained. `next_id` is the only monotonic counter and
  is bounded by `u64`.
- `sessions()` and `selected()` allocate fresh views per call; nothing grows
  with call count. No resource handles are held. No leak.

## Unintended deletions / lease

`git diff --name-status origin/main...HEAD` → three `A` (add) entries only:
`src/session.rs`, `tests/session_domain.rs`, `docs/coordination/handoffs/glm-a.md`.
Zero deletions. Forbidden files untouched: the combined diff for
`lib.rs`, `Cargo.toml`, `Cargo.lock`, `status.md` is empty. `lib.rs` is **not**
wired (as required); the `#[path]` test mechanism is the sole consumer.

## Findings

### MAJOR — Public types deviate from the D-M3-001 contract

The task spec's **Public API contract** section says: *"Owns and defines every
type in D-M3-001. Others import… A lane needing a contract change escalates
instead of forking it."* The committed types fork the contract in 6 of 8 places:

| Type | D-M3-001 (fleet `state/D-M3-001-session-api.md`) | Committed (`crates/noren-app/src/session.rs`) | Deviation |
| --- | --- | --- | --- |
| `SessionKind` | `Local, Project{..}, Worktree{..}, Ssh{..}, Agent{..}` | `Local, Ssh, Agent` (`session.rs:56-65`) | **missing `Project`, `Worktree`**; struct-variants rendered unit |
| `SessionStatus` | `Starting, Running, Exited{code:Option<i32>}, Failed{reason:String}` | `Created, Running, Failed, Exited` (`session.rs:84-95`) | `Starting`→`Created`; **dropped `Exited.code` and `Failed.reason` payloads** |
| `SessionDescriptor` | `{id,kind,status,title:String}` | `{id,kind,status,label:Option<String>}` (`session.rs:102-108`) | `title`→`label`, `String`→`Option<String>` |
| `SessionAction` | `Create{kind}, Select{id}, Close{id}` | adds `Observe{id,status}`, adds `label` to `Create` (`session.rs:141-167`) | additive |
| `SessionEvent` | `Created(SessionId), Selected(Option<SessionId>), StatusChanged, Closed(SessionId)` | `Created{id,descriptor}, …, SelectionChanged{selected}` (`session.rs:173-199`) | `Selected`→`SelectionChanged`; `Created` payload widened |
| `SelectedSession` | `pub type SelectedSession = Option<SessionId>;` | `struct SelectedSession{id,descriptor}` (`session.rs:206-210`) | type alias → struct |

`SessionId` and `SessionRegistry` conform; `SessionError` is a reasonable
addition (D-M3-001 defines no error type).

**Why it matters (expected vs actual):** the other four M3 lanes (M3-1b
supervisor, M3-3, M3-4 dispatch, M3-ADV) are told to *import* these types. A
lane constructing `SessionKind::Project`, matching `SessionEvent::Selected`, or
reading `descriptor.title` / `Exited { code }` **will not compile** against this
branch. Expected: lanes share one contract shape; actual: this branch is the
contract owner yet diverges from it — exactly the fork the contract exists to
prevent.

**Root cause / mitigation (recorded for fairness, not as absolution):**
`git ls-tree -r origin/main` confirms D-M3-001 is **absent from the noren repo**
(no `docs/coordination/decisions/`). The author's lane pointed at a path that did
not exist, so the fork was made blind and was explicitly flagged in the handoff
("A reviewer should diff my types against the real spec the moment it lands").
The behavioural core is sound and reusable; the fix is shape alignment, not a
rewrite.

**Minimal suggested fix:** before the serial wiring commit lands, align the six
types to D-M3-001 (add `Project`/`Worktree`, restore `Starting` and the
`Exited{code}`/`Failed{reason}` payloads, rename to `title: String`, restore
`Selected(Option<SessionId>)` and `pub type SelectedSession = Option<SessionId>`),
and decide with the coordinator whether the added `Observe` action/`Created.label`
should be ratified *into* D-M3-001 (escalate, per the stop conditions) rather than
silently kept. Independently, the coordinator should sync D-M3-001 into the repo
so downstream lanes are not coding blind.

### No BLOCKER

ADR 0003 is respected (no pane/tab/layout/split type), so the prompt's explicit
blocker condition is not triggered. The MAJOR above is a contract-shape
mismatch, recoverable by alignment; it is not an architectural boundary break.

## Verdict

Behavioural acceptance criteria are met and mutation-tested; no panics, leaks,
unbounded growth, unintended deletions, or boundary violations. The single issue
is that the owned public types do not match the D-M3-001 contract that four
downstream lanes import. That must be reconciled before integration.

`REVIEW_M3-1a verdict=FINDINGS blockers=0 majors=1 minors=0 tests=PASS total=385`
