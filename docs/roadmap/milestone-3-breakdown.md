# Milestone 3 work breakdown

Status: Planning lane deliverable. Snapshot: 2026-08-07 (Asia/Tokyo), against
`main` at `8d44960`. This is analysis and documentation only: it changes no
code, adds no tests, and advances no compatibility matrix row. It decomposes the
Milestone 3 scope in [ROADMAP.md](../../ROADMAP.md) — *tabs, panes, workspaces,
persistence, sidebar, palette, configurable keybindings, Zellij pass-through* —
into independently landable Issues with explicit file ownership so parallel
lanes do not collide.

Everything below is grounded in code read at this head. Where the
[gap analysis](../compatibility/zellij-gap-analysis.md) (dated 2026-08-05 at
`b3391cc`) is cited, note it predates several landings and is re-checked here.

## 1. What exists today that M3 builds on

### Terminal core (`noren-terminal`)

The renderer-independent core owns mutable state behind a narrow contract
([`TerminalEngine`](../../crates/noren-terminal/src/lib.rs) bytes/dimensions in,
immutable `TerminalSnapshot` out, `lib.rs:34-46`):

- **Mutable state shape.** `TerminalState` holds an active `ScreenState`, an
  optional primary screen, a `TerminalModes` snapshot, a pen, a `Parser`, and a
  bounded scrollback `VecDeque<Vec<Cell>>` (`state.rs:653-664`). This is the
  unit that becomes "many" under M3.
- **Modes.** `TerminalModes` carries alternate-screen, application-cursor,
  application-keypad, and bracketed-paste flags (`state.rs:259-294`). The app
  reads these to drive key encoding.
- **Scroll regions, alternate screen, cursor save/restore.** DECSTBM margins and
  mode-1049 screen switching with saved-cursor restore are implemented
  (`state.rs:610-631` resize/save/restore on `ScreenState`). The gap analysis
  verified the load-bearing subset round-trips.
