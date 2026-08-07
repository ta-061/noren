# Milestone 3 work breakdown

Status: Planning lane deliverable. Snapshot: 2026-08-07 (Asia/Tokyo), against
`main`. This is analysis and documentation only: it changes no code, adds no
tests, and advances no compatibility matrix row. It decomposes the Milestone 3
scope in [ROADMAP.md](../../ROADMAP.md) into independently landable Issues with
explicit file ownership so parallel lanes do not collide.

This revision supersedes the breakdown previously drafted on branch
`docs/m3-breakdown`, which proposed seven Issues built on a Noren-side layout
tree and flagged an A-vs-B data-structure decision. That decision is now **moot**:
[ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md) drew the
Noren/Zellij responsibility boundary — Noren manages the workspace *outside* the
terminal; Zellij manages tabs, panes, splits, and layout *inside* the terminal.
With no Noren-side layout tree, the flat-list-versus-split-tree question
disappears, and persistence never serializes a layout. This document reflects
that boundary. Everything below is grounded in code read at the current head.

## 1. What exists today that M3 builds on

### Terminal core (`noren-terminal`)

The renderer-independent core owns mutable state behind a narrow contract
([`TerminalEngine`](../../crates/noren-terminal/src/lib.rs) bytes/dimensions in,
immutable `TerminalSnapshot` out, `lib.rs:34-46`):

- **Mutable state shape.** `TerminalState` holds an active `ScreenState`, an
  optional primary screen, a `TerminalModes` snapshot, a pen, a `Parser`, and a
  bounded scrollback `VecDeque<Vec<Cell>>` (`state.rs:653-664`). This remains the
  unit per session.
- **Modes.** `TerminalModes` carries alternate-screen, application-cursor,
  application-keypad, and bracketed-paste flags (`state.rs:259-294`). The app
  reads these to drive key encoding.
- **Scroll regions, alternate screen, cursor save/restore.** DECSTBM margins and
  mode-1049 screen switching with saved-cursor restore are implemented
  (`state.rs:610-631`).
- **Erase/edit, SGR, application modes.** SGR attributes are modeled as
  `CellAttributes`/`Color`/`AnsiColor`, re-exported at `lib.rs:18`. Truecolor SGR
  exists in the public types; a rendered-frame oracle is still absent per
  [status](../coordination/status.md).
- **Bounded scrollback.** `MAX_SCROLLBACK_LINES = 10_000`, primary-screen only,
  eviction-bounded (`state.rs:44-72`).
- **Unicode width.** Cells are width/continuation-aware; `cell_width` uses
  `unicode-width` (`lib.rs:75-77`, `state.rs:82-117`).
- **Selection.** `Selection`/`SelectionMode`/`GridPoint`/`SelectionGrid`
  (`lib.rs:22`; `Selection::new` at `selection.rs:244`).
- **Search.** Renderer-independent scrollback search (`lib.rs:19-22`;
  `Search::new` at `search.rs:116`).
- **Resize.** Preserves the overlapping top-left of active and primary screens
  (`TerminalState::resize` `state.rs:694-701`).

### Application layer (`noren-app`)

- **Single window, single terminal, single PTY.** `NorenApp` owns exactly one
  `terminal: Option<TerminalState>` and one `pty: Option<PtySession>`
  (`main.rs:32-33`). Every path — input, resize, selection, draw — assumes this
  singular terminal. This is the ownership model M3 changes: from "one terminal"
  to "a flat list of sessions, exactly one selected."
- **Event loop.** `winit` `ApplicationHandler` (`main.rs:516-564`) drives
  initialize, window events, `about_to_wait` (resize coalesce + PTY drain), and
  redraw.
- **Key encoder — fixed and hardcoded.** `KeyEncoder` is a pure, code-defined
  mapping from `KeyInput` to bytes (`lib.rs:368-492`), with byte tables mirrored
  from terminal modes in `input.rs`. There is **no binding table loaded from
  configuration**; the map is compiled in.
- **Key translation.** `winit` `KeyEvent` → app-owned `KeyInput`/`KeypadInput`
  (`main.rs:598-681`), then encoded against the single terminal's modes
  (`current_input_mode` `main.rs:353-370`) and sent to the single PTY
  (`send_input` `main.rs:343-351`).
