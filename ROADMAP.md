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
| 7 — Quality | Unit/integration/compatibility/fault/security/visual tests, fuzzing, soak tests and benchmarks | In progress — the benchmark slice landed (PR #171): a criterion suite over the paths with a defect history (`feed_bytes`, `ssh_config_parse` incl. the #137 shape, renderer frame prep, per-frame `snapshot`, `search`) behind a `bench-support` feature gate with a recorded reference-machine baseline and a report-never-gate policy ([docs/testing/benchmarks.md](docs/testing/benchmarks.md)); its first finding (46.3 ms per-frame `snapshot` at full scrollback) was filed separately as #172 rather than changed in the measuring PR. Fuzzing, soak, and visual suites remain not started |
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
colour. The palette is no longer fixed: `[theme]` in `config.toml` now selects
one of three built-in themes — `dark` (the default, byte-identical to the
single table this close shipped), `light`, and `high-contrast` — with measured,
test-pinned WCAG contrast floors (`theme.rs`; see
[What blocks a public preview](#what-blocks-a-public-preview) for the default
`dark` palette's open AA failure). IME and accessibility remain deferred.

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
| — SSH connections, agents | Connections run for discovered aliases; discovery explicitly partial with every absence explained; agents launchable from configuration | At most 64 positive literal OpenSSH aliases become `SessionKind::Ssh` and `SidebarEntry::SshConnection` rows; the status identifies partial discovery, and past the cap it reports how many are shown of how many and why (`showing first 64 of 70; 6 past sidebar bound`). Wildcard `Host` patterns never become rows — a pattern is a rule, not a connectable destination — but they are counted per occurrence (including through followed `Include` files) and the notice reports `N wildcard patterns not listed`, so a pattern-built config is never silently under-represented (issue #175). Negations are filters: they suppress matches and add no row and no absence count. Clicking a row launches the fixed system `/usr/bin/ssh` client in the terminal's single PTY (argv is exactly `ssh -- <alias>`; no credential is ever argv-visible), with launch, connect, and disconnect failures as visible per-row and status-row states (PR #138). Wildcard/dynamic destinations still cannot be enumerated as a complete host inventory; that boundary is now stated with counts rather than silence. `[[agents]]` entries in `config.toml` become at most 24 `EntryKind::Agent` rows (beyond the cap the omitted count is reported); selecting one creates a `SessionKind::Agent` session backed by a real PTY running the configured command as a shell-free argv vector (absolute path, no `PATH` lookup; a missing or non-executable command is a visible per-row and status-row failure), persisting and restoring through `sessions.toml` like every other session kind |
| — terminal sessions | Runtime for local sessions | Every palette `session_create` spawns a real `SessionKind::Local` PTY (`spawn_local_session`); sidebar clicks and the palette's `session_select` switch the live view between live sessions (`switch_live_session`, parked surfaces keep draining and resizing), and `session_close` reaps the closed child and repairs the view. Rows restored from disk come back `Restored` with no live surface and cannot take the live view |
| Single-session view | Done | The active terminal is drawn beside the sidebar, narrowed to the remaining columns; switching swaps the active surface whole, and rows without a live surface cannot claim its selection or input owner |
| Session lifecycle | Done | `SessionStatus` advances `Starting -> Running -> Exited/Failed` via `SessionRegistry::observe`, wired in `main.rs` for spawned, parked, closed, and restored sessions alike |
| Sidebar-state persistence | Done | `sessions.toml` (`SESSION_STATE_FILE_NAME`) under the `config::default_path` directory, resolved by `session_state_path` |
| Palette | Done | `Super+p` via `palette_policy`; `Palette::noren`'s four commands dispatched by `handle_palette_key` |
| Configurable keybindings | Done for the palette surface | `[keys]` in `config.toml` (`KeymapConfig` in `config.rs`) rebinds the palette opener and the four palette command chords with the previous values as defaults; `palette_policy`/`handle_palette_key` in `main.rs` honor them, unparseable chords and unknown actions are typed errors, and the opener is validated against the pinned Zellij corpus and the exit leader. The exit leader, palette navigation keys, diagnostics chord, and clipboard shortcuts remain fixed |
| Zellij pass-through | Done against a pinned corpus and a live installed Zellij | The shipped policy (`palette_policy` in `main.rs`) claims exactly two Super-modified chords — `Super+Escape` (exit leader) and `Super+p` (palette opener) — that the pinned Zellij `v0.44.3` default corpus (`ZELLIJ_FIXTURE_TAG`) never binds, and `tests/zellij_live.rs` drives an INSTALLED Zellij in a real PTY through the same parser, gate, and key encoder: attach enables mouse tracking in `TerminalState`, gated `Ctrl+t`/`n`/`Ctrl+p` reach Zellij and render tab #2 and pane #2, typed text reaches the pane's shell, and the installed version's default keybinds bind nothing in the Super/Cmd/Meta space. The harness skips (visibly, on the real stderr) when no `zellij` is on `PATH`. Empirical wire-shape note: Zellij 0.44.3 sends `1002`/`1006` as separate single-parameter DECSETs across its whole lifecycle and does not forward a pane program's multi-parameter DECSET to the host terminal, so the multi-parameter form `CSI ? 1002;1006 h` (the PR #113 regression site) is pinned as a co-located regression guard beside the live assertions, with the live multi-parameter count printed as drift telemetry. Beyond the skip, the suite today runs only where a developer runs it — no gating machine executes it (issue #153) |

The host-discovery gap that issue
[#175](https://github.com/ta-061/noren/issues/175) named is closed: positive
literal aliases appear in a bounded sidebar list (now 64 rows) and a click
launches a real system-ssh connection (PR #138); wildcard and negation
patterns are resolved with OpenSSH semantics and, because a pattern is a rule
rather than a connectable destination, are counted and explained in the
status notice instead of vanishing; `Include` files feed the same block list
to discovery and resolution, so an alias declared only in an included file is
discovered like a top-level one; and past the sidebar cap the notice reports
how many are shown of how many and why. Discovery is still *explicitly
partial* by design — no destination enumeration happens here — but no absence
is silent. Projects are reachable now: a `[[projects]]` configuration entry
constructs an `EntryKind::Project` row and selecting it creates a
`SessionKind::Project` session backed by a real PTY whose child starts in
the configured root. Agent entries are no longer fixtures: `[[agents]]`
configuration rows launch a configured, shell-free argv command in a real
PTY (the first Milestone 5 slice); the remaining agent-experience scope
(verified adapters, trustworthy state, notifications, jump-to-source) is
Milestone 5 work. Milestone 3 nonetheless stays **In progress**:
configurable keybindings are satisfied for the palette
surface only — the exit leader, palette navigation, diagnostics chord, and
clipboard shortcuts remain compiled in — and the live-Zellij pass-through
evidence still gates no machine
([#153](https://github.com/ta-061/noren/issues/153)). Since "Only
evidence-backed work is marked complete", those items, not host discovery,
now hold the milestone open.

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
[D-M8-001](docs/coordination/decisions/D-M8-001-preview-scope.md). Much of what
those reviews named has since been fixed — the workspace slice reaches the
binary, colour reaches the pixels, and both font defects are retired — so this
section states what blocks a preview **at this tree**, each item verified
against it:

- **The workspace is real but single-viewport.** The sidebar is drawn, the
  palette opens on `Super+p`, local sessions spawn real PTYs that switch,
  park, and close through the live view, mouse reports reach the active PTY,
  and sidebar state persists across a restart. Git worktrees of the launch
  repository are discovered (`git worktree list --porcelain`, driven from
  `git_worktree.rs`) and launched as sessions whose child's working directory
  IS the worktree, proven by reading the child's own `pwd` back through the
  terminal. `[[projects]]` entries launch directory-rooted sessions,
  `[[agents]]` entries launch their configured command as a shell-free argv
  vector (`AgentLaunchPolicy` in `noren-pty` requires an absolute program and
  never consults a shell or `PATH`), and OpenSSH discovery surfaces at most
  `MAX_SSH_SIDEBAR_HOSTS` (64) positive literal aliases as
  `SidebarEntry::SshConnection` rows that launch the fixed system `ssh`
  client in the terminal's PTY, with wildcard patterns counted and explained
  rather than silently dropped (`SshConfig::unlisted_wildcard_patterns`).
  Keybindings are configurable through `[keys]` (see
  [Milestone 3 status](#milestone-3-status)). What still blocks is breadth
  and depth: exactly one session owns the viewport at a time, a restored
  row's shell does not exist until it is relaunched, and SSH discovery stays
  explicitly partial by design — no wildcard, `Match`, or token-expansion
  enumeration ever happens.
- **The default `dark` palette fails WCAG AA on its own background.**
  `[theme] name` in `config.toml` selects one of three built-in themes —
  `dark` (the default), `light`, and `high-contrast` — and every theme
  carries a measured, test-pinned contrast floor (`theme.rs` and
  `crates/noren-app/tests/theme.rs`; the selection reaching the renderer is
  pinned app-level by `configured_theme_reaches_the_app_renderer_input`).
  The blocker is the default itself: the `dark` palette's worst slot is ANSI
  black at **1.06:1**, and five of its sixteen ANSI entries fall below the
  4.5:1 AA floor for normal text — pinned deliberately by
  `default_dark_palette_minimum_is_pinned_below_the_aa_floor`, because the
  no-`[theme]` default must stay byte-identical to the pre-theme renderer
  (`dark_theme_is_byte_identical_to_the_pre_theme_renderer`) and changing the
  values is a separate, currently-untaken decision (issue #168). `light`
  (minimum 5.07:1) and `high-contrast` (7.84:1, beyond AAA) pass every slot,
  so a user who needs AA must know to opt in — a preview whose default look
  is below AA cannot ship as the product's face. Themes are built-in only:
  no custom-palette or colour-vision-friendly surface exists.
- **The font is a hand-built 5x7 bitmap with bounded coverage.** Printable
  ASCII keeps distinct upper/lower case, and the Latin-1 Supplement
  (`U+00A0..=U+00FF`) and Box Drawing (`U+2500..=U+257F`) blocks have
  per-character bitmaps, but every other code point — CJK text and emoji
  included — draws a fixed replacement glyph: unreadable boxes laid out at
  the **correct** two-column width, pinned through real pixels by
  `cjk_text_occupies_two_cells_per_character_and_fails_visibly_not_corruptingly`
  and its wide-character and combining-mark neighbours in
  `crates/noren-app/tests/frame_oracle.rs`, but with no glyphs. Rendering
  real CJK needs a real font stack, which is deliberately not claimed.
  Seven glyph pairs remain visually identical by an allowlisted collision
  set (pinned by
  `covered_range_glyph_collisions_match_the_hardcoded_allowlist` in
  `renderer.rs`'s tests). Both former font defects — case-folding and
  non-ASCII rendering as `?` — are retired and guarded by running, passing
  tests (`lowercase_distinct_from_uppercase`,
  `non_ascii_glyph_is_not_the_question_mark`), not `#[ignore]`d ones.
- **IME input is discarded, and there is no accessibility surface.** The
  `WindowEvent::Ime(_)` arm in `main.rs`'s event handler drops the event
  without forwarding it, so Japanese, Chinese, and Korean input methods
  produce nothing; nothing in the tree integrates with assistive
  technology. Both are Milestone 6 scope and neither has started.
- **The view layer is incomplete beyond colour and glyphs.** There is no
  visible cursor: `glyph_vertices` in `renderer.rs` emits sidebar rows,
  per-cell backgrounds, glyph bitmaps, and a status line — never a cursor —
  and the word "cursor" does not appear in that file. There is no scrollback
  viewport: rendering stays on the newest suffix of the content because no
  scroll-offset input exists (the only scroll offset in `main.rs` is the
  sidebar's own). Selection is tracked and copies correctly but is never
  highlighted. A terminal in which a stranger cannot see the cursor, cannot
  scroll back, and cannot see their selection is not preview-ready.
- **An open correctness defect sits on the CJK layout path.** `CSI T` (SD),
  `CSI S` (SU), and `CSI M` (DL) shift rows without re-snapping the cursor
  to a wide character's lead cell, so the cursor can be stranded on a
  continuation cell — the very contract the frame oracle pins. Found by the
  parser fuzz harness (`crates/noren-terminal/tests/fuzz_feed_bytes.rs`),
  reported as issue #176, and deliberately not yet fixed. It is reachable
  by any pager or TUI that scrolls a region while CJK text is on screen.
- **The FR-005 rendered-frame oracle exists and runs, and its boundary is
  the honesty requirement.** It drives the real `wgpu` pipeline offscreen
  (`crates/noren-app/src/renderer_capture.rs`,
  `crates/noren-app/tests/frame_oracle.rs`); its active assertions cover
  structure, SGR foregrounds, ANSI/256-colour and direct RGB resolution,
  defaults, explicit backgrounds, and the CJK width contract. It does
  **not** verify that an `A` is shaped like an A, and there is still no key
  injection into the real window, so live keyboard input remains unverified
  by automation — the manual macOS gate is the only perceptual check. A
  preview must state this boundary rather than imply the oracle proves more.
- **Live-Zellij pass-through evidence is gathered by CI but gates nothing.**
  `.github/workflows/zellij-live.yml` installs the pinned Zellij release
  and runs `crates/noren-app/tests/zellij_live.rs` for real on every PR,
  push to `main`, and nightly, with guards that fail the job on a failed
  install, a checksum or version mismatch, or any skip notice. The job is
  deliberately **not** in the branch-protection required-check list
  (advisory by choice, because the pinned artifact lives upstream): a red
  live suite does not block a merge, so a pass-through regression can land
  between the runs that notice it.
- **NFR-009 requires release-integrity gates** — signing, notarization,
  packaging — to pass before any Preview claim. A local `cargo build`
  produces an arm64 binary carrying only macOS's automatic ad-hoc signature
  (`codesign -dvvv` reports `Signature=adhoc` and `TeamIdentifier=not
  set`), so a distributed binary would meet Gatekeeper as an unidentified
  developer. Checksums, release tags, and publication are
  release-integrity concerns under the same requirement, and none exist.

Milestone 8 therefore stops at a release candidate. Signing keys, Apple
certificates, tagging, and publication are owner decisions and are not taken
autonomously.
