# Roadmap

Status terms: **Not started**, **In progress**, **Gate review**, **Complete**.
Only evidence-backed work is marked complete.

| Milestone | Scope | Status |
| --- | --- | --- |
| 0 — Discovery | Landscape, feature/library matrices, risks, agent inventory and calibration | Complete |
| 1 — Requirements and design | Independent proposals, critiques, integrated requirements, architecture, threat model, tests, RFCs, ADRs | Complete |
| 2 — Terminal foundation | Window, PTY, shell, terminal state/rendering, input, resize, scrollback, selection, copy/paste/search, configuration and diagnostics | Complete |
| 3 — Workspace | External workspace management (sidebar: projects, git worktrees, SSH connections, agents, terminal sessions), single-session view, session lifecycle, sidebar-state persistence, palette, configurable keybindings, Zellij pass-through — no native tabs/panes/layout (delegated to Zellij per [ADR 0003](docs/adr/0003-noren-zellij-responsibility-boundary.md)) | In progress — vertical slice landed; see [Milestone 3 status](#milestone-3-status) |
| 4 — SSH and remote | OpenSSH configuration, connections, reconnect, remote panes, daemon decision/PoC and recovery | In progress — bounded, explicitly partial literal-alias discovery landed, and selecting an alias launches the fixed system `ssh` client in the terminal's PTY (PR #138); reconnect, remote panes, the daemon decision/PoC, and recovery are not started |
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
defaults. It does **not** verify that an `A` is shaped like an A. Its two former
defect tests (`lowercase_distinct_from_uppercase`,
`non_ascii_glyph_is_not_the_question_mark`) are no longer `#[ignore]`d: PR #141
retired both font defects — upper/lower case are now distinct glyphs, the
Latin-1 Supplement and Box Drawing blocks have per-character bitmaps, and every
other code point draws a visible replacement glyph instead of `?` — and the
tests now guard those fixes. Key injection into the real window still does not
exist, so live
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
| — projects, git worktrees | Worktrees and projects launchable | Startup runs `git worktree list --porcelain` in the launch directory (`git_worktree.rs`) and shows at most 24 discovered worktrees as `EntryKind::Worktree` rows (beyond the cap the omitted count is reported); a registered-but-deleted worktree is listed with a `(missing)` marker and refused on selection; selecting a present row creates a `SessionKind::Worktree` session backed by a real `/bin/zsh` PTY whose child's working directory IS the worktree (verified by reading the child's own `pwd` back through the terminal), persisting and restoring through `sessions.toml` like a local session. `[[projects]]` configuration entries (`config.rs`, strict schema: absolute `root`, typed errors naming the offending key, no value echo) appear as at most 24 `EntryKind::Project` rows — the fixed `PRJ-` state prefix distinguishes them from prefix-less worktree rows — and selecting one whose root exists creates a `SessionKind::Project` session with the same directory-rooted launch shape and `pwd` proof; a configured-but-gone root is refused visibly like a deleted worktree. Project sessions persist and restore through `sessions.toml` like every other kind |
| — SSH connections, agents | Connections run for discovered aliases; discovery explicitly partial; agents launchable from configuration | At most 24 positive literal OpenSSH aliases become `SessionKind::Ssh` and `SidebarEntry::SshConnection` rows; the status identifies partial discovery, and clicking one launches the fixed system `/usr/bin/ssh` client in the terminal's single PTY (argv is exactly `ssh -- <alias>`; no credential is ever argv-visible), with launch, connect, and disconnect failures as visible per-row and status-row states (PR #138). Wildcard/dynamic destinations are not a complete host inventory. `[[agents]]` entries in `config.toml` become at most 24 `EntryKind::Agent` rows (beyond the cap the omitted count is reported); selecting one creates a `SessionKind::Agent` session backed by a real PTY running the configured command as a shell-free argv vector (absolute path, no `PATH` lookup; a missing or non-executable command is a visible per-row and status-row failure), persisting and restoring through `sessions.toml` like every other session kind |
| — terminal sessions | Runtime for local sessions | Every palette `session_create` spawns a real `SessionKind::Local` PTY (`spawn_local_session`); sidebar clicks and the palette's `session_select` switch the live view between live sessions (`switch_live_session`, parked surfaces keep draining and resizing), and `session_close` reaps the closed child and repairs the view. Rows restored from disk come back `Restored` with no live surface and cannot take the live view |
| Single-session view | Done | The active terminal is drawn beside the sidebar, narrowed to the remaining columns; switching swaps the active surface whole, and rows without a live surface cannot claim its selection or input owner |
| Session lifecycle | Done | `SessionStatus` advances `Starting -> Running -> Exited/Failed` via `SessionRegistry::observe`, wired in `main.rs` for spawned, parked, closed, and restored sessions alike |
| Sidebar-state persistence | Done | `sessions.toml` (`SESSION_STATE_FILE_NAME`) under the `config::default_path` directory, resolved by `session_state_path` |
| Palette | Done | `Super+p` via `palette_policy`; `Palette::noren`'s four commands dispatched by `handle_palette_key` |
| Configurable keybindings | Done for the palette surface | `[keys]` in `config.toml` (`KeymapConfig` in `config.rs`) rebinds the palette opener and the four palette command chords with the previous values as defaults; `palette_policy`/`handle_palette_key` in `main.rs` honor them, unparseable chords and unknown actions are typed errors, and the opener is validated against the pinned Zellij corpus and the exit leader. The exit leader, palette navigation keys, diagnostics chord, and clipboard shortcuts remain fixed |
| Zellij pass-through | Done against a pinned corpus and a live installed Zellij | The shipped policy (`palette_policy` in `main.rs`) claims exactly two Super-modified chords — `Super+Escape` (exit leader) and `Super+p` (palette opener) — that the pinned Zellij `v0.44.3` default corpus (`ZELLIJ_FIXTURE_TAG`) never binds, and `tests/zellij_live.rs` drives an INSTALLED Zellij in a real PTY through the same parser, gate, and key encoder: attach enables mouse tracking in `TerminalState`, gated `Ctrl+t`/`n`/`Ctrl+p` reach Zellij and render tab #2 and pane #2, typed text reaches the pane's shell, and the installed version's default keybinds bind nothing in the Super/Cmd/Meta space. The harness skips (visibly, on the real stderr) when no `zellij` is on `PATH`. Empirical wire-shape note: Zellij 0.44.3 sends `1002`/`1006` as separate single-parameter DECSETs across its whole lifecycle and does not forward a pane program's multi-parameter DECSET to the host terminal, so the multi-parameter form `CSI ? 1002;1006 h` (the PR #113 regression site) is pinned as a co-located regression guard beside the live assertions, with the live multi-parameter count printed as drift telemetry. Beyond the skip, the suite today runs only where a developer runs it — no gating machine executes it (issue #153) |

