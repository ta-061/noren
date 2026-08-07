# Handoff — M3 session domain lane (GLM, `glm-a`)

> **Note on the handoff template:** `docs/coordination/handoffs/TEMPLATE.md`
> did **not** exist on `origin/main` at the time this lane ran. This file is
> structured from the lane prompt's required fields rather than from a template.

## Identity

- **Lane:** `glm-a` (session domain model), engine GLM 5.2 via opencode.
- **Branch:** `agent/m3-session-domain`, branched from `origin/main` @
  `1d329a5` (353 workspace tests passing at branch point).
- **Initial code commit:** `d31e3ac75a9a8f5cf16b6b4ac9d7dcb4e33ff27e`
- **First conformance fix:** `df3afcc65b3487a3cfe6627520acb0b3fd3544e7`
  (conformed 5 of 6 deviations; silently forked the 6th — see revision
  history).
- **StatusChanged conformance fix (current authoritative code):**
  `65ebc453499130ba69db4384fc834e956f145a08`
- **Independent reviews:** `docs/coordination/reviews/M3-1a-review.md` —
  first review `b0f61c3` (one MAJOR, contract fork in 6/8 places, void after
  the first fix); re-review `6fc1e39` (found 5/6 resolved, one fork remained).
- **Base SHA:** `1d329a5`.
- **Diff vs main:** only the two leased files plus this handoff and the
  reviews; **no deletions, no edits to `lib.rs`, `Cargo.toml`, `Cargo.lock`,
  or `status.md`** (verified: their combined diff vs main is empty).

## Post-review correction on the merge candidate

The recovery review of PR #74 made three follow-up corrections after the lane
history recorded below:

1. The canonical D-M3-001 product contract is now published as
   [session API](../session-api.md), including the concrete kind payloads,
   registry-local ID scope, and the supervisor observation seam. Review no
   longer depends on an inaccessible operations repository.
2. The later adversarial finding ADV-S1 was real: `observe` allowed
   `Exited -> Running`. The merge candidate enforces monotonic lifecycle ranks
   and returns `InvalidStatusTransition` without mutation on regression or
   resurrection.
3. The two enum-shape guards now use exhaustive matches. Constructing known
   variants did not prove that no extra variant existed; the earlier wording
   claiming otherwise was incorrect.

The historical commits, test counts, and review narrative below describe the
earlier lane checkpoints. The current-head gate and independent PR review are
the merge authority.