- **Erase/edit, SGR, application modes.** SGR attributes are modeled as
  `CellAttributes`/`Color`/`AnsiColor`, re-exported at `lib.rs:18`. Indexed and
  truecolor SGR landed after the gap analysis (PR #56, `712e368`/`290bfa5`); the
  public color types exist, but I did **not** read the renderer's color path, so
  "truecolor reaches the screen" is an *inference* pending a rendered-frame
  oracle (still absent per [status](../coordination/status.md)).
- **Bounded scrollback.** `MAX_SCROLLBACK_LINES = 10_000`, primary-screen only,
  eviction-bounded, with a documented per-cell/per-line memory ceiling
  (`state.rs:44-72`).
- **Unicode width.** Cells are width/continuation-aware; `cell_width` uses
  `unicode-width` (`lib.rs:75-77`, `state.rs:82-117`). Wide-cell grid invariants
  are repaired per row.
- **Selection.** `Selection`/`SelectionMode`/`GridPoint`/`SelectionGrid`
  (`lib.rs:22`; `Selection::new` at `selection.rs:244`, `is_valid` at
  `selection.rs:317`). Extract is grid-stamp-aware and expires on resize/output.
- **Search.** Renderer-independent scrollback search (`Search`, `SearchIter`,
  etc., `lib.rs:19-22`; `Search::new` at `search.rs:116`), landed as PR #65.
- **Resize.** Preserves the overlapping top-left of active and primary screens
  (`TerminalState::resize` `state.rs:694-701`).

### Application layer (`noren-app`)

- **Single window, single terminal, single PTY.** `NorenApp` owns exactly one
  `terminal: Option<TerminalState>` and one `pty: Option<PtySession>`
  (`main.rs:32-33`). Every path — input, resize, selection, draw — assumes this
  singular terminal. This is the ownership model M3 changes.
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
  (`main.rs:163-258`, `clipboard.rs`, `encode_paste`). Note the gap analysis
  row 5 ("no paste path") is now stale: paste exists.
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
- **No tabs, panes, or workspace model** — one terminal.
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

### M3-1 — Pane, tab, and workspace model + layout engine (FOUNDATION)

**Objective.** Replace the singular `terminal`/`pty` pair with a workspace model
(tabs containing panes in a layout), a focus concept, and per-pane routing of
input, resize, output, and selection. Introduce a small **action/command
dispatch seam** so later Issues (sidebar, palette, keybindings) register
behaviors without each editing `main.rs`.

**Scope.**
- New `workspace.rs`: `Workspace` → `Tab` → pane container, a layout model, a
  focused-pane cursor, and an action enum (focus next/prev, split, close, etc.).
- Each pane owns its own `TerminalState` and `PtySession` (`PtySession` is
  already self-contained and multi-instance-safe: `noren-pty/src/lib.rs:273-280`).
- `GridGeometry` generalized from "window → one grid" to "window → layout →
  per-pane grids" (`lib.rs:133-185`).
- `main.rs` rewired to hold a `Workspace`, route `send_input`/`drain_pty`/
  `redraw`/selection to the focused pane, and expose the action-dispatch seam.

**Forbidden scope.** No persistence format (M3-7), no chrome rendering (M3-5),
no keybinding configuration (M3-3). The layout **data shape** is fixed by the
gated decision (Section 4) before this Issue implements serialization-relevant
structs.

**Dependencies.** Gated on the Section 4 architectural decision (owner
approval). No Issue depends on nothing else first, but M3-2 may proceed in
parallel.

**Acceptance criteria.**
- Two panes render side by side; each has an independent shell (`stty size`
  agrees with its allocated grid, not the window).
- Keyboard input routes only to the focused pane; focus is switchable.
- Window resize re-layouts all panes and resizes each PTY with no zero-size
  exposure.
- Output from each PTY drains only into its own `TerminalState`.
- Selection is per-focused-pane and expires correctly on cross-pane focus and
  resize.
- Existing single-pane behavior is preserved when one tab holds one pane.

**File ownership.** `crates/noren-app/src/workspace.rs` (NEW),
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
- Integration point: a config handle the keybinding (M3-3) and palette (M3-6)
  Issues consume.

**Forbidden scope.** No keybinding semantics (M3-3), no pane model (M3-1), no
persistence of workspace state (M3-7 — config ≠ workspace state).

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

### M3-3 — Configurable keybindings

**Objective.** Replace the hardcoded `KeyEncoder` dispatch with a binding
manifest: every shortcut independently rebindable and disableable, a default
table, and a dispatch path that either consumes a key as a Noren action or
forwards it to the focused pane's PTY. This is the substrate pass-through
(M3-4) sits on.

**Scope.**
- New `keymap.rs`: binding manifest, resolve/bind/disable, collision detection,
  and the consume-vs-forward decision.
- `input.rs` stays the byte-table encoder for forwarded keys; the new layer sits
  *before* it.
- Dispatch wired through the M3-1 action seam so it targets the focused pane.

**Forbidden scope.** No pass-through leader mode/state machine (M3-4), no
sidebar/palette UI. Must not change `KeyEncoder`'s byte contract for forwarded
keys.

**Dependencies.** M3-2 (config) and M3-1 (focused-pane routing + action seam).

**Acceptance criteria.**
- Every default shortcut can be rebound and disabled; a disabled/invalid binding
  never creates a keyboard trap (a pointer-invoked recovery path remains — see
  M3-6 and [zellij.md](../compatibility/zellij.md) pass-through row).
- A key not bound to a Noren action is forwarded byte-for-byte to the focused
  PTY exactly as today.
- Collision/shadow diagnostics are deterministic.

**File ownership.** `crates/noren-app/src/keymap.rs` (NEW),
`crates/noren-app/src/input.rs`. Edits `main.rs` only via the M3-1 action seam.

### M3-4 — Zellij pass-through mode

**Objective.** A mode in which Noren intercepts the minimal accepted set and
forwards everything else untouched, so a focused Zellij (or any nested
multiplexer) retains child input. Concretely defined in Section 5.

**Scope.**
- Leader-bound entry/exit, the frozen interception manifest (the minimal set
  Noren may consume while in the mode), and forwarding semantics for the rest.
- Pointer-invoked and GUI recovery from the mode (never a trapped session).

**Forbidden scope.** Does not invent mouse forwarding — that is the separate
#46 blocker (Section 5). Does not change output-side parsing.

**Dependencies.** M3-3 (interception manifest + leader machinery).

**Acceptance criteria.** Per the pass-through contract in Section 5 and the
`noren_zellij_pass_through` row in [zellij.md](../compatibility/zellij.md): only
the minimal accepted set is intercepted; all other keys continue to the child;
exit works via leader, palette, and GUI; a disabled/invalid/shadowed/unreachable
leader yields config rejection or pointer recovery, never a trap.

**File ownership.** `crates/noren-app/src/keymap.rs` (shared with M3-3 — these
two Issues are sequenced, not parallel) plus a new `passthrough.rs` for the
mode state machine if separated. Sequenced after M3-3.

### M3-5 — Sidebar (workspace chrome)

**Objective.** A chrome surface that lists tabs and panes, shows focus, and
allows focus/switch via pointer — the visibility layer over the M3-1 model.

**Scope.**
- New `sidebar.rs`: tab/pane list view over `Workspace`, focus indication,
  pointer hit-testing into the chrome.
- `renderer.rs` extended to draw chrome regions in addition to terminal
  snapshots.

**Forbidden scope.** No new terminal-state behavior, no keybinding work. Pointer
actions on chrome dispatch through the M3-1 action seam.

**Dependencies.** M3-1 (the tree to display).

**Acceptance criteria.**
- Tabs and panes are listed with the focused one marked; clicking a pane focuses
  it (routed through M3-1).
- Chrome occupies a fixed region; terminal grids are allocated the remainder and
  never overlap the sidebar.
- Resize recomputes both chrome and terminal regions.

**File ownership.** `crates/noren-app/src/sidebar.rs` (NEW),
`crates/noren-app/src/renderer.rs`. Does not edit `main.rs` directly (uses the
action seam).

### M3-6 — Command palette

**Objective.** A fuzzy/action palette that invokes Noren actions (focus, split,
close, switch tab, exit pass-through, rebind) by pointer and keyboard. This is
the **non-keyboard recovery surface** [zellij.md](../compatibility/zellij.md)
requires remain reachable regardless of binding state.

**Scope.**
- New `palette.rs`: action catalog, fuzzy match, overlay state, invocation via
  the M3-1 action seam.

**Forbidden scope.** No new pane model, no renderer chrome beyond the overlay.

**Dependencies.** M3-1 (action seam) and M3-2 (config, for rebind/disable
actions). Strengthens M3-3/M3-4 recovery but does not hard-depend on them.

**Acceptance criteria.**
- Every Noren action is reachable through the palette by pointer even when all
  keybindings are disabled.
- Opening the palette does not forward keystrokes to the PTY while it is open;
  closing restores focused-pane input.

**File ownership.** `crates/noren-app/src/palette.rs` (NEW). Uses the action
seam; no direct `main.rs` edits.

### M3-7 — Workspace persistence

**Objective.** Save and restore the workspace (tabs, panes, layout, focus) with
a versioned, crash-consistent format and atomic writes.

**Scope.**
- New `persist.rs`: serialize/deserialize the M3-1 model, atomic save, version
  migration, reload-on-launch.
- Adds a serialization dependency (none exists today — no `serde` in `crates/`).

**Forbidden scope.** Does not redefine the M3-1 model; serializes whatever shape
M3-1 settled on. Does not persist PTY runtime state as live processes (a restored
workspace re-spawns shells; it is not process resurrection — see
[zellij.md](../compatibility/zellij.md) detach/attach row obligation).

**Dependencies.** **Hard** sequential: M3-1, and specifically the gated decision
(Section 4), because the serialized format is close to irreversible once shipped
(see risk [R-DL-01](../roadmap/risk-register.md)).

**Acceptance criteria.**
- A workspace with multiple tabs/panes round-trips through save/reload with
  layout and focus restored.
- A corrupted/partial file is rejected without data loss; the last valid
  workspace is retained.
- No secrets, raw commands, or clipboard contents are persisted.

**File ownership.** `crates/noren-app/src/persist.rs` (NEW),
`crates/noren-app/Cargo.toml` (serialization dependency).

## 3. Ordering constraints

Hard "must precede" edges (the rest are parallelizable):

1. **Architectural decision (Section 4) must precede M3-1's data shape and M3-7's
   format.** The layout data structure, once serialized, is effectively
   irreversible; settling it first is a precondition, not a preference.
2. **M3-1 must precede M3-5.** A sidebar can only list a tree that exists.
3. **M3-1 must precede M3-7.** Persistence serializes whatever the pane/tab model
   becomes; it cannot be designed before the model is stable.
4. **M3-2 must precede M3-3.** Keybindings are read from config; there is no
   config today.
5. **M3-3 must precede M3-4.** Pass-through reuses the interception manifest and
   leader machinery introduced for configurable keybindings.
6. **M3-1 must precede M3-6.** The palette dispatches actions over the workspace.

Parallel opportunities given file ownership:

- **M3-1 ∥ M3-2** from the start — disjoint files (`workspace.rs`/`main.rs`/
  `lib.rs` vs `config.rs`/`Cargo.toml`).
- After M3-1 lands the action seam, **M3-5** (`sidebar.rs`/`renderer.rs`),
  **M3-6** (`palette.rs`), and the **M3-3 → M3-4** chain (`keymap.rs`/
  `input.rs`/`passthrough.rs`) touch disjoint files and can advance in parallel.
- **M3-7** waits on the decision and on M3-1; it is the last to land.

The bottleneck is `main.rs`. Any Issue that needs `main.rs` edits is sequenced
through M3-1 (which owns the initial rewire and the dispatch seam). Once that
seam exists, later Issues register against it instead of editing `main.rs`, which
is what unlocks the parallelism above.

## 4. Architectural decision M3 forces — FLAGGED, not decided

Multiple panes means multiple PTYs and multiple `TerminalState`s. Today the app
owns **one** terminal (`main.rs:32-33`) and every path assumes exactly one:
modes read from the one terminal (`main.rs:353-370`), input to the one PTY
(`main.rs:343-351`), pixel→grid mapped against the one terminal
(`main.rs:309-341`), redraw snapshots the one terminal (`main.rs:489`), and
`GridGeometry` computes one grid (`lib.rs:133-185`). M3 moves the ownership
model from **"one terminal"** to **"a tree of terminals"**, and persistence
(M3-7) serializes that tree — which is close to irreversible once a format
ships. **This decision needs owner approval before M3-1 implements the
serialization-relevant structs and before M3-7 begins.** Options and trade-offs
only:

**A. Flat list of panes per tab + a separate layout pass.**
- *Shape:* `Tab { panes: Vec<Pane>, focus: usize, layout: LayoutPlan }`, where a
  `LayoutPlan` assigns rectangles to pane indices.
- *Pros:* Simplest model; focus is an index; adding/closing panes is O(1).
- *Cons:* Splits and proportional resize are not intrinsic — the `LayoutPlan` is
  a parallel structure that must stay consistent with the pane list, and
  nesting (split inside a split) has no natural home. Migration to a tree later
  is a format break.

**B. Split tree (tmux/Zellij-style).**
- *Shape:* `enum Node { Pane(Pane), Split { axis, ratio, children: [Box<Node>; 2] } }`;
  a tab holds a root `Node`.
- *Pros:* Layout, resize, and proportional splits fall out of the tree
  recursively; persistence serializes the tree directly; mirrors how mature
  multiplexers model it (see [zellij.md](../compatibility/zellij.md) pane
  operations row, which records that "which layer owns a split" is an explicit
  variable).
- *Cons:* More machinery up front; focus and traversal need a tree walk;
  re-balancing a degenerate tree is extra policy.

**PTY lifetime ownership.** `PtySession` already encapsulates the supervisor +
reader + child handle and is multi-instance-safe (`noren-pty/src/lib.rs:273-280`,
`Drop` shuts down idempotently at `lib.rs:420-424`). The natural choice is that
**each pane owns its own `PtySession`**; the alternative (a central PTY registry
keyed by pane id) is more machinery for no current benefit. This sub-decision is
lower-risk and reversible, but is called out because it determines how a child
failure is isolated (one pane dies, not the window — compare risk
[R-PTY-01](../roadmap/risk-register.md)).

**Resize propagation.** Window resize → layout resolves each pane's
`(rows, cols)` → each pane calls `TerminalState::resize`
(`state.rs:694-701`) then `PtySession::resize`. `GridGeometry` becomes
"window → layout rect per pane" rather than "window → one grid". Zero-size
panes (a tiny window or an extreme split) must be clamped to 1×1 and never send
zero dimensions to a PTY — the existing zero-coalescing rule
(`lib.rs:165-184`) must extend to the per-pane level.

**What is NOT decided here:** the tree shape (A vs B), and therefore the exact
serialization shape. The recommendation is to settle A-vs-B in an ADR gated on
this document before M3-1's data structs land, because M3-7's format follows
from it.

## 5. Zellij pass-through, concretely

Noren targets Zellij compatibility ([zellij.md](../compatibility/zellij.md)) and
Zellij is itself a multiplexer, so "pass-through" must mean a specific thing,
not "zero transformation." Two layers must be kept distinct (the
[Z-2L oracle](../compatibility/zellij.md)): the **outer** boundary
(Noren→Zellij PTY bytes) and the **inner** boundary (Zellij→child bytes). Per
[zellij.md](../compatibility/zellij.md) and the
[gap analysis](../compatibility/zellij-gap-analysis.md), concretely:

**Forward untouched (do not interpret) when a terminal pane is focused:**
- All printable UTF-8, Enter/Backspace/Tab/Escape, arrows, navigation and
  function keys, and their xterm modifier-parameter forms — i.e. today's
  `KeyEncoder` byte contract (`lib.rs:368-492`). The pass-through obligation is
  that these reach the focused PTY **unchanged**, the same bytes a direct-host
  control would send.
- Application-cursor/keypad mode selection driven by the **focused pane's**
  `TerminalModes` (`state.rs:274-283`), not a global. A focused Zellij pane sets
  DECCKM/DECKPAM; Noren must follow *that pane's* modes, not a single global.
- Bracketed-paste markers only when the focused pane enabled mode 2004
  (`main.rs:238-244`).

**Interpret (Noren consumes, does not forward) — the minimal accepted set:**
- Only the configured pass-through leader (entry/exit), and only the actions in
  the frozen interception manifest. Everything else forwards. This is the
  `noren_zellij_pass_through` contract in [zellij.md](../compatibility/zellij.md).

**How it interacts with Noren having its own panes.** Noren panes/tabs and
Zellij panes/tabs are **different layout layers**
([zellij.md](../compatibility/zellij.md) layout unknowns). A Noren split and a
Zellij split inside one Noren pane coexist: Noren routes bytes to the focused
*Noren* pane, and within it Zellij routes bytes to *its* focused pane. Noren must
not steal a chord that Zellij binds in Locked/normal mode
([zellij.md](../compatibility/zellij.md) Unlock-First row: bound secondary-modifier
chords are Zellij actions, not forwarding candidates — only **unbound** chords
forward). Pixel/coordinate work (mouse) differs across Noren chrome, Zellij
frames, and cell scaling, so coordinate-sensitive interaction is gated on the
mouse path below.

**Remaining blocking gap — mouse input (Issue #46).** Pointer-first Zellij
interaction (pane click/focus, drag-resize, wheel) is dead without a mouse input
encoder. Per [status](../coordination/status.md) and #46, this belongs in an
**input encoder in `noren-app`** (pointer events → mode-selected mouse bytes →
`send_input`), **not** output-side parsing — the withdrawn X10 `CSI M` "fix"
(status.md) confirmed that path is input-only. Mouse-mode *tracking* (DECSET
1000/1002/1003/1006/1015) lives in `noren-terminal` state, but the **encoder**
that turns pointer events into SGR-1006 reports belongs in `noren-app`. Until #46
lands, pass-through covers keyboard forwarding only; mouse-driven Zellij
workflows remain blocked and must not be implied as supported. This is tracked as
#46, not re-scoped into M3-4.

**Gap-analysis staleness note.** The [gap analysis](../compatibility/zellij-gap-analysis.md)
is dated `b3391cc`; since then search (PR #65), truecolor SGR (PR #56), and
clipboard/paste (PR #66) landed. Its paste (row 5) and color (row 4) "Noren
today" cells are stale; its input-side rows (mouse row 2, focus row 8, KKP/legacy
rows) remain accurate as open gaps.

## Non-claims

This document executes nothing in `crates/*/src/`, adds no tests, and advances no
matrix row. It selects no architecture: the Section 4 choice is explicitly left
for owner approval. No milestone date is promised. Zellij is named nominatively
as the compatibility target; no Zellij code or assets were copied here (legal
boundary in [zellij.md](../compatibility/zellij.md)).
