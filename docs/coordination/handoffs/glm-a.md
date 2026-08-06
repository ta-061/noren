# Handoff — M3 session domain lane (GLM, `glm-a`)

> **Note on the handoff template:** `docs/coordination/handoffs/TEMPLATE.md`
> and `docs/coordination/tasks/M3-1a.md` /
> `docs/coordination/decisions/D-M3-001-session-api.md` did **not** exist on
> `origin/main` at the time this lane ran (`1d329a5`). This file is therefore
> structured from the lane prompt's required fields rather than from the
> missing template. A reviewer resuming from here should treat the lane prompt
> (reproduced in spirit below) as the authority and create the missing template
> if one is wanted for later lanes.

## Identity

- **Lane:** `glm-a` (session domain model), engine GLM 5.2 via opencode.
- **Branch:** `agent/m3-session-domain`, branched from `origin/main` @
  `1d329a5` (353 workspace tests passing at branch point).
- **Code commit (authoritative):** `d31e3ac75a9a8f5cf16b6b4ac9d7dcb4e33ff27e`
- **Handoff commit:** the commit that adds this file (separate, so this file can
  record the stable code SHA above).
- **Base SHA:** `1d329a5`.
- **Diff vs main:** `git diff --stat origin/main...HEAD` shows the two leased
  files added only; **no deletions, no edits to `lib.rs`, `Cargo.toml`,
  `Cargo.lock`, or `status.md`.**

## Files touched (within the lease)

| File | Status | Purpose |
| --- | --- | --- |
| `crates/noren-app/src/session.rs` | new | The domain model. |
| `crates/noren-app/tests/session_domain.rs` | new | The invariant test suite. |
| `docs/coordination/handoffs/glm-a.md` | new | This handoff. |

Nothing else was created or edited. The module is **not** wired into
`crates/noren-app/src/lib.rs` (forbidden by the lease; reserved for the serial
integration commit).

## What was implemented

The shared session contract, defined once for every other M3 lane to import:

- `SessionId` — opaque `u64` newtype; private inner field so callers cannot
  fabricate ids, minted only by `SessionRegistry`. `Copy/Clone/Eq/Ord/Hash`,
  plus `Display` (`session-<n>`). No numeric accessor exposed (kept opaque).
- `SessionKind` — `Local` (default, only launchable), `Ssh`, `Agent` (both
  **reserved shapes only**; `is_launchable()` returns false). `#[derive(Default)]`
  via `#[default] Local`.
- `SessionStatus` — `Created` (default/initial), `Running`, `Failed`, `Exited`.
- `SessionDescriptor` — `{ id, kind, status, label: Option<String> }` with
  `id()/kind()/status()/label()` accessors.
- `SessionAction` — command enum: `Create{kind,label}`, `Close{id}`,
  `Select{id}`, `Observe{id,status}`.
- `SessionEvent` — result enum: `Created{id,descriptor}`, `Closed{id}`,
  `StatusChanged{id,status}`, `SelectionChanged{selected: Option<SessionId>}`.
- `SelectedSession` — `{ id, descriptor }` view returned by `selected()`.
- `SessionError` — `UnknownSession` (implements `Display + std::error::Error`).
- `SessionRegistry` — pure state machine. API: `new()`/`Default`, `apply(action)`
  -> `Result<Vec<SessionEvent>, SessionError>`, convenience `create(kind,label)`
  -> `SessionId` (infallible), `close/select/observe` -> `Result<(), _>`, and
  queries `get/sessions/selected/len/is_empty`.

### Key design decisions (so a reviewer can challenge them)

1. **Closing the selected session clears the selection to `None`** (rather than
   reassigning to another live session). The invariant only requires "empty or
   on another existing session — never dangling"; clearing is the simplest
   always-correct policy. Reassignment-to-most-recent is left as a future UX
   decision for the app lane.
2. **No auto-select on create.** Selection is explicit via `Select`. The first
   created session is not selected until a `Select` follows. This keeps the
   invariant trivially provable.