- **Renderer.** A bounded `wgpu` view that consumes **one** `TerminalSnapshot`
  plus an optional status string (`renderer.rs:55-64`; called at `main.rs:489`).
  It is ASCII-raster only and draws no chrome.
- **Geometry.** `GridGeometry` converts window pixels to **one** grid, clamped to
  `MAX_RENDER_ROWS`/`MAX_RENDER_COLS` (`lib.rs:133-185`). This is the only place
  a grid is computed.
- **Clipboard + paste.** Copy/paste with bracketed-paste (mode 2004) gating
  (`main.rs:163-258`, `clipboard.rs`, `encode_paste`).
- **Selection — per-app, single.** `selection`/`drag_origin`/`drag_mode`
  (`main.rs:41-42`), with pixel→grid mapping against the one terminal
  (`grid_point_at` `main.rs:309-341`).
- **Resize propagation to one PTY.** Coalesced physical resize → terminal resize
  → single PTY resize (`main.rs:372-405`).

### What does NOT exist (read, not inferred)

- **No configuration system.** There is no `config` module anywhere under
  `crates/` (only `wgpu::SurfaceConfiguration`, unrelated), and no config
  dependency in `Cargo.toml`. Nothing is user-configurable today.
- **No configurable keybindings** — the encoder is compiled-in.
- **No session/workspace/sidebar model** — one terminal.
- **No persistence.** No `serde`/`Serialize`/`Deserialize` appears anywhere in
  `crates/` (verified by search); `TerminalState` carries no serialization, and
  [open-questions](../coordination/open-questions.md) lists the persistence
  format as design-required.
- **No sidebar or command palette** — the renderer draws terminal content and a
  status string only.
- **No Zellij pass-through** — there is no interception layer; a key either
  encodes to PTY bytes or drops.

## 2. Decomposition into landable Issues

Seven Issues. File ownership is stated per Issue and is the constraint that lets
lanes run in parallel: two Issues that both edit the same file cannot run
concurrently and are instead sequenced by a dependency edge (Section 3).

`main.rs` is the single integration point today (it owns the terminal, PTY,
input dispatch, selection, and redraw). It is therefore called out explicitly:
an Issue that must edit `main.rs` is **not** parallel-safe with another such
Issue unless M3-1 has first introduced the action-dispatch seam described below.

The defining constraint of this milestone, from
[ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): **Noren manages
the workspace outside the terminal; Zellij manages inside.** No Issue models
panes, tabs, splits, or a layout tree. Exactly one session is visible at a time.

### M3-1 — Session model + single-session view + action dispatch seam (FOUNDATION)

**Objective.** Replace the singular `terminal`/`pty` pair with a flat list of
sessions, each owning its own `TerminalState` and `PtySession`, with exactly one
selected and visible. Introduce a small **action/command dispatch seam** so later
Issues (sidebar, palette, keybindings) register behaviors without each editing
`main.rs`.

**Scope.**
- New `sessions.rs`: a `SessionManager` holding `Vec<Session>` (or equivalent)
  and a selected index/id. Each `Session` owns one `TerminalState` and one
  `PtySession` (`PtySession` is already self-contained and multi-instance-safe:
  `noren-pty/src/lib.rs:273-280`). **No layout tree, no panes, no splits.**
- A session is created, selected, and terminated. Terminating a session drops its
  `PtySession` (idempotent shutdown already exists, `lib.rs:420-424`) and its
  `TerminalState`.
- `GridGeometry` stays "window-region → one grid for the selected session"
  (`lib.rs:133-185`); M3-3 (sidebar) carves out a chrome region and the selected
  session gets the remainder.
- `main.rs` rewired to hold a `SessionManager`, route `send_input`/`drain_pty`/
  `redraw`/selection to the **selected session only**, and expose the
  action-dispatch seam.

**Forbidden scope.** No sidebar chrome rendering (M3-3), no keybinding
configuration (M3-4), no persistence (M3-7). No tab, pane, split, or layout
model — ever (see Section 6).

**Dependencies.** None; this is the foundation.

**Acceptance criteria.**
- Multiple sessions can be created; exactly one is selected and rendered at a
  time; the selected session's shell sees its own `stty size` agreeing with the
  allocated grid.
- Keyboard input routes only to the selected session; selecting a different
  session switches input/output/redraw.
- Window resize resizes the selected session's terminal and PTY with no
  zero-size exposure.