Local recovery verification on the merge candidate:

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --workspace --all-targets -- -D warnings`: pass, zero warnings.
- `cargo test --workspace`: **388 passed, 0 failed, 1 pre-existing ignored**;
  `session_domain` contributes 35 passing tests.
- `python3 scripts/check_docs.py` and its 7 unit tests: pass.
- Mutation check: disabling the lifecycle-rank guard makes
  `observe_rejects_regression_and_terminal_resurrection` fail on
  `Running -> Starting`; restoring it makes the test pass.

## Revision history

1. `d31e3ac` — initial model. Implemented to the lane prompt because the named
   spec files (`M3-1a.md`, `D-M3-001-session-api.md`, ADR 0003) were **absent**
   from `origin/main`. The handoff explicitly flagged this and asked a reviewer
   to diff against the real contract when it landed.
2. `b0f61c3` — first independent Qwen review. Found one MAJOR: the public types
   fork the D-M3-001 contract in 6 of 8 places (the contract is now published
   as [session API](../session-api.md); the review quotes it side-by-side).
   The coordinator judged the finding real. (This review is void after the fix
   below; its `SessionEvent` row misquoted the contract.)
3. `df3afcc` — first conformance fix. Conformed **5 of 6** deviations
   (`SessionKind`, `SessionStatus`, `SessionDescriptor`, `SessionAction`,
   `SelectedSession`). The 6th, `SessionEvent::StatusChanged`, was **not**
   conformed: it was kept as a unit variant. The fix worked from the first
   review's quoted contract, which misquoted `StatusChanged` as unit, instead
   of the canonical D-M3-001 struct variant. It then documented the unit shape
   as "intentionally payload-free" and the handoff certified **all six** as
   conformant. That certification was wrong, and a handoff that claims
   conformance while the type differs is worse than an openly-declared
   deviation. Corrected below.
4. `6fc1e39` — independent re-review. Re-diffed every contract type against the
   canonical D-M3-001 (not the handoff's table). Confirmed 5/6 resolved; found
   the `StatusChanged` unit fork remaining, frozen by an inverted guard test,
   and shown to break the M3-ADV lane which already constructs the contract
   struct shape.
5. **`65ebc45` — `StatusChanged` conformance fix (current).** Conformed the
   last deviation to the contract exactly. With this, all six deviations are
   genuinely conformed; the table below now states what the code actually does.

## Contract conformance — current state (all 6 conformed)

For each deviation the branch now **conforms** to the contract as written; the
contract was written to be imported by four other lanes, not to be optimal.
`StatusChanged` took two passes (see revision history) and is the one to
re-check on review.

| Type | Deviation (original) | Resolution (current) |
| --- | --- | --- |
| `SessionKind` | missing `Project`/`Worktree`; `Ssh`/`Agent` were unit | `Local`, `Project{root:PathBuf}`, `Worktree{path:PathBuf}`, `Ssh{target:String}`, `Agent{name:String}` |
| `SessionStatus` | `Created` vs `Starting`; dropped `Exited.code`/`Failed.reason` | `Starting`, `Running`, `Exited{code:Option<i32>}`, `Failed{reason:String}` |
| `SessionDescriptor` | `label:Option<String>` vs `title:String` | `title:String`, registry-generated at create (the contract `Create` carries no title) |
| `SessionAction` | added `Observe` and `Create.label` | `{Create{kind}, Select{id}, Close{id}}` |
| `SessionEvent` | `Created{id,descriptor}`, `SelectionChanged`, payload mismatch | `Created(SessionId)`, `Selected(Option<SessionId>)`, **`StatusChanged{id:SessionId,status:SessionStatus}`** (struct variant), `Closed(SessionId)` |
| `SelectedSession` | `struct{id,descriptor}` | `pub type SelectedSession = Option<SessionId>;` |

`SessionId` and `SessionRegistry` already conformed (per the reviews);
`SessionError` is a local addition both reviews accepted (D-M3-001 defines no
error type).

### What this required beyond renaming

- **Status payloads made `SessionStatus` and `SessionKind` non-`Copy`.**
  `Descriptor::kind()`/`status()` now return references; `create(kind)` takes
  `kind` by value. `is_launchable` is now `&self`.
- **`title` generation.** Since `Create{kind}` carries no title, the registry
  generates one: the session's stable display id (e.g. `"session-1"`). This is a
  policy choice on top of the contract, not a contract claim; a future lane may
  override it once a rename/observation path exists.
- **`observe` moved off the action enum.** It is now a registry **method**
  `observe(id, status) -> Result<Option<SessionEvent>, SessionError>` returning
  `Some(StatusChanged { id, status })` on change (the new status is cloned,
  since `SessionStatus` is non-Copy via `Failed{reason}`). This preserves
  invariant 3 (status is only set from a reported observation) without
  re-forking `SessionAction`.

### Observation seam decision (resolved)

`SessionRegistry::observe` is the only way a session advances past `Starting`,
and remains a registry method rather than one of the three `SessionAction`
variants. The public [session API](../session-api.md) now records that decision:
actions are user/UI requests, while an observed process status is a supervisor
fact. This preserves invariant 3 without expanding the exhaustive dispatch
action set.

## What was implemented (current, conformed shape)

- `SessionId` — opaque `u64` newtype; private field (not fabricable); `Copy`,
  `Ord`, `Hash`, `Display` (`session-<n>`).
- `SessionKind` — `Local` (default, only launchable), `Project{root}`,
  `Worktree{path}`, `Ssh{target}`, `Agent{name}`. `is_launchable(&self)` is
  `Local`-only.
- `SessionStatus` — `Starting` (default), `Running`, `Exited{code}`,
  `Failed{reason}`.
- `SessionDescriptor` — `{ id, kind, status, title }`; accessors
  `id()->SessionId`, `kind()->&SessionKind`, `status()->&SessionStatus`,
  `title()->&str`.
- `SessionAction` — `Create{kind}`, `Select{id}`, `Close{id}`.
- `SessionEvent` — `Created(SessionId)`, `Selected(Option<SessionId>)`,
  `StatusChanged { id: SessionId, status: SessionStatus }` (struct variant),
  `Closed(SessionId)`.
- `SelectedSession` — `pub type SelectedSession = Option<SessionId>;`.
- `SessionError` — `UnknownSession` and `InvalidStatusTransition`
  (`Display + std::error::Error`).
- `SessionRegistry` — `new()`/`Default`; `apply(SessionAction) ->
  Result<Vec<SessionEvent>, SessionError>`; `create(kind)->SessionId`
  (infallible); `close/select -> Result<(),_>`; **`observe(id,status) ->
  Result<Option<SessionEvent>,_>`** (the observation seam); queries
  `get/sessions/selected/len/is_empty`.

### Standing design decisions (so a reviewer can challenge them)

1. Closing the selected session **clears** selection to `None` (never dangles;
   the invariant allows empty-or-another; clear is simplest).
2. **No auto-select on create**; selection is explicit.
3. `Close` **removes** the entry (no tombstone) — this is what bounds live state.
4. `observe` mutates only the status field; it never removes an entry.
5. `create` is **infallible**; the lone panic point is `checked_add` id-space
   exhaustion (reasoned; a u64 counter cannot realistically exhaust).
6. No event history is retained; `apply`/`observe` return events, state stays
   bounded to live sessions + counter + selection.
7. No persistence format; no `serde`; in-memory only.
8. No pane/tab/layout/split type (ADR 0003 respected).
9. Two **compile-shape guard tests** (`session_action_has_exactly_the_three_contract_variants`,
   `session_event_matches_the_contract_variants`) exhaustively match the
   contract enums, so adding an unreviewed variant fails to compile. Earlier
   versions merely constructed the known variants and did not enforce this;
   the merge-candidate review corrected them. The public contract remains the
   authority.

## How the unwired module is tested

The lease forbids editing `lib.rs`, so the module is compiled **standalone** in
the integration test:

```rust
#[path = "../src/session.rs"]
mod session;
```

`cargo test --workspace` and `cargo clippy --workspace --all-targets` both see
`session.rs` through the test target. **When the serial wiring commit adds
`pub mod session;` to `lib.rs`, that `#[path]` line must become
`use noren_app::session;`** or the module compiles twice as two unrelated types.

