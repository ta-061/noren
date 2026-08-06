# Independent review — M3-1a session domain model (`glm-a`) at the StatusChanged fix

Independent review. I did not author this code; I reviewed head from scratch on
the branch rather than trusting the author handoff. This supersedes the two
earlier reviews on this branch (first review `b0f61c3`, re-review `6fc1e39`,
which found the since-fixed `StatusChanged` fork); their substance is re-verified
below against the current head.

- Reviewed at: `0718f56` on `agent/m3-session-domain`, off `origin/main` @
  `1d329a5` (macOS arm64).
- Authority: task spec `state/tasks/M3-1a.md` (fleet repo `noren-fleet-private`).
- Contract source: `state/D-M3-001-session-api.md` (fleet repo). Diffed directly,
  not via the handoff's table.
- Scope of diff (`git diff --name-status origin/main...HEAD`): 4 files, all `A`
  (additions only, +1473 / −0). Note: the lane prompt's `git checkout
  agent/m3-session-domain` was run from a sibling worktree because the branch was
  already checked out at `pool-m3a` (worktree lock) — same commit, clean tree.

## Gate output (actually run at `0718f56`)

```
$ cargo fmt --all -- --check
    exit 0 (no diff)
$ touch src/session.rs tests/session_domain.rs        # force rebuild, not cache
$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (.../pool-m3a/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo]  → exit 0, 0 warnings
$ cargo test --workspace
    PASSED=387 FAILED=0 IGNORED=1
    (session_domain: running 34 tests → ok. 34 passed; 0 failed)
```

Totals reconcile with the handoff's claim (353 baseline + 34 `session_domain` =
387). The 34 include 5 `#[cfg(test)]` unit tests compiled standalone into the
integration target via `#[path]`.

## Acceptance criteria (`state/tasks/M3-1a.md`), one by one

1. **"At most one selection, never dangling after close." — MET.** Selection is a
   single `Option<SessionId>` (`session.rs:276`); `close_events` clears it when
   the closed id was selected (`session.rs:429-438`). Mutation M1 below removed
   the clear → 5 tests FAILED.
2. **"Registry spawns no process; domain tests run without any child." — MET.**
   No `Command`/`spawn`/`std::process` in `session.rs` outside doc comments; the
   full-lifecycle test (`tests/session_domain.rs:129-143`) runs purely in memory.
   The crate's real process machinery (`clipboard.rs`, `main.rs`) is untouched and
   not imported by the leased files.
3. **"Status is only set from a reported observation, never inferred." — MET.**
   `create` records `Starting` (`session.rs:333`); only `observe`
   (`session.rs:364-381`) advances status. Mutation M2 (infer `Running` on
   create) → 3 tests FAILED.
4. **"No pane, tab, or layout type exists anywhere in the module." — MET.**
   `grep -iE 'pane|tab|layout|split|zellij'` over `session.rs` and the test file
   matches only the boundary doc comment (`session.rs:8-9`). The `Tab` hits in
   `lib.rs`/`main.rs` are the pre-existing keyboard `Key::Tab`, not this diff.
   ADR 0003 respected; nothing reads or persists Zellij-internal state.

## Contract conformance (diffed against canonical D-M3-001)

| Type | Head location | Conforms? |
| --- | --- | --- |
| `SessionId(u64)` | `session.rs:54` | yes |
| `SessionKind` (5 variants) | `session.rs:80-104` | yes — contract leaves `{..}` payloads unspecified; `root/path/target/name` are an implementation choice flagged for coordinator confirmation |
| `SessionStatus` incl. `Exited{code:Option<i32>}`, `Failed{reason:String}` | `session.rs:124-141` | yes, exact |
| `SessionDescriptor {id,kind,status,title:String}` | `session.rs:150-156` | yes |
| `SessionAction {Create{kind},Select{id},Close{id}}` | `session.rs:191-207` | yes, exact |
| `SessionEvent` incl. `StatusChanged { id, status }` | `session.rs:216-230` | **yes — previously-forked variant now matches the contract struct shape** |
| `SelectedSession = Option<SessionId>` | `session.rs:237` | yes |
| `SessionRegistry { .. }` | unspecified by contract | yes (methods free) |