3. **`Close` removes the entry entirely** — no tombstone. This is what makes
   "repeated create/close does not grow state" hold; a closed id becomes
   `UnknownSession` to every later action.
4. **`Observe` only mutates the status field**; it never removes an entry.
   Lifecycle removal is exclusively `Close`'s job, so observation and lifecycle
   stay decoupled.
5. **`create` is infallible** (returns `SessionId`, not `Result`). The counter
   is `u64`; exhaustion is unreachable in practice and would corrupt uniqueness,
   so it `.expect()`s with a clear message. This is the **only** panic point in
   the module and it is reasoned, not accidental.
6. **`SessionEvent` history is not retained.** `apply` returns events; the
   registry stores no `Vec<SessionEvent>`, so state stays bounded to live
   sessions + counter + selection.
7. **No persistence format chosen.** No `serde` derives; the model is in-memory
   only, as instructed.
8. **No Noren/Zellij boundary crossing.** No pane, tab, layout, or split type
   exists anywhere in the model.
9. **Reserved kinds are bookkept but not gated at create.** `Create{Ssh}` /
   `Create{Agent}` succeed (pure data); the future spawn layer gates on
   `SessionKind::is_launchable()`.

## How the unwired module is tested

Because the lease forbids editing `lib.rs`, the module is not part of the
crate's module tree yet. The integration test compiles it **standalone** with:

```rust
#[path = "../src/session.rs"]
mod session;
```

This is the idiomatic way to test an unwired module: `cargo test --workspace`
and `cargo clippy --workspace --all-targets` both see `session.rs` through the
test target's module tree. **When the serial wiring commit adds
`pub mod session;` to `lib.rs`, that `#[path]` line must be replaced by
`use noren_app::session;`** — otherwise the module compiles twice (once in lib,
once in the test) as two unrelated types. This is the single integration step a
resumer must perform; it is called out here because it cannot be done from this
branch.

## Commands actually run (gate), with real results

Run from the worktree root on `agent/m3-session-domain` after the code commit
`d31e3ac`, on macOS arm64, rustc 1.88.0 (pinned by `rust-toolchain.toml`).

1. `cargo fmt --all` → exit 0 (applied formatting).
   `cargo fmt --all --check` → exit 0 (clean).
2. `cargo clippy --workspace --all-targets -- -D warnings` → exit 0, no
   warnings. (One earlier run failed with three compile errors — missing
   `#[default]` on the two derived-`Default` enums, and `const fn new()` calling
   non-const `HashMap::new()`. All three were fixed: `#[default]` markers added
   to `Local`/`Created`, and `new()` made non-`const`. The clean run above is
   post-fix.)
3. `cargo test --workspace` → exit 0.

### Test result totals (`cargo test --workspace`)

**385 passed, 0 failed, 1 ignored, 0 measured.**

Breakdown of the relevant targets:

| Target | Result |
| --- | --- |
| `noren-app` lib unittests (`src/lib.rs`) | 79 passed, 1 ignored (macOS clipboard test) |
| `noren-app` bin unittests (`src/main.rs`) | 24 passed |
| `tests/session_domain.rs` (NEW) | **32 passed** (27 integration + 5 inline `session::tests`) |
| `tests/verify59_independent.rs` | 19 passed |
| `noren-pty` | 10 passed |
| `noren-terminal` | 45 passed |
| `noren-terminal` adversarial / feature suites | 153 passed total |
| doc-tests (all crates) | 0 |

Baseline at branch point was **353** workspace tests; this lane adds **32**
(`session_domain`), bringing the workspace to **385**. The arithmetic
(353 + 32 = 385) reconciles.

## How each required invariant is tested

All four are pinned in `tests/session_domain.rs`:

1. **At most one selected; closing the selected never dangles** —
   `selecting_replaces_the_prior_selection`, `closing_the_selected_session_clears_the_selection`,
   `closing_a_non_selected_session_keeps_the_selection`, `closing_the_only_session_leaves_no_selection`,
   `selecting_an_unknown_session_errors`, `selected_descriptor_matches_get_descriptor`.