One named scope item remains unsatisfied: **host discovery is explicitly
partial**. Projects are reachable now: a `[[projects]]` configuration entry
constructs an `EntryKind::Project` row and selecting it creates a
`SessionKind::Project` session backed by a real PTY whose child starts in
the configured root. Positive literal aliases appear in a bounded sidebar
list and a click launches a real system-ssh connection (PR #138), but
wildcard or dynamic destinations are not presented as a complete host
inventory. Agent entries are no longer fixtures: `[[agents]]`
configuration rows launch a configured, shell-free argv command in a real
PTY (the first Milestone 5 slice); the remaining agent-experience scope
(verified adapters, trustworthy state, notifications, jump-to-source) is
Milestone 5 work. Configurable keybindings are satisfied for the palette
surface only; the exit leader, palette navigation, diagnostics chord, and
clipboard shortcuts remain compiled in. Since "Only evidence-backed work
is marked complete" and the scope line names the unsatisfied item,
Milestone 3 stays **In progress** on host discovery.

Open engineering issues a reader of this section should know about: the
behavior-preserving split of the oversized binary and SSH-parser modules
([#123](https://github.com/ta-061/noren/issues/123)) is still open — the
binary test module and the SSH-parser tests have been extracted, and the
remaining production-side splits are tracked there; the live-Zellij
pass-through suite runs on no machine that gates a merge
([#153](https://github.com/ta-061/noren/issues/153)) — its evidence is
gathered only where a developer happens to run it; and the PTY-level
`spawn_in_dir_runs_the_child_in_that_directory` test cannot distinguish an
honoured working directory from portable-pty's HOME fallback
([#162](https://github.com/ta-061/noren/issues/162)), so the app-level `pwd`
proof cited in the worktree row above is the real guarantee that a worktree
session's child starts in the worktree.

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
  `SidebarEntry::SshConnection` rows, and selecting one launches the fixed
  system `ssh` client in the terminal's PTY. Git
  worktrees of the launch repository ARE reachable now (discovered from
  `git worktree list --porcelain`, shown as bounded rows, and launched as
  worktree-scoped sessions), configured agents are launchable through the
  `[[agents]]` section (a shell-free argv PTY launch with visible failure
  states), and configured projects are launchable through the
  `[[projects]]` section (a directory-rooted PTY launch in the configured
  root, with the same visible-failure discipline); keybindings ARE
  configurable through the `[keys]` section since this milestone (see
  [Milestone 3 status](#milestone-3-status)).
- **Colour rendering exists, but themes are fixed.** `renderer.rs` resolves
  each cell's SGR foreground and any explicit background through its compiled-in
  ANSI/256-colour palette or as direct RGB truecolor. The vertex layout carries
  the resolved colour alongside position and `fs_main` returns that per-vertex
  input. There is no configuration surface for the default palette or theme,
  so light, dark, high-contrast, and colour-vision-friendly themes remain
  Milestone 6 work.
- **The font is a hand-built 5x7 bitmap with bounded coverage.** Printable
  ASCII keeps distinct upper/lower case, and the Latin-1 Supplement
  (`U+00A0..=U+00FF`) and Box Drawing (`U+2500..=U+257F`) blocks have
  per-character bitmaps, but every other code point — CJK text and emoji
  included — draws a fixed replacement glyph, and seven glyph pairs are
  visually identical by an allowlisted collision set (pinned by
  `covered_range_glyph_collisions_match_the_hardcoded_allowlist` in
  `renderer.rs`'s tests). Both former font defects — case-folding and
  non-ASCII rendering as `?` — were retired by PR #141 and are now guarded by
  running tests, not `#[ignore]`d ones.
- **The FR-005 rendered-frame oracle now exists** (PR #89). It drives the real
  pipeline offscreen; active colour-aware assertions cover SGR foregrounds,
  ANSI/256-colour and direct RGB resolution, defaults, and explicit backgrounds.
  Its former defect tests (`lowercase_distinct_from_uppercase`,
  `non_ascii_glyph_is_not_the_question_mark`) now run and pass, guarding the
  PR #141 font fixes — see the font bullet above for what the font still cannot
  do.
- **NFR-009 requires release-integrity gates** — signing, notarization,
  packaging — to pass before any Preview claim.

Milestone 8 therefore stops at a release candidate. Signing keys, Apple
certificates, tagging, and publication are owner decisions and are not taken
autonomously.