The earlier MAJOR (`StatusChanged` unit fork, `6fc1e39`'s finding) is genuinely
resolved: head emits the struct variant from `observe` (`session.rs:377-380`) and
the guard test now constructs the contract shape
(`tests/session_domain.rs:415-418`). Mutation M5 (revert to a unit variant) fails
to compile (E0559) — so the guard is no longer inverted; it bites in the correct
direction. `SessionError` remains an acceptable local addition (D-M3-001 defines
no error type); `observe` as a registry *method* is conforming (the contract's
`SessionRegistry { .. }` fixes no methods) and was escalated in the handoff, not
smuggled into `SessionAction`.

## Required tests — present and effective

- create/select/close incl. closing the selected session:
  `closing_the_selected_session_clears_the_selection` (`:85`),
  `apply_close_of_selected_emits_closed_then_selected_none` (`:317`).
- selection None-or-existing after every operation: covered across
  `:34`, `:58`, `:85`, `:97`, `:110`.
- bounded state under repeated create/close:
  `repeated_create_close_cycles_do_not_accumulate` (`:268`, 1000 cycles).
- invalid actions rejected without panic: `:58`, `:221`, `:290`, `:370`
  (unknown id, double close, observe-after-close all return
  `Err(UnknownSession)`).

## Mutation testing (each reverted; suite green after each revert)

| Mutation | Result |
| --- | --- |
| M1: `close_events` stops clearing the selection | **5 FAILED** (`closing_the_selected_session_clears_the_selection`, etc.) |
| M2: `create` infers `Running` | **3 FAILED** (`a_newly_created_session_is_starting_not_running`, etc.) |
| M3: `close` keeps a tombstone instead of removing | **10 FAILED** (`repeated_create_close_cycles_do_not_accumulate`, etc.) |
| M4: `observe` emits even when status unchanged | **2 FAILED** (`observing_the_current_status_is_a_no_op`) |
| M5: revert `StatusChanged` to unit variant | **compile failure (E0559)** — guard + `observe` pin the contract shape |
| M6: re-selecting the selected session re-emits `Selected` | **1 FAILED** (`selecting_the_already_selected_session_is_a_no_op`) |

The behavioural suite genuinely bites; the conformance guard now bites in the
right direction. Tests are not vacuous.

## Regressions, boundaries, and combinations the author did not test

I compiled a scratch `#[path]` harness (in a temp dir, then deleted; tree left
clean) and ran interactions beyond the author's suite — 9 scratch tests + the 5
module unit tests, `ok. 14 passed; 0 failed`:

- **Cross-registry id aliasing.** Two independent `SessionRegistry::new()` both
  mint `SessionId(1)`; because `SessionId` is a bare opaque u64 with no registry
  affinity, registry A *accepts* registry B's id as its own (`a.select(b_id)` is
  `Ok`). Within one registry the no-aliasing invariant holds (ids are never
  reused live), and the contract explicitly scopes ids to a single run/single
  registry, so this is not a contract violation — but it is a real footgun if the
  app ever recreates a registry mid-run and a stale id survives it. Recorded for
  the coordinator; see Observations.
- **Non-monotonic status.** `observe` accepts `Running→Failed→Running→Exited{None}`
  (pure reporter, no transition gating). Contract is silent and puts lifecycle
  truth in the supervisor; not a defect.
- **Selecting an `Exited`/`Failed` session is allowed**, and closing it
  afterwards still clears selection. Contract silent; consistent with prior
  review.
- **`apply` round-trip over a reserved kind** (`Create{Agent}→Select→observe→
  Close`) emits exactly `Created / Selected(Some) / StatusChanged / Closed +
  Selected(None)` in order.
- **Degenerate payloads:** empty `Ssh.target`/`Agent.name`, empty `Project.root`,
  NUL/newline/emoji/CJK in `Worktree.path`, and a 1,000,000-char `Failed.reason`
  all round-trip without panic.
- **`Exited{code:None}` vs `Exited{code:Some(0)}`** are distinct for the no-op
  check; re-observing an identical payload is a no-op.
- **Snapshot isolation:** a `get()` descriptor is a clone — later `observe` does
  not rewrite it.
- **Listing stays id-sorted after closing middle ids.**
- **200k create/close churn:** `len()` stays 0, ids strictly increase and never
  alias a closed session; `sessions()`/`select`/`close` still work after.

## Panics, leaks, unbounded growth

- One deliberate panic point, reasoned: u64 id-space exhaustion via `checked_add`
  (`session.rs:326-329`). No other `unwrap`/`expect`/`panic!` in non-test code
  (`serde`, `unreachable!`, `todo!` absent).
- No resource handles; the registry owns only `HashMap + Option<SessionId> + u64`.
- `close` removes the entry; no event history is retained; `next_id` is the sole
  monotonic counter. Live state is bounded to live sessions + counter + selection;
  nothing grows with call count. `HashMap` capacity is bounded by peak concurrent
  sessions (not churn count), which is consistent with "no session count cap is
  implied."

## Unintended deletions, lease, and forbidden files

`git diff --name-status origin/main...HEAD` → four `A` entries
(`src/session.rs`, `tests/session_domain.rs`, the handoff, the prior review);
**zero deletions**. Combined diff of the forbidden files (`lib.rs`, `main.rs`,
`Cargo.toml`, `Cargo.lock`, `status.md`) is 0 lines, and `lib.rs` contains no
`session` — the module is unwired per the lease, reachable only through the
`#[path]` test. The handoff and review files are lane/reviewer coordination
artifacts, not a lease violation (the lease forbids `status.md` specifically, not
handoffs/reviews). The handoff's "when wiring, replace `#[path]` with
`use noren_app::session;`" note remains correct and necessary.

## Sound areas

The behavioural core is solid and mutation-verified: single-selection invariant,
no-process purity, observed-only status, bounded live state, typed rejection of
unknown/double-close, and now exact `StatusChanged` conformance pinned by a
correctly-oriented guard. Escalating `observe` (rather than forking
`SessionAction`) and keeping `SessionError` local were the right calls under the
stop conditions.

## Observations for the coordinator (not ranked defects)

1. **`SessionKind` payload field names** (`root`/`path`/`target`/`name`) are
   inferred; the canonical contract elides them as `{..}`. Confirm before any
   downstream lane codes against them (highest-risk unverifiable item, per the
   handoff).
2. **`observe` ratification.** Decide whether D-M3-001 should ratify an
   observation action or keep it a registry method (handoff escalation item).
3. **Cross-registry id opacity** (see Combinations): ids carry no registry
   affinity; fine for the single-registry design but worth a sentence in D-M3-001
   if registry recreation or multiple registries ever become possible.

## Verdict

PASS — all four acceptance criteria met; all six previously-deviating contract
types (including the `StatusChanged` struct variant that forked twice) now match
D-M3-001, verified by direct diff and by a compile-failing mutation; the test
suite bites under six independent mutations and survived nine adversarial
interaction/growth scenarios beyond the author's suite. No BLOCKER, MAJOR, or
MINOR defect found; three coordination observations recorded.

`REVIEW_M3-1a verdict=PASS blockers=0 majors=0 minors=0 tests=PASS total=387`