- Output from each PTY drains only into its own `TerminalState`.
- Terminating a session reaps its child and joins workers within the existing
  deadline (NFR-004); the session is removed and another (or an empty state) is
  selected.
- Existing single-session behavior is preserved when one session exists.

**File ownership.** `crates/noren-app/src/sessions.rs` (NEW),
`crates/noren-app/src/main.rs`, `crates/noren-app/src/lib.rs`. Introduces the
dispatch seam others depend on.

### M3-2 — Configuration system

**Objective.** A config loader: schema, parse, validate, bounded reload, and a
typed config object the rest of the app reads. This is foundational because
nothing is configurable today.

**Scope.**
- New `config.rs`: schema for the user config (format chosen by this Issue and
  recorded in an ADR), load + validate, last-valid retention on failed reload
  (per [risk R-DL-01](../roadmap/risk-register.md)), no keys/tokens/raw commands
  persisted.
- Integration point: a config handle the keybinding (M3-4) and palette (M3-6)
  Issues consume.

**Forbidden scope.** No keybinding semantics (M3-4), no session model internals
(M3-1), no persistence of sidebar state (M3-7 — config ≠ sidebar state).

**Dependencies.** None; parallel-safe with M3-1 (disjoint files).

**Acceptance criteria.**
- A valid config is loaded and exposed read-only to consumers; an invalid config
  is rejected before activation with a nonzero, diagnosable failure and the last
  valid config retained.
- Reload is bounded and never blocks the UI; a failed reload leaves the session
  usable.
- No secret material is accepted or logged.

**File ownership.** `crates/noren-app/src/config.rs` (NEW),
`crates/noren-app/Cargo.toml` (new parsing dependency, if any). Does **not** edit
`main.rs` beyond a single registration point coordinated with M3-1.

### M3-3 — Sidebar (external-context chrome)

**Objective.** A chrome surface that lists external-context entries — projects,
git worktrees, SSH connections, agents, and terminal sessions — and lets the user
create, select, and terminate sessions from there. This is the visibility layer
over the M3-1 session model. It is where Noren's "workspace outside the terminal"
lives ([ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md)).

**Scope.**
- New `sidebar.rs`: entry list over the `SessionManager` (sessions) plus
  external-context entries (projects, worktrees, SSH targets, agents), selection
  indication, and pointer hit-testing into the chrome.
- Selecting a session entry selects that session (via the M3-1 action seam);
  creating an entry spawns a new session; terminating tears it down.
- `renderer.rs` extended to draw the sidebar chrome region in addition to the
  selected session's terminal snapshot.

**Forbidden scope.** No new terminal-state behavior, no keybinding work, no pane
or layout. The sidebar lists sessions and external context — it does not model or
display tabs/panes inside a session (that is Zellij's surface, not Noren's).
Pointer actions on chrome dispatch through the M3-1 action seam.

**Dependencies.** M3-1 (the session model to display and select).

**Acceptance criteria.**
- Sessions are listed with the selected one marked; clicking a session selects
  it (routed through M3-1); creating/terminating from the sidebar works.
- External-context entries (projects, worktrees, SSH targets, agents) are listed;
  the exact entry schema is an open question (Section 7), but the sidebar renders
  whatever M3-1/M3-7 define.
- Chrome occupies a fixed region; the selected session's terminal grid is
  allocated the remainder and never overlaps the sidebar.
- Resize recomputes both chrome and terminal regions.

**File ownership.** `crates/noren-app/src/sidebar.rs` (NEW),
`crates/noren-app/src/renderer.rs`. Does not edit `main.rs` directly (uses the
action seam).

### M3-4 — Configurable keybindings (Zellij-collision-aware)

**Objective.** Replace the hardcoded `KeyEncoder` dispatch with a binding
manifest: every Noren-side shortcut independently rebindable and disableable, a
default table, and a dispatch path that either consumes a key as a Noren action
(select session, create/terminate, open palette, toggle pass-through) or forwards
it to the selected session's PTY. Bindings must not collide with Zellij chords
the user expects to reach the session.

**Scope.**
- New `keymap.rs`: binding manifest, resolve/bind/disable, collision detection
  against Zellij default/preset chords, and the consume-vs-forward decision.
- `input.rs` stays the byte-table encoder for forwarded keys; the new layer sits
  *before* it.