## Commands actually run (gate), with real results

On `agent/m3-session-domain` at `StatusChanged` fix commit `65ebc45`, macOS
arm64, rustc 1.88.0.

```
$ cargo fmt --all && cargo fmt --all --check   → exit 0 (clean)
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile; exit 0, 0 warnings
$ cargo test --workspace                       → exit 0
    PASSED=387 FAILED=0 IGNORED=1
```

This fix run was clean on the first clippy/test pass: the change is small
(restore the struct variant, emit it from `observe`, repoint two test sites).
The earlier `df3afcc` run had hit two borrow-check compile errors (a borrow of
a temporary `Descriptor`, and a mutable-then-immutable borrow of `registry`),
both fixed by binding intermediates.

### Test totals (`cargo test --workspace`)

**387 passed, 0 failed, 1 ignored.** Baseline 353 → `session_domain` now **34**
(was 32; +3 contract-shape/type-alias guards, −2 removed label/struct-view
tests). Reconciles: 353 + 34 = 387.

## Historical limits and current disposition

- **The module is not compiled as part of `noren-app`'s library yet** (`mod
  session;` absent from `lib.rs` by lease). It compiles only via the `#[path]`
  test. `noren_app::session::*` cannot resolve until the serial wiring commit.
- The previously inferred `SessionKind` payloads are now recorded explicitly in
  the public [session API](../session-api.md): `root`/`path` use `PathBuf`, and
  `target`/`name` use `String`.
- The observation seam is resolved as a registry method for supervisor facts,
  not an additional user-facing action variant.
- The generated display-id title remains the initial policy and is documented
  in the public contract; a future rename API may replace it explicitly.
- ADR 0003 and D-M3-001 are now both available in the public repository. The
  earlier inaccessible-contract limitation no longer applies.

## Authorship / conflict of interest

- **I (GLM `glm-a`) authored all the code** (`session.rs`, `session_domain.rs`)
  and this handoff, across the initial commit (`d31e3ac`), the first
  conformance fix (`df3afcc`), and the `StatusChanged` conformance fix
  (`65ebc45`). I did **not** author either review (`b0f61c3`, `6fc1e39` are an
  independent Qwen lane). Per the [development model](../development-model.md),
  an independent reviewer must cover the current head. The recovery review
  supplied the public contract, monotonic transition fix, and exhaustive enum
  guards; its GitHub review and current-head gate are separate from the GLM
  implementation record.

## Resume instructions

1. `git checkout agent/m3-session-domain`; confirm the current PR head.
2. Re-run the gate: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` (expect 388 passed / 1 ignored after the monotonic
   lifecycle regression test).
3. Re-verify the exact type shapes, observation seam, and lifecycle rules
   against the public [session API](../session-api.md).
4. To wire into the crate (serial integration commit, **not** this branch): add
   `pub mod session;` to `crates/noren-app/src/lib.rs`, then change the first
   non-comment line of `tests/session_domain.rs` from
   `#[path = "../src/session.rs"] mod session;` to `use noren_app::session;`.
