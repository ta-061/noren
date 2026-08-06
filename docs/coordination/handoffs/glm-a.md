# Handoff — M3 session domain lane (GLM, `glm-a`)

> **Note on the handoff template:** `docs/coordination/handoffs/TEMPLATE.md`
> did **not** exist on `origin/main` at the time this lane ran. This file is
> structured from the lane prompt's required fields rather than from a template.

## Identity

- **Lane:** `glm-a` (session domain model), engine GLM 5.2 via opencode.
- **Branch:** `agent/m3-session-domain`, branched from `origin/main` @
  `1d329a5` (353 workspace tests passing at branch point).
- **Initial code commit:** `d31e3ac75a9a8f5cf16b6b4ac9d7dcb4e33ff27e`
- **Conformance-fix commit (current authoritative code):**
  `df3afcc65b3487a3cfe6627520acb0b3fd3544e7`
- **Independent review:** `docs/coordination/reviews/M3-1a-review.md` (commit
  `b0f61c3`), one MAJOR (contract fork), no BLOCKER.
- **Base SHA:** `1d329a5`.
- **Diff vs main:** only the two leased files plus this handoff and the review;
  **no deletions, no edits to `lib.rs`, `Cargo.toml`, `Cargo.lock`, or
  `status.md`** (verified: their combined diff vs main is empty).

## Revision history

1. `d31e3ac` — initial model. Implemented to the lane prompt because the named
   spec files (`M3-1a.md`, `D-M3-001-session-api.md`, ADR 0003) were **absent**
   from `origin/main`. The handoff explicitly flagged this and asked a reviewer
   to diff against the real contract when it landed.
2. `b0f61c3` — independent Qwen review. Found one MAJOR: the public types fork
   the D-M3-001 contract in 6 of 8 places (the contract lives in the fleet repo
   at `state/D-M3-001-session-api.md`; the review quotes it side-by-side). The
   coordinator judged the finding real.
3. **`df3afcc` — conformance fix (current).** All six deviations conformed to
   the contract as written; none were kept. Details below.

## The conformance fix (`df3afcc`) — all 6 deviations conformed, 0 kept

For each deviation I chose to **conform** rather than argue a better shape: the
contract was written to be imported by four other lanes, not to be optimal.

| Type | Deviation | Resolution |
| --- | --- | --- |
| `SessionKind` | missing `Project`/`Worktree`; `Ssh`/`Agent` were unit | **Conformed:** `Local`, `Project{root:PathBuf}`, `Worktree{path:PathBuf}`, `Ssh{target:String}`, `Agent{name:String}` |
| `SessionStatus` | `Created` vs `Starting`; dropped `Exited.code`/`Failed.reason` | **Conformed:** `Starting`, `Running`, `Exited{code:Option<i32>}`, `Failed{reason:String}` |
| `SessionDescriptor` | `label:Option<String>` vs `title:String` | **Conformed:** `title:String`, registry-generated at create (the contract `Create` carries no title) |
| `SessionAction` | added `Observe` and `Create.label` | **Conformed:** reduced to `{Create{kind}, Select{id}, Close{id}}` |
| `SessionEvent` | `Created{id,descriptor}`, `SelectionChanged`, `StatusChanged{id,status}` | **Conformed:** tuple `Created(SessionId)`, `Selected(Option<SessionId>)`, `StatusChanged` (unit), `Closed(SessionId)` |
| `SelectedSession` | `struct{id,descriptor}` | **Conformed:** `pub type SelectedSession = Option<SessionId>;` |

`SessionId` and `SessionRegistry` already conformed (per the review); `SessionError`
is a local addition the review accepted (D-M3-001 defines no error type).

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
  `Some(StatusChanged)` on change. This preserves invariant 3 (status is only set
  from a reported observation) without re-forking `SessionAction`.

### Escalation item for the coordinator (do not let this stay implicit)