2. **Registry spawns no process** — documented and exercised by
   `a_full_session_lifecycle_runs_without_any_child_process` (a full
   create/observe/select/close lifecycle runs in pure memory).
3. **Status observed, not inferred** — `a_newly_created_session_is_created_not_running`,
   `observe_advances_status_to_running`, `observe_records_failure_and_exit_statuses`,
   `observing_the_current_status_is_a_no_op`, `create_then_observe_then_close_keeps_status_observed_only`.
4. **Bounded live state** — `repeated_create_close_cycles_do_not_accumulate`
   (1000 cycles, `len()==0`), `a_recreated_session_gets_a_fresh_distinct_id`,
   `close_is_idempotent_in_state_only_second_close_errors`.

The reducer event contract is pinned by `apply_*` tests; the query surface and
reserved-kind behavior by `sessions_are_listed_in_identifier_order`,
`descriptors_expose_kind_label_and_status`, and
`reserved_kinds_can_be_bookkept_but_are_not_launchable`.

## What could NOT be verified (wiring pending)

- **The module does not compile as part of `noren-app`'s library yet**, because
  `mod session;` is deliberately absent from `lib.rs`. It compiles and is
  lint-tested **only** through the `#[path]` include in the integration test.
  A reviewer cannot confirm `noren_app::session::SessionRegistry` resolves until
  the serial wiring commit lands; that is expected by design.
- **No `#[path]`-free consumer exists**, so I could not exercise the types from
  a second crate or the binary. The `#[path]` test is the sole consumer.
- I could not run any gate against the spec documents
  `docs/coordination/tasks/M3-1a.md` and
  `docs/coordination/decisions/D-M3-001-session-api.md` because **they are not
  present on `origin/main`** (nor anywhere in the tree at `1d329a5`). I
  implemented to the lane prompt, which is the stated authority ("it is the
  authority, not this prompt" referred to a file that does not exist; the prompt
  body was therefore the only available contract). **A reviewer should diff my
  types against the real spec the moment it lands** and reconcile any field or
  variant mismatch — I had no way to do so.
- **ADR 0003** is referenced by the prompt as "owner-decided" but is not present
  in `docs/adr/` (only 0001/0002 + the template exist). I honored the boundary
  as described in the prompt (no pane/tab/layout/split) but could not read the
  ADR itself.

## Assumptions

- The lane prompt is the authority because the named spec files are absent.
- `SessionAction::Create` does not need a command/argv field for this lane: the
  registry owns no process, so launch arguments belong to the future spawn lane.
  I kept `Create` to `{kind, label}` only. If the real D-M3-001 requires argv on
  the descriptor, that is an additive change for the wiring commit.
- "Status is only set from a reported observation" means create records
  `Created` and only `Observe` advances it. I treated `Created` as a recorded
  fact of entry existence, not an inference of liveness.
- Selection-on-close policy (clear vs. reassign) was left to my judgment since
  the prompt allowed both; I chose clear.

## Unresolved findings

- None within the code. The only open items are the missing spec/ADR/TEMPLATE
  documents listed above, which are outside this lane's file lease.

## Authorship / conflict of interest

- **I (GLM `glm-a`) authored all of the code under review** (`session.rs` and
  `session_domain.rs`). I did **not** review a different lane's code here. Per
  fleet policy (lanes scoped so two engines never review the same code), an
  independent lane should perform the review, not this one.
- I also authored this handoff.

## Resume instructions

1. `git checkout agent/m3-session-domain`; confirm code commit
   `d31e3ac` is present.
2. Re-run the gate to reproduce: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` (expect 385 passed / 1 ignored).
3. To wire into the crate (serial integration commit, **not** this branch): add
   `pub mod session;` to `crates/noren-app/src/lib.rs`, then change the first
   non-comment line of `tests/session_domain.rs` from
   `#[path = "../src/session.rs"] mod session;` to `use noren_app::session;`.
4. Reconcile the types against `M3-1a.md` / `D-M3-001-session-api.md` once those
   files exist on `main`.