- Dispatch wired through the M3-1 action seam so it targets the selected session.
- Collision diagnostics surface where a Noren binding shadows a Zellij binding
  (the `noren_zellij_compatible_preset` contract in
  [zellij.md](../compatibility/zellij.md) requires this).

**Forbidden scope.** No pass-through leader mode/state machine (M3-5), no
sidebar/palette UI. Must not change `KeyEncoder`'s byte contract for forwarded
keys. Noren actions are session/sidebar actions only — no tab/pane/split actions
exist to bind.

**Dependencies.** M3-2 (config) and M3-1 (selected-session routing + action
seam).

**Acceptance criteria.**
- Every default shortcut can be rebound and disabled; a disabled/invalid binding
  never creates a keyboard trap (a pointer-invoked recovery path remains — see
  M3-6 and [zellij.md](../compatibility/zellij.md) pass-through row).
- A key not bound to a Noren action is forwarded byte-for-byte to the selected
  PTY exactly as today.
- Collision/shadow diagnostics are deterministic and flag any Noren binding that
  would steal a chord Zellij binds in normal/locked mode.

**File ownership.** `crates/noren-app/src/keymap.rs` (NEW),
`crates/noren-app/src/input.rs`. Edits `main.rs` only via the M3-1 action seam.

### M3-5 — Zellij pass-through mode

**Objective.** A mode in which Noren intercepts the minimal accepted set and
forwards everything else untouched, so a focused Zellij (or any nested
multiplexer/application) retains child input inside the selected session.

**Scope.**
- Leader-bound entry/exit, the frozen interception manifest (the minimal set
  Noren may consume while in the mode), and forwarding semantics for the rest.
- Pointer-invoked and GUI recovery from the mode (never a trapped session).
- Operates on the single selected session — there is no pane routing to decide.

**Forbidden scope.** Does not invent mouse forwarding — that is the separate
#46 blocker (Section 5). Does not change output-side parsing. Does not model
Zellij's layout.

**Dependencies.** M3-4 (interception manifest + leader machinery).

**Acceptance criteria.** Per the pass-through contract in Section 5 and the
`noren_zellij_pass_through` row in [zellij.md](../compatibility/zellij.md): only
the minimal accepted set is intercepted; all other keys continue to the child;
exit works via leader, palette, and GUI; a disabled/invalid/shadowed/unreachable
leader yields config rejection or pointer recovery, never a trap.

**File ownership.** `crates/noren-app/src/keymap.rs` (shared with M3-4 — these
two Issues are sequenced, not parallel) plus a new `passthrough.rs` for the
mode state machine if separated. Sequenced after M3-4.

### M3-6 — Command palette

**Objective.** A fuzzy/action palette that invokes Noren actions (select/create/
terminate session, toggle pass-through, rebind) by pointer and keyboard. This is
the **non-keyboard recovery surface** [zellij.md](../compatibility/zellij.md)
requires remain reachable regardless of binding state.

**Scope.**
- New `palette.rs`: action catalog, fuzzy match, overlay state, invocation via
  the M3-1 action seam.

**Forbidden scope.** No new session model, no renderer chrome beyond the overlay.
No tab/pane/split actions in the catalog — those do not exist.

**Dependencies.** M3-1 (action seam) and M3-2 (config, for rebind/disable
actions). Strengthens M3-4/M3-5 recovery but does not hard-depend on them.

**Acceptance criteria.**
- Every Noren action is reachable through the palette by pointer even when all
  keybindings are disabled.
- Opening the palette does not forward keystrokes to the PTY while it is open;
  closing restores selected-session input.

**File ownership.** `crates/noren-app/src/palette.rs` (NEW). Uses the action
seam; no direct `main.rs` edits.

### M3-7 — Sidebar persistence

**Objective.** Save and restore Noren's **sidebar state** — which projects,
worktrees, SSH targets, agents, and sessions exist, plus which session is
selected — with a versioned, crash-consistent format and atomic writes.

**Scope.**
- New `persist.rs`: serialize/deserialize the sidebar/session *metadata* model,
  atomic save, version migration, reload-on-launch.
- Adds a serialization dependency (none exists today — no `serde` in `crates/`).