`SessionRegistry::observe` is the only way a session advances past `Starting`,
yet it is a registry method, not one of the three contract `SessionAction`
variants. This is **not** a contract-type deviation (`SessionRegistry` conforms;
methods are not specified by D-M3-001), but the contract's `SessionAction` set
has no observation path, so the question is open: **should D-M3-001 ratify an
observation action, or is a registry method the intended seam?** The review
explicitly flagged this as an escalate-don't-silently-keep item. I kept the
mechanism (invariant 3 requires it) and am calling it out here rather than
dropping it or smuggling it back into `SessionAction`.

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
  `StatusChanged`, `Closed(SessionId)`.
- `SelectedSession` — `pub type SelectedSession = Option<SessionId>;`.
- `SessionError` — `UnknownSession` (`Display + std::error::Error`).
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
   `session_event_matches_the_contract_variants`) fail to build if anyone
   re-forks the contract enum shapes.

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

On `agent/m3-session-domain` at fix commit `df3afcc`, macOS arm64, rustc 1.88.0.

```
$ cargo fmt --all && cargo fmt --all --check   → exit 0 (clean)
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile; exit 0, 0 warnings
$ cargo test --workspace                       → exit 0
    PASSED=387 FAILED=0 IGNORED=1
```

Two compile errors were hit and fixed during the fix run: a borrow of a
temporary `Descriptor` (`.title()` on an unnamed `get().unwrap()`) and a
mutable-then-immutable borrow of `registry` in one expression. Both fixed by
binding intermediates; the clean clippy output above is post-fix.

### Test totals (`cargo test --workspace`)

**387 passed, 0 failed, 1 ignored.** Baseline 353 → `session_domain` now **34**
(was 32; +3 contract-shape/type-alias guards, −2 removed label/struct-view
tests). Reconciles: 353 + 34 = 387.

## What could NOT be verified

- **The module is not compiled as part of `noren-app`'s library yet** (`mod
  session;` absent from `lib.rs` by lease). It compiles only via the `#[path]`
  test. `noren_app::session::*` cannot resolve until the serial wiring commit.
- **`SessionKind` struct-variant field names are inferred.** The review quotes
  D-M3-001 with `{..}` ellipsis for `Project`/`Worktree`/`Ssh`/`Agent` payloads;
  the full contract file is **not in this repo**. I chose `root`/`path`
  (`PathBuf`) and `target`/`name` (`String`) as the conventional names. **The
  coordinator must confirm exact field names/types against the canonical
  D-M3-001** — if they differ, four downstream lanes coding against my names
  would break. This is the single highest-risk unverifiable item.
- **`SessionAction` lacks an observation path in the contract.** I implemented
  observation as a registry method (see the escalation item). Whether D-M3-001
  intends an action variant is not knowable from this repo.
- **`title` generation policy is unspecified by the contract.** I generate the
  display id; the contract only requires `title: String`.
- D-M3-001, `M3-1a.md`, and ADR 0003 are still absent from `origin/main`; I
  worked from the review's quoted contract.

## Authorship / conflict of interest

- **I (GLM `glm-a`) authored all the code** (`session.rs`, `session_domain.rs`)
  and this handoff, across both the initial commit and the conformance fix. I
  did **not** author the review (`b0f61c3` is an independent Qwen lane). Per
  fleet policy an independent lane should review the fix, not this one.

## Resume instructions

1. `git checkout agent/m3-session-domain`; confirm fix commit `df3afcc`.
2. Re-run the gate: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` (expect 387 passed / 1 ignored).
3. **Reconcile `SessionKind` field names** against canonical D-M3-001 before any
   downstream lane codes against them.
4. Decide the `observe` escalation (ratify as a contract action vs. keep as a
   registry method).
5. To wire into the crate (serial integration commit, **not** this branch): add
   `pub mod session;` to `crates/noren-app/src/lib.rs`, then change the first
   non-comment line of `tests/session_domain.rs` from
   `#[path = "../src/session.rs"] mod session;` to `use noren_app::session;`.
