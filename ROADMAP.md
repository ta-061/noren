# Roadmap

Status terms: **Not started**, **In progress**, **Gate review**, **Complete**.
Only evidence-backed work is marked complete.

| Milestone | Scope | Status |
| --- | --- | --- |
| 0 — Discovery | Landscape, feature/library matrices, risks, agent inventory and calibration | Complete |
| 1 — Requirements and design | Independent proposals, critiques, integrated requirements, architecture, threat model, tests, RFCs, ADRs | Complete |
| 2 — Terminal foundation | Window, PTY, shell, terminal state/rendering, input, resize, scrollback, selection, copy/paste/search, configuration and diagnostics | Complete |
| 3 — Workspace | External workspace management (sidebar: projects, git worktrees, SSH connections, agents, terminal sessions), single-session view, session lifecycle, sidebar-state persistence, palette, configurable keybindings, Zellij pass-through — no native tabs/panes/layout (delegated to Zellij per [ADR 0003](docs/adr/0003-noren-zellij-responsibility-boundary.md)) | In progress — vertical slice landed; see [Milestone 3 status](#milestone-3-status) |
| 4 — SSH and remote | OpenSSH configuration, connections, reconnect, remote panes, daemon decision/PoC and recovery | In progress — bounded OpenSSH configuration discovery and sidebar target selection landed; no connection or remote PTY |
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

**What this does not establish.** A rendered-frame oracle now exists
(`crates/noren-app/tests/frame_oracle.rs`, `crates/noren-app/src/renderer_capture.rs`,
PR #89): it drives the real `wgpu` pipeline offscreen and checks *structural*
properties — blank cells are dark, distinct glyphs have distinct lit patterns,
glyphs do not bleed into neighbouring cells, and the drawn grid agrees with the
terminal-state snapshot. It does **not** verify that an `A` is shaped like an A,
and two of its tests are `#[ignore]`d because they document real font defects
(case-folding in the bitmap font, and every non-ASCII code point falling through
to the `?` glyph). Key injection into the real window still does not exist, so
live keyboard input remains unverified by automation; the byte-level input
contract is covered by tests instead. Mouse reporting is no longer encoder-only:
`MouseEncoder::encode` (`crates/noren-app/src/mouse.rs`) is now reached from the
pointer and wheel handlers in `main.rs` through their shared
`encode_and_send_mouse` helper, which writes the report bytes to the PTY.
Truecolor is modelled in terminal state but not yet wired to drawing. IME
and accessibility remain deferred.

No milestone date is promised. Implementation advances through scoped Issues,
Draft PRs, and current-head CI evidence.

## Milestone 3 status

**In progress, not Complete.** The vertical slice reached the binary: launching
the build now draws a workspace sidebar, and sessions can be created, selected,
and closed from a command palette.

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
| — projects, git worktrees | Modelled, not launchable | `EntryKind::Project`/`Worktree`, `SessionKind::Project`/`Worktree` exist; no creation path constructs them |
| — SSH connections, agents | Configured targets listed, not connected; agents fixture only | Bounded OpenSSH facts become `SessionKind::Ssh` and `SidebarEntry::SshConnection` rows; clicking records a pending target but opens no SSH connection or PTY; agent entries remain reserved |
| — terminal sessions | Done | `SessionKind::Local` is created, selected, and closed from the running binary |
| Single-session view | Done | Terminal drawn beside the sidebar, narrowed to the remaining columns |
| Session lifecycle | Done | `SessionStatus` advances `Starting -> Running -> Exited/Failed` via `SessionRegistry::observe`, wired in `main.rs` |
| Sidebar-state persistence | Done | `sessions.toml` (`SESSION_STATE_FILE_NAME`) under the `config::default_path` directory, resolved by `session_state_path` |
| Palette | Done | `Super+p` via `palette_policy`; `Palette::noren`'s four commands dispatched by `handle_palette_key` |
| Configurable keybindings | **Not started** | `palette_policy` and `handle_palette_key` hard-code the chords; `config.rs` exposes no keymap surface |
| Zellij pass-through | Done against a pinned corpus | `passthrough.rs` policy claims only Super-space chords that the pinned Zellij `v0.44.3` default corpus (`ZELLIJ_FIXTURE_TAG`) never binds; no test drives a live Zellij |

Two named scope items remain unsatisfied: **configurable keybindings do not
exist at all**, and **SSH connections and agents do not run**. Configured SSH
targets now appear in the sidebar and a click records a pending target, but
opens no SSH connection or PTY; agent entries remain fixtures and launch no
agent. Since "Only evidence-backed work is marked complete" and the scope line
names both, Milestone 3 stays **In progress**. The SSH and agent session kinds
also depend on Milestones 4 and 5; whether they are retired from Milestone 3's
scope or carried is an open scoping decision, not something to settle by
relabelling the status.

## What blocks a public preview

Two independent specification reviews, run without sight of each other, both
concluded that the current tree cannot honestly be released as "0.1.0-preview of
the Noren terminal." The reasoning and the decision are recorded in
[D-M8-001](docs/coordination/decisions/D-M8-001-preview-scope.md). In short:

- **The workspace is a slice, not a product.** The Milestone 3 modules now
  reach the binary: the sidebar is drawn, the palette opens on `Super+p`,
  sessions are created/selected/closed, mouse reports reach the PTY, and
  sidebar state persists across a restart. What is still missing is breadth —
  bounded OpenSSH configuration now produces `SessionKind::Ssh` values and
  `SidebarEntry::SshConnection` rows, but selecting one only records a pending
  target and opens no connection or PTY. Only `SessionKind::Local` reaches a
  launch path; git worktrees remain unreachable, agents remain fixture-only,
  and keybindings are not configurable. See [Milestone 3 status](#milestone-3-status).
- **The renderer is monochrome.** The fragment shader `fs_main` in
  `renderer.rs` returns a constant colour and the vertex layout carries no
  colour channel, so `ls --color`, `vim`, and Zellij's status bar all draw in
  one shade. Truecolor is modelled in terminal state and never reaches drawing.
- **The font is ASCII-only and case-blind.** Non-ASCII renders as `?`, and the
  `renderer.rs` test `ascii_glyphs_are_distinct_and_unknown_is_question_mark`
  asserts `glyph_rows('a') == glyph_rows('A')`.
- **The FR-005 rendered-frame oracle now exists** (PR #89). It drives the real
  pipeline offscreen, and its `#[ignore]`d defect tests record the font's
  case-fold and non-ASCII-`?` failures — the same defects above.
- **NFR-009 requires release-integrity gates** — signing, notarization,
  packaging — to pass before any Preview claim.

Milestone 8 therefore stops at a release candidate. Signing keys, Apple
certificates, tagging, and publication are owner decisions and are not taken
autonomously.