**Forbidden scope.** Does **not** persist terminal content, Zellij's tab/pane
layout, or anything Noren cannot authoritatively observe outside the session
([ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md)). Does not
redefine the M3-1 model; serializes whatever sidebar/session metadata M3-1 and
M3-3 settled on. A restored session re-spawns a shell; it is not process
resurrection (see [zellij.md](../compatibility/zellij.md) detach/attach row
obligation).

**Dependencies.** **Hard** sequential: M3-1 (session model) and M3-3 (sidebar
entries), because the serialized shape follows from them.

**Acceptance criteria.**
- A sidebar with multiple sessions and external-context entries round-trips
  through save/reload with the selected session and entry list restored.
- A corrupted/partial file is rejected without data loss; the last valid
  sidebar state is retained.
- No secrets, raw commands, terminal content, or layout are persisted.

**File ownership.** `crates/noren-app/src/persist.rs` (NEW),
`crates/noren-app/Cargo.toml` (serialization dependency).

## 3. Ordering constraints

Hard "must precede" edges (the rest are parallelizable):

1. **M3-1 must precede M3-3.** A sidebar can only list/select sessions that
   exist.
2. **M3-1 must precede M3-7.** Persistence serializes whatever the session/
   sidebar metadata model becomes; it cannot be designed before the model is
   stable.
3. **M3-3 must precede M3-7.** Sidebar persistence includes the external-context
   entries the sidebar defines.
4. **M3-2 must precede M3-4.** Keybindings are read from config; there is no
   config today.
5. **M3-4 must precede M3-5.** Pass-through reuses the interception manifest and
   leader machinery introduced for configurable keybindings.
6. **M3-1 must precede M3-6.** The palette dispatches actions over the session
   model.

Parallel opportunities given file ownership:

- **M3-1 ∥ M3-2** from the start — disjoint files (`sessions.rs`/`main.rs`/
  `lib.rs` vs `config.rs`/`Cargo.toml`).
- After M3-1 lands the action seam, **M3-3** (`sidebar.rs`/`renderer.rs`),
  **M3-6** (`palette.rs`), and the **M3-4 → M3-5** chain (`keymap.rs`/
  `input.rs`/`passthrough.rs`) touch disjoint files and can advance in parallel.
- **M3-7** waits on M3-1 and M3-3; it is the last to land.

The bottleneck is `main.rs`. Any Issue that needs `main.rs` edits is sequenced
through M3-1 (which owns the initial rewire and the dispatch seam). Once that
seam exists, later Issues register against it instead of editing `main.rs`, which
is what unlocks the parallelism above.

## 4. What the boundary settles (the former A-vs-B decision is moot)

The `docs/m3-breakdown` branch flagged a gating architectural decision: when
moving from "one terminal" to "many," should the model be a flat list of panes
per tab plus a separate layout pass (Option A), or a split tree
(tmux/Zellij-style, Option B)? It warned that the serialized format is close to
irreversible once shipped, so the choice needed owner approval before
serialization-relevant structs landed.

[ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md) resolves this by
removing the premise: Noren does not model panes, splits, or layout at all. The
selected session owns one `TerminalState` and one `PtySession`; the window shows
exactly that one session. In-terminal splitting is Zellij's job. The A-vs-B
question disappears, and no Issue in Section 2 needs an owner gate on a data
structure before it can begin.

This is also why persistence (M3-7) is lower-risk than the old M3-7: it
serializes a flat sidebar/session metadata list, not a layout tree whose format
is hard to migrate.

## 5. Zellij pass-through, concretely

Noren targets Zellij compatibility ([zellij.md](../compatibility/zellij.md)) and
Zellij is itself a multiplexer, so "pass-through" must mean a specific thing,
not "zero transformation." Two layers must be kept distinct (the
[Z-2L oracle](../compatibility/zellij.md)): the **outer** boundary
(Noren→session PTY bytes) and the **inner** boundary (Zellij→child bytes). Per
[zellij.md](../compatibility/zellij.md), concretely:

**Forward untouched (do not interpret) when a session is focused:**
- All printable UTF-8, Enter/Backspace/Tab/Escape, arrows, navigation and
  function keys, and their xterm modifier-parameter forms — i.e. today's
  `KeyEncoder` byte contract (`lib.rs:368-492`). The pass-through obligation is
  that these reach the selected session's PTY **unchanged**, the same bytes a
  direct-host control would send.
