# ADR 0003: Noren/Zellij responsibility boundary — outside vs inside the terminal

- Status: Accepted
- Date: 2026-08-07
- Decision owners: Human owner (ta-061) stated the boundary; this lane records it
- Related Issue/RFC: Owner-stated scope change; supersedes the M3 architecture
  question flagged in the `docs/m3-breakdown` branch

## Context

Noren targets Zellij compatibility ([zellij.md](../compatibility/zellij.md)) and
Zellij is itself a terminal multiplexer: it owns tabs, pane splits, layout, and
focus movement *inside* a terminal session. A parallel Noren-side layout model
would duplicate Zellij's abstractions in a second layer, and the two layers would
have to stay consistent without either being able to see the other's state.

This hazard was already documented before the boundary was drawn.
[zellij.md](../compatibility/zellij.md) records it in two places:

- The Pane operations row requires a test that asserts "no accidental Noren pane
  action" and lists "which layer owns a split" as an explicit layout variable
  ([zellij.md:279](../compatibility/zellij.md)).
- The layout unknowns section states that "Noren panes/tabs and Zellij
  panes/tabs are different layout layers" and that visual similarity alone is
  insufficient evidence
  ([zellij.md:313-315](../compatibility/zellij.md)).

The milestone-3 work breakdown (branch `docs/m3-breakdown`,
`docs/roadmap/milestone-3-breakdown.md`) decomposed M3 into seven Issues built on
a Noren-side layout tree and flagged an A-vs-B architecture decision (flat pane
list per tab vs a split tree) as needing owner approval before the
serialization-relevant structs could land, because the persisted format would be
effectively irreversible once shipped.

The owner resolved the boundary instead of the data structure.

## Decision drivers

- Avoid two layers owning the same abstraction (the standing principle the owner
  stated: never give both layers the same abstraction).
- Keep Noren's persistence surface small and free of layout that Noren cannot
  authoritatively observe.
- Let Zellij (or any nested multiplexer) retain full control of in-terminal
  layout without Noren reinterpreting or fighting it.
- Make the architecture simpler: collapse the open A-vs-B layout question rather
  than answer it.

## Options considered

1. **Noren owns a full layout tree (the `docs/m3-breakdown` plan).** Noren
   models tabs, panes, splits, and focus; persistence serializes that tree.
   Rejected: it duplicates Zellij's abstractions, forces the irreversible A-vs-B
   format decision, and requires asserting "no accidental Noren pane action"
   against a layer that *does* perform pane actions — a structural contradiction
   of [zellij.md:279](../compatibility/zellij.md).
2. **Noren owns a flat pane list only (Option A in the breakdown).** Rejected for
   the same duplication reason; a flat list is still a layout model that must
   stay consistent with Zellij's and still gets serialized.
3. **Owner's boundary (this ADR).** Noren manages the workspace *outside* the
   terminal; Zellij manages the workspace *inside* the terminal. Each layer owns
   disjoint abstractions.

## Decision

Noren manages the workspace OUTSIDE the terminal. Zellij manages the workspace
INSIDE the terminal.

Concretely:

- Noren keeps a **sidebar** for external context: projects, git worktrees, SSH
  connections, agents, and terminal sessions. Sessions are created, selected, and
  terminated from the sidebar.
- The right side of the window shows **exactly one selected terminal session**.
- Tabs, pane splits, layout, and focus movement *inside* that terminal session
  are **Zellij's job** (or whatever multiplexer/application runs inside the
  session).
- Noren does **not** implement native tabs or pane splits that duplicate Zellij.
- Noren does **not** hold, interpret, or persist Zellij's internal layout
  structure.

Standing principle for future features:

- A feature that manages a resource or session **outside** the terminal belongs
  to Noren.
- A feature that concerns display, splitting, or manipulation **inside** the
  terminal belongs to Zellij.
- **Never give both layers the same abstraction.**

## Consequences

The flat-list-versus-split-tree architecture question **disappears**: with no
Noren-side layout tree, there is nothing to model as a flat list or a split tree.
M3 no longer needs an A-vs-B gating decision before implementation.

Noren-side persistence never serializes a layout. What Noren *may* persist is its
own sidebar state: which projects, worktrees, SSH targets, agents, and sessions
exist, plus which session is selected. It must not persist terminal content,
Zellij's tab/pane layout, or anything Noren cannot authoritatively observe
outside the session.

The "one selected session" model replaces the "one terminal" ownership in
`noren-app` today (`main.rs:32-33` owns exactly one `TerminalState` and one
`PtySession`). M3 generalizes that to a flat list of sessions with exactly one
selected, rather than to a tree of panes.

This decision narrows, not widens, scope: native tabs and pane splits are removed
entirely. Configurable keybindings remain, but must avoid collision with Zellij
bindings (see [zellij.md](../compatibility/zellij.md) preset and pass-through
rows); the command palette remains as the non-keyboard recovery surface those rows
require.

## Security and reliability impact

A smaller Noren-side state surface means less to corrupt or leak. Noren never
holds layout it cannot verify, so a persistence bug cannot desynchronize a
serialized layout from the live Zellij layout. Failure isolation simplifies: one
session dies, not a pane inside a shared layout tree. No new security surface is
introduced; the SSH (FR-010) and agent (FR-011) boundaries are unchanged. This
ADR does not implement anything; it records a boundary.

## Validation evidence

No executable evidence exists yet — every affected requirement and matrix row
remains **Planned** or **Not planned**. This ADR is the design evidence that
resolves the boundary. The pre-existing hazard evidence is
[zellij.md:279](../compatibility/zellij.md) ("no accidental Noren pane action";
"which layer owns a split" as an explicit variable) and
[zellij.md:313-315](../compatibility/zellij.md) (separate layout layers). The
`docs/m3-breakdown` branch's Section 4 is the record of the A-vs-B question this
decision makes moot.

## Reversal or replacement plan

Reversal would mean Noren reintroducing native tabs/panes or a layout tree. That
would require a superseding ADR that (a) re-opens the A-vs-B format question with
owner approval, (b) reconciles the duplicated abstraction against the standing
principle, and (c) re-establishes how "no accidental Noren pane action" is
asserted when Noren itself performs pane actions. Until then, native tabs, pane
splits, and layout modeling are out of scope for every milestone.

## What this supersedes

- The A-vs-B architecture decision flagged in `docs/roadmap/milestone-3-breakdown.md`
  (branch `docs/m3-breakdown`, Section 4): **moot**. No Noren-side layout tree is
  built; no format choice is needed.
- `FR-009` in [v0.1.md](../requirements/v0.1.md) as originally written ("Native
  tabs, panes, workspaces, persistence..."): rewritten to the new boundary.
- Every cmux-parity row that assumed Noren-side panes: reclassified in
  [cmux-parity.md](../compatibility/cmux-parity.md) with a recorded disposition.

## Dissent and unresolved questions

The owner stated the boundary; this lane does not relitigate it. Open questions
the boundary does not settle are listed in the revised
[milestone-3-breakdown.md](../roadmap/milestone-3-breakdown.md) under "Open
questions." This ADR invents no answer for them.
