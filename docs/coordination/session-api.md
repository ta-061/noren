# D-M3-001: Milestone 3 session API

Status: Accepted, internal and reversible. Last updated 2026-08-07.

This is the public product record for the shared Milestone 3 session vocabulary.
It is an in-process API, not a persistence format, wire protocol, or stable
third-party interface.

## Responsibility boundary

Per [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md), Noren
manages sessions outside the terminal while Zellij manages tabs, panes, splits,
and layouts inside one session. Therefore this API contains no pane, tab, split,
or layout type and never inspects terminal content.

Exactly zero or one Noren session is selected at a time.

## Contract

The concrete Rust shapes selected by the session-domain implementation are:

```rust
pub struct SessionId(u64);

pub enum SessionKind {
    Local,
    Project { root: PathBuf },
    Worktree { path: PathBuf },
    Ssh { target: String },
    Agent { name: String },
}

pub enum SessionStatus {
    Starting,
    Running,
    Exited { code: Option<i32> },
    Failed { reason: String },
}

pub struct SessionDescriptor {
    id: SessionId,
    kind: SessionKind,
    status: SessionStatus,
    title: String,
}

pub enum SessionAction {
    Create { kind: SessionKind },
    Select { id: SessionId },
    Close { id: SessionId },
}

pub enum SessionEvent {
    Created(SessionId),
    Selected(Option<SessionId>),
    StatusChanged { id: SessionId, status: SessionStatus },
    Closed(SessionId),
}

pub struct SessionRegistry { /* private fields */ }
pub type SelectedSession = Option<SessionId>;
```

`Local` is the only launchable kind in this milestone. The other kinds reserve
the shared data shape only; their launch and connection semantics remain
unimplemented.

`SessionDescriptor::title` initially uses the generated display id. A later
rename feature may change the title through an explicit API without changing
the rest of this contract.

## Registry and observation seam

`SessionRegistry` owns data only. It creates, selects, queries, and closes
entries but never spawns, waits on, reads from, or terminates a process. Process
ownership belongs to the lifecycle supervisor.

Supervisor facts enter through this registry method:

```rust
observe(
    id: SessionId,
    status: SessionStatus,
) -> Result<Option<SessionEvent>, SessionError>
```

Observation is deliberately not a `SessionAction`. Actions represent user or UI
requests handled by the dispatch seam; an observed process status is a fact from
the supervisor. Keeping those channels distinct lets the sidebar, keybindings,
and command palette use the three exhaustive action variants without inventing
runtime truth.

`SessionError` is an implementation-local addition with `UnknownSession` and
`InvalidStatusTransition` variants.

## Lifecycle rules

- Creation records `Starting`; it never infers `Running`.
- `Starting` may advance to `Running` or directly to a terminal status.
- `Running` may advance to `Exited` or `Failed`.
- Once terminal, a session cannot return to `Starting` or `Running`.
- A later terminal observation may refine an earlier terminal payload or
  variant, such as `Failed` followed by `Exited { code: Some(...) }`.
- Re-observing an identical status is a no-op and emits no event.
- An invalid transition returns `InvalidStatusTransition` without mutation.

These rules prevent stale or reordered observations from resurrecting a dead
session while still allowing a provisional terminal report to gain an exit
code later.

## Identity and bounded state

A `SessionId` is opaque and stable only within one `SessionRegistry` lifetime.
Noren uses one registry per running application. IDs from different registries
must never be mixed: both registries begin their private counters at one, so
equal numeric values do not establish cross-registry identity. IDs are not
persistence keys.

Closing removes the descriptor and clears the selection when necessary. The
registry retains no tombstones or event history, so repeated create/close cycles
do not accumulate dead entries. IDs are not reused within a registry lifetime.

## Deferred decisions

- Persistence schema and restoration behavior.
- SSH connection and reconnect semantics.
- Agent launch and resume semantics.
- Whether selecting a project entry creates a session automatically.
- Whether restoration spawns a new shell or reattaches by a Zellij session
  name.

Any lane that needs a pane, tab, split, layout, persistence key, or additional
action variant must update this decision explicitly instead of silently forking
the shared type shapes.