- Application-cursor/keypad mode selection driven by the **selected session's**
  `TerminalModes` (`state.rs:274-283`), not a global. A focused Zellij session
  sets DECCKM/DECKPAM; Noren must follow *that session's* modes.
- Bracketed-paste markers only when the selected session enabled mode 2004
  (`main.rs:238-244`).

**Interpret (Noren consumes, does not forward) — the minimal accepted set:**
- Only the configured pass-through leader (entry/exit), and only the actions in
  the frozen interception manifest. Everything else forwards. This is the
  `noren_zellij_pass_through` contract in [zellij.md](../compatibility/zellij.md).

**How it interacts with the boundary.** Under
[ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md), Noren routes
bytes to the selected session and nothing finer — there is no Noren pane for
Zellij's panes to coexist with or collide against. Noren must not steal a chord
that Zellij binds in Locked/normal mode ([zellij.md](../compatibility/zellij.md)
Unlock-First row: bound secondary-modifier chords are Zellij actions, not
forwarding candidates — only **unbound** chords forward). Coordinate-sensitive
interaction (mouse) is gated on the path below.

**Remaining blocking gap — mouse input (Issue #46).** Pointer-first Zellij
interaction (pane click/focus, drag-resize, wheel) is dead without a mouse input
encoder. Per [status](../coordination/status.md) and #46, this belongs in an
**input encoder in `noren-app`** (pointer events → mode-selected mouse bytes →
`send_input`), **not** output-side parsing. Mouse-mode *tracking* (DECSET
1000/1002/1003/1006/1015) lives in `noren-terminal` state, but the **encoder**
that turns pointer events into SGR-1006 reports belongs in `noren-app`. Until #46
lands, pass-through covers keyboard forwarding only; mouse-driven Zellij
workflows remain blocked and must not be implied as supported. This is tracked as
#46, not re-scoped into M3-5.

## 6. What M3 must NOT do

This list exists so a later lane cannot drift back into the superseded plan:

- **No native tabs.** Tabs inside a session are Zellij's job. Noren shows exactly
  one selected session.
- **No pane splits.** No horizontal/vertical split, no split tree, no layout
  engine, no focus movement between panes. All of that is Zellij's surface.
- **No layout model.** Noren must not hold, interpret, or compute a layout tree.
  The window is "sidebar region + one session region."
- **No layout persistence.** M3-7 persists sidebar/session metadata only. It must
  never serialize terminal content, Zellij's tab/pane topology, or any structure
  Noren cannot observe outside the session.
- **No duplicated abstraction.** Never give both Noren and Zellij the same
  abstraction (the standing principle in
  [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md)). If a future
  feature concerns display/splitting/manipulation *inside* the terminal, it
  belongs to Zellij, not Noren.

## 7. Open questions

The boundary settles where responsibility lies, not every detail of the sidebar.
These remain open and are not answered here:

1. **Sidebar entry schema.** What fields does each entry type carry? A "project"
   or "git worktree" needs a path; an "SSH target" needs host alias/user/port;
   an "agent" needs an adapter identity. The exact typed shape is deferred to the
   M3-3/M3-1 implementation Issues.
2. **Entry-to-session relationship.** Does selecting an external-context entry
   (e.g., a project) auto-create a new session, focus an existing one, or offer a
   choice? The boundary does not specify this lifecycle rule.
3. **Agent entries.** Does the sidebar's "agents" list manage processes Noren
   launches (FR-011), display adapter state only, or both? FR-011's trust
   boundary is separate; how it surfaces in the sidebar is undecided.
4. **Session re-spawn versus reattach.** When M3-7 restores a session, it
   re-spawns a shell. Whether Noren also reattaches to a Zellij session by name
   (so the user resumes a `zellij attach`) is a policy question the boundary does
   not settle.
5. **Persistence format.** The sidebar-state persistence format (schema,
   crash-consistency, migration) remains a design-required item in
   [open-questions](../coordination/open-questions.md); M3-7 must settle it but
   this breakdown does not pre-select it.

## Non-claims

This document executes nothing in `crates/*/src/`, adds no tests, and advances no
matrix row. It selects no library or persistence format. No milestone date is
promised. Zellij is named nominatively as the compatibility target; no Zellij
code or assets were copied here (legal boundary in
[zellij.md](../compatibility/zellij.md)).
