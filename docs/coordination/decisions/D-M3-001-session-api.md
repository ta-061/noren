# D-M3-001: Milestone 3 session API contract

- Status: Accepted (coordinator decision; reversible internal API)
- Date: 2026-08-07
- Scope: the minimum shared vocabulary M3-1 lanes code against
- Governed by: ADR 0003 (Noren/Zellij responsibility boundary), landing in PR #68

## Why this exists

Five M3 lanes run in parallel against a session model none of them owns alone.
Without a fixed contract they would each invent an incompatible shape and the
integration commit would be a rewrite. This records the contract so lanes can
start now.

This is an **internal, reversible** API. It is not a persistence format, not a
wire protocol, and not a public interface — those need their own decisions, and
per the standing stop conditions a persistence format is an owner decision.

## The boundary this contract must respect

ADR 0003: Noren manages the workspace **outside** the terminal; Zellij manages
it **inside**. Therefore:

- there is no `Pane`, no `Tab`, no `Layout`, and no split anywhere in this model;
- a session is opaque — Noren knows it exists, its kind, and whether it is
  alive, and **nothing about what is displayed inside it**;
- exactly **one** session is selected and visible at a time.

Any lane that finds itself wanting a pane or layout type has hit the boundary and
must stop and escalate rather than add one.

## Contract

```rust
/// Opaque, stable within one run. Not a persistence key.
pub struct SessionId(u64);

/// What external context a session belongs to. Zellij's own tabs and panes
/// live *inside* one of these and are not represented here.
pub enum SessionKind {
    Local,                 // a shell on this machine
    Project { .. },        // a shell rooted in a project directory
    Worktree { .. },       // a shell rooted in a git worktree
    Ssh { .. },            // reserved shape; M3-1 does not implement SSH
    Agent { .. },          // reserved shape; M3-1 does not launch agents
}

pub enum SessionStatus {
    Starting,
    Running,
    Exited { code: Option<i32> },
    Failed { reason: String },
}

/// Everything the sidebar needs to render one entry. Renderer-independent:
/// no colors, no geometry, no widget types.
pub struct SessionDescriptor {
    id: SessionId,
    kind: SessionKind,
    status: SessionStatus,
    title: String,
}

/// Requests *into* the domain. The dispatch seam speaks these so the sidebar,
/// keybindings, and palette never call the registry directly.
pub enum SessionAction {
    Create { kind: SessionKind },
    Select { id: SessionId },
    Close { id: SessionId },
}

/// Facts *out of* the domain, for observers to react to.
pub enum SessionEvent {
    Created(SessionId),
    Selected(Option<SessionId>),
    StatusChanged { id: SessionId, status: SessionStatus },
    Closed(SessionId),
}

/// Owns the set of sessions and the single selection. Holds no process handles:
/// the lifecycle supervisor owns those, so the domain stays unit-testable
/// without spawning anything.
pub struct SessionRegistry { .. }

/// Zero or one. Never more.
pub type SelectedSession = Option<SessionId>;
```

## Invariants the lanes must uphold

1. **At most one selected session.** `SelectedSession` is `Option`, and closing
   the selected session must leave the selection either empty or on another
   existing session — never dangling.
2. **No session count cap is implied**, but the registry must not grow without
   bound from repeated create/close cycles; ids may be reused only if that
   cannot alias a live session.
3. **`SessionRegistry` spawns nothing.** It is pure state, so the domain tests
   run without processes. Process ownership belongs to the supervisor.
4. **Status is reported, never inferred.** A session is `Running` because the
   supervisor observed it, not because creation returned.
5. **Nothing in this model describes terminal content or layout.**

## Deliberately not fixed here

- Persistence format and schema — owner decision, and M3-7's problem.
- SSH connection semantics — `Ssh` is a reserved shape only; FR-010 governs.
- Agent launch semantics — `Agent` is a reserved shape only; FR-011 governs.
- Whether selecting a project entry auto-creates a session or offers a choice.
- Whether a restored session re-spawns a shell or reattaches by Zellij name.

The last two are open questions in the M3 breakdown. Lanes must not silently
settle them.

## Ownership

`SessionId`, `SessionKind`, `SessionStatus`, `SessionDescriptor`,
`SessionRegistry`, `SessionAction`, and `SessionEvent` are defined **once**, by
the session-domain lane (M3-1a), in a new module. Every other lane imports them
and may not redefine or shadow them. A lane needing a change to the contract
escalates instead of forking it.
