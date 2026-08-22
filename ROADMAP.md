# Roadmap

Status terms: **Not started**, **In progress**, **Gate review**, **Complete**.
Only evidence-backed work is marked complete.

| Milestone | Scope | Status |
| --- | --- | --- |
| 0 — Discovery | Landscape, feature/library matrices, risks, agent inventory and calibration | Complete |
| 1 — Requirements and design | Independent proposals, critiques, integrated requirements, architecture, threat model, tests, RFCs, ADRs | Complete |
| 2 — Terminal foundation | Window, PTY, shell, terminal state/rendering, input, resize, scrollback, selection, copy/paste/search, configuration and diagnostics | Complete |
| 3 — Workspace | External workspace management (sidebar: projects, git worktrees, SSH connections, agents, terminal sessions), single-session view, session lifecycle, sidebar-state persistence, palette, configurable keybindings, Zellij pass-through — no native tabs/panes/layout (delegated to Zellij per [ADR 0003](docs/adr/0003-noren-zellij-responsibility-boundary.md)) | In progress — vertical slice landed; see [Milestone 3 status](#milestone-3-status) |
| 4 — SSH and remote | OpenSSH configuration, connections, reconnect, remote panes, daemon decision/PoC and recovery | In progress — bounded, explicitly partial literal-alias discovery and source-attributed sidebar selection landed; no connection or remote PTY |
| 5 — Agent experience | Launchers, verified adapters, trustworthy state, notifications and jump-to-source | Not started |
| 6 — Themes and accessibility | Light/dark/high-contrast palettes, contrast checks, IME/CJK/HiDPI and keyboard/accessibility work | Not started |
| 7 — Quality | Unit/integration/compatibility/fault/security/visual tests, fuzzing, soak tests and benchmarks | Not started |
| 8 — Public Preview | Honest docs/site, binaries, checksums, release review, known limitations and `0.1.0-preview` | Not started; scope decided by [D-M8-001](docs/coordination/decisions/D-M8-001-preview-scope.md) |

A renderer-independent terminal state core is merged as PR
[#19](https://github.com/ta-061/noren/pull/19) (`c695920`), described in
[terminal core foundation](docs/architecture/terminal-core-foundation.md).

The parallel Terminal Core stack is merged as PR
[#29](https://github.com/ta-061/noren/pull/29) (`22c985e`): scrolling regions
and margins, alternate screen with DEC private mode 1049, erase/insert/delete
operations, SGR and cell attributes, and application cursor/keypad modes wired
into the key encoder. PR [#32](https://github.com/ta-061/noren/pull/32)
(`aa41530`) adds a bounded VT compatibility harness. Escape-intermediate
sequences and horizontal tab are handled.

Those foundation PRs did not by themselves establish VT100/xterm or
vim/tmux/zellij compatibility. Their tracked follow-ups were subsequently
closed: PR [#38](https://github.com/ta-061/noren/pull/38) aligned renderer and
PTY grids (Issue #35), PRs [#40](https://github.com/ta-061/noren/pull/40),
[#48](https://github.com/ta-061/noren/pull/48), and
[#60](https://github.com/ta-061/noren/pull/60) completed key encoding (Issue
#36), and PR [#45](https://github.com/ta-061/noren/pull/45) fixed DECSTBM
clamping and embedded C0 handling (Issue #37). PR
[#53](https://github.com/ta-061/noren/pull/53) added Unicode/CJK display width;
IME, origin mode, and query/reply remain deferred. The completion evidence and
remaining limits below supersede that earlier foundation-only snapshot.

## Milestone 2 completion evidence

Closed at `1d329a5` with **353 workspace tests** passing, plus `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`, MSRV
verification, and the documentation validator — all four CI checks required by branch
protection.

Delivered across the milestone: window and local zsh PTY, the renderer-independent
terminal state core, scroll regions, alternate screen and mode state, erase/insert/
delete, SGR with 256-color and truecolor including colon sub-parameter forms, escape
intermediates and horizontal tab, DCS/SOS/PM/APC string sequences, DECSTBM clamping,
C0 inside CSI, application cursor and keypad modes, key encoding through xterm
modifier parameters, Unicode/CJK display width, bounded scrollback, grid selection
with clipboard copy/paste, scrollback search, and optional configuration with
diagnostics.

Quality evidence: a bounded VT compatibility harness, two independent adversarial
hostile-input suites, and a per-cell grapheme cap that made the documented memory
ceiling true rather than aspirational.

Manual macOS gate, re-run at this head: a release build opened a window, owned a
direct `zsh` child, and that child's tty reported `30 90` — the 900x600 window divided
by the 10x20 cell, so the window to grid to PTY chain agrees. On termination the app
exited, the child was reaped, and the pty device was gone.

**What the Milestone 2 close did not itself establish.** A rendered-frame oracle
was added later (`crates/noren-app/tests/frame_oracle.rs`,
`crates/noren-app/src/renderer_capture.rs`, PR #89). The current oracle drives
the real `wgpu` pipeline offscreen and checks *structural* properties — blank
cells are dark, distinct glyphs have distinct lit patterns, glyphs do not bleed
into neighbouring cells, and the drawn grid agrees with the terminal-state
snapshot — plus colour behaviour: distinct SGR foregrounds, fixed-palette ANSI
and 256-colour values, direct RGB truecolor, explicit backgrounds, and unchanged
defaults. It does **not** verify that an `A` is shaped like an A, and two of its
tests are `#[ignore]`d because they document real font defects (case-folding in
the bitmap font, and every non-ASCII code point falling through to the `?`
glyph). Key injection into the real window still does not exist, so live
keyboard input remains unverified by automation; the byte-level input contract
is covered by tests instead. Mouse reporting is no longer encoder-only:
`MouseEncoder::encode` (`crates/noren-app/src/mouse.rs`) is now reached from the
pointer and wheel handlers in `main.rs` through their shared
`encode_and_send_mouse` helper, which writes the report bytes to the PTY.
Colour drawing also landed after the close: `renderer.rs` resolves each cell's
SGR foreground and any explicit background through the fixed ANSI/256-colour
palette or as direct RGB, then carries that result to the shader as per-vertex
colour. The default palette and theme are fixed and not user-configurable. IME
and accessibility remain deferred.

No milestone date is promised. Implementation advances through scoped Issues,
Draft PRs, and current-head CI evidence.

## Milestone 3 status

**In progress, not Complete.** The vertical slice reached the binary: launching
the build now draws a workspace sidebar and starts one local PTY. Session
switching is real for local sessions: palette-created rows spawn their own
PTY, sidebar clicks and `session_select` move the live view between them, and
`session_close` reaps the closed child.

The milestone's own scope line is the test, so it is quoted here in full:

> External workspace management (sidebar: projects, git worktrees, SSH
> connections, agents, terminal sessions), single-session view, session
> lifecycle, sidebar-state persistence, palette, configurable keybindings,
> Zellij pass-through — no native tabs/panes/layout (delegated to Zellij per
> ADR 0003)

Measured against it, item by item:

| Scope item | State | Evidence |
| --- | --- | --- |
| Sidebar drawn | Done | `SIDEBAR_COLS` reserved in `renderer.rs`; `glyph_vertices` applies the column offset; `sidebar_text_lines` formats rows |
| — projects, git worktrees | Modelled, not launchable | `EntryKind::Project`/`Worktree`, `SessionKind::Project`/`Worktree` exist; no runtime path creates them, though `parse_session` will reconstruct one from a hand-written `sessions.toml` `kind` field — nothing in the product writes such an entry |
| — SSH connections, agents | Partial configured-target list, not connected; agents fixture only | At most 24 positive literal OpenSSH aliases become `SessionKind::Ssh` and `SidebarEntry::SshConnection` rows; the status identifies partial discovery, and clicking shows bounded root-relative source provenance while opening no SSH connection or PTY; agent entries remain reserved |
| — terminal sessions | Runtime for local sessions | Every palette `session_create` spawns a real `SessionKind::Local` PTY (`spawn_local_session`); sidebar clicks and the palette's `session_select` switch the live view between live sessions (`switch_live_session`, parked surfaces keep draining and resizing), and `session_close` reaps the closed child and repairs the view. Rows restored from disk come back `Restored` with no live surface and cannot take the live view |
| Single-session view | Done | The active terminal is drawn beside the sidebar, narrowed to the remaining columns; switching swaps the active surface whole, and rows without a live surface cannot claim its selection or input owner |
| Session lifecycle | Done | `SessionStatus` advances `Starting -> Running -> Exited/Failed` via `SessionRegistry::observe`, wired in `main.rs` for spawned, parked, closed, and restored sessions alike |
| Sidebar-state persistence | Done | `sessions.toml` (`SESSION_STATE_FILE_NAME`) under the `config::default_path` directory, resolved by `session_state_path` |
| Palette | Done | `Super+p` via `palette_policy`; `Palette::noren`'s four commands dispatched by `handle_palette_key` |
| Configurable keybindings | Done for the palette surface | `[keys]` in `config.toml` (`KeymapConfig` in `config.rs`) rebinds the palette opener and the four palette command chords with the previous values as defaults; `palette_policy`/`handle_palette_key` in `main.rs` honor them, unparseable chords and unknown actions are typed errors, and the opener is validated against the pinned Zellij corpus and the exit leader. The exit leader, palette navigation keys, diagnostics chord, and clipboard shortcuts remain fixed |
| Zellij pass-through | Done against a pinned corpus | The shipped policy (`palette_policy` in `main.rs`) claims exactly two Super-modified chords — `Super+Escape` (exit leader) and `Super+p` (palette opener) — that the pinned Zellij `v0.44.3` default corpus (`ZELLIJ_FIXTURE_TAG`) never binds; no test drives a live Zellij |

One named scope item remains unsatisfied: **SSH connections and agents do
not run**. Positive literal aliases now appear in a bounded sidebar list and
a click records a source-attributed pending target, but opens no SSH
connection or PTY; wildcard or dynamic destinations are not presented as a
complete host inventory, and agent entries remain fixtures and launch no
agent. Since "Only evidence-backed work is marked complete" and the scope
line names it, Milestone 3 stays **In progress**. The SSH and agent session
kinds also depend on Milestones 4 and 5; whether they are retired from
Milestone 3's scope or carried is an open scoping decision, not something to
settle by relabelling the status.

## What blocks a public preview

Two independent specification reviews, run without sight of each other, both
concluded that the current tree cannot honestly be released as "0.1.0-preview of
the Noren terminal." The reasoning and the decision are recorded in
[D-M8-001](docs/coordination/decisions/D-M8-001-preview-scope.md). In short:

- **The workspace is a slice, not a product.** The Milestone 3 modules now
  reach the binary: the sidebar is drawn, the palette opens on `Super+p`,
  local sessions spawn real PTYs that switch, park, and close through the
  live view, mouse reports reach the active PTY, and sidebar state persists
  across a restart. What is still missing is breadth —
  bounded OpenSSH configuration now produces an explicitly partial list of at
  most 24 positive literal aliases as `SessionKind::Ssh` values and
  `SidebarEntry::SshConnection` rows. The UI labels the discovery scope and
  shows bounded root-relative source provenance on selection, but selecting one
  only records a pending target and opens no connection or PTY. Only
  `SessionKind::Local` reaches a
  launch path; git worktrees remain unreachable, and agents remain
  fixture-only; keybindings ARE configurable through the `[keys]` section
  since this milestone (see [Milestone 3 status](#milestone-3-status)).
- **Colour rendering exists, but themes are fixed.** `renderer.rs` resolves
  each cell's SGR foreground and any explicit background through its compiled-in
  ANSI/256-colour palette or as direct RGB truecolor. The vertex layout carries
  the resolved colour alongside position and `fs_main` returns that per-vertex
  input. There is no configuration surface for the default palette or theme,
  so light, dark, high-contrast, and colour-vision-friendly themes remain
  Milestone 6 work.
- **The font is ASCII-only and case-blind.** Non-ASCII renders as `?`, and the
  `renderer.rs` test `ascii_glyphs_are_distinct_and_unknown_is_question_mark`
  asserts `glyph_rows('a') == glyph_rows('A')`.
- **The FR-005 rendered-frame oracle now exists** (PR #89). It drives the real
  pipeline offscreen; active colour-aware assertions cover SGR foregrounds,
  ANSI/256-colour and direct RGB resolution, defaults, and explicit backgrounds.
  Its `#[ignore]`d defect tests still record the font's case-fold and
  non-ASCII-`?` failures — the same defects above.
- **NFR-009 requires release-integrity gates** — signing, notarization,
  packaging — to pass before any Preview claim.

Milestone 8 therefore stops at a release candidate. Signing keys, Apple
certificates, tagging, and publication are owner decisions and are not taken
autonomously.
