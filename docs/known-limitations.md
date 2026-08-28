# Known limitations

This document exists so that the first thing a reader meets is what Noren
**cannot** do today, not what it hopes to do. Decision D-M8-001 settled that the
first artifact is an explicitly dated developer preview, not a
`0.1.0-preview` of the product; this page is the substance behind that framing.
This page retains a 2026-08-08 verification baseline and was re-verified clause
by clause against the working tree at the 2026-08-27 milestone-gate sync.
Citations point at the file, function, type, constant, or test that establishes
them.
Names are used rather than line numbers, which rot; where a count is genuinely
needed the command that reproduces it is given instead. If anything here has
drifted, treat the code as correct and this page as a bug.

## What Noren is today

Noren today is a **working single-session workspace on a terminal foundation**:
a macOS window backed by a PTY that spawns a local `/bin/zsh`, a
renderer-independent terminal state core (scroll regions, alternate screen, SGR
attributes, Unicode display width, bounded scrollback, selection, clipboard,
search), and an input encoder covering xterm modifier parameters and
application cursor/keypad modes.

Since the first draft of this page, the Milestone 3 vertical slice reached the
binary. What now actually happens on screen:

- **A sidebar is drawn.** The leftmost `SIDEBAR_COLS` columns (a constant equal
  to 16 in `renderer.rs`) are reserved for workspace rows, and the terminal is
  drawn starting at that column offset with its grid narrowed to match — see
  `glyph_vertices` in `renderer.rs`, which takes a `sidebar` argument and
  applies `col_offset`, and `sidebar_text_lines` in `main.rs`, which formats the
  rows.
- **The session palette operates on real local sessions.** `Super+p` opens
  the command palette (claimed by `palette_policy` in `main.rs` as
  `PassthroughAction::OpenCommandPalette`); the opener and the four command
  chords are configurable through `[keys]` in `config.toml` with
  `super+p`/`c`/`s`/`x`/`f` as the defaults. The `c` command spawns a real
  `/bin/zsh` PTY and gives it the live view (parking the previous one, not
  killing it); `s` cycles the live view to the next live session in sidebar
  order; `x` closes the selected row, reaping its child and falling back to
  the topmost remaining live session — or an honest empty view when it was the
  last one. The `f` command dispatches sidebar focus — currently a no-op, since
  the sidebar is always visible. Arrow
  keys and Enter navigate the same command list; Escape dismisses it.
- **Mouse reporting reaches the program.** `handle_mouse_button`,
  `handle_mouse_move`, and `handle_mouse_wheel` in `main.rs` each call
  `encode_and_send_mouse`, the helper that invokes `MouseEncoder::encode` and
  writes the resulting report bytes to the PTY. Encoding follows the terminal
  state's authoritative mode tracking (`current_mouse_modes` in `main.rs`): a
  program that never enables a tracking mode receives no reports, and holding
  Shift bypasses reporting so local text selection still works
  (`mouse_reportable`). Clicks, drags, and the wheel therefore reach programs
  that ask for them — Zellij, `vim` with `set mouse=a`, and `tmux` among them.
- **Configured cell size reaches the renderer.** `[font] cell_width` /
  `cell_height` flow through `GridGeometry::with_cells` to the drawing path;
  the regression test `configured_cell_sizes_drive_the_app_geometry` in the
  binary's extracted test module (`src/main/tests.rs`) pins it, and a mutation
  note in `renderer.rs`'s tests records that
  reverting `push_glyph` to the fixed `POC_CELL_WIDTH` constants must fail.
- **Sidebar state survives a restart.** State is written to `sessions.toml`
  (`SESSION_STATE_FILE_NAME`) in the directory `config::default_path` resolves —
  `~/Library/Application Support/Noren/` on macOS — resolved by
  `session_state_path` in `main.rs`. With `HOME` unset the app runs in-memory
  only, by design. Saves are evidence-based rather than fire-and-forget:
  `persist` in `main.rs` snapshots the file before and after each write
  (`PersistenceState`), an external replacement becomes a sticky conflict
  surfaced through the diagnostics line (`with_persistence_conflict`), and a
  save that cannot be inspected or verified says so
  (`with_persistence_unverified`) — failures print to stderr and never crash
  the app.
- **Session status reflects the real lifecycle.** `SessionStatus` advances
  `Starting -> Running -> Exited/Failed` through `SessionRegistry::observe`
  rather than sitting at a permanent "starting"; `main.rs` observes `Running` on
  spawn and `Exited { code }` on child exit.
- **The SSH sidebar is bounded and explicitly partial.** `SshConfig` labels its
  scope as `HostDiscoveryKind::PartialLiteralPatterns`: only positive literal
  aliases written in `Host` directives become browseable targets. Wildcard
  patterns never become rows — a pattern is a matching rule, not a connectable
  destination — but they are counted per occurrence (including patterns inside
  blocks that also name literals, and patterns reached through followed
  Includes) and reported as `N wildcard patterns not listed` in the status
  notice, so a pattern-built config is never silently under-represented
  (issue #175). Negations are OpenSSH filters rather than destinations: they
  suppress matches during resolution and self-negated literals during
  discovery, and they add no row and no absence count. `HostName`,
  `User`, and `Port` participate in bounded first-value resolution, but
  `HostName` and `User` remain literal: `%h`, `%p`, `%r`, and other percent
  tokens are not expanded. Those values are discovery metadata and must be
  resolved with OpenSSH-equivalent semantics or rejected before any future
  connection use. Root-relative `Include` files are followed in lexical order
  only when their canonical targets remain below the top-level config
  directory — the same block list feeds discovery and resolution, so a host
  declared only in an included file is discovered exactly like one in the
  top-level file. That canonical-root confinement is intentionally stricter
  than OpenSSH: absolute, `~`, `..`, and symlinked targets outside the root are
  ignored. `Match`, system configuration, token
  expansion, and other dynamic OpenSSH behaviour cannot make this a complete
  host inventory. The UI says `partial literal aliases`, retains at most
  `MAX_SSH_SIDEBAR_HOSTS` (64), and beyond the cap reports how many are shown
  of how many and why (`showing first 64 of 70; 6 past sidebar bound`); it
  shows the selected alias's stable source tag
  plus a bounded root-relative label; it never retains or displays the
  canonical HOME prefix. Hostile configuration fails closed rather than
  hanging: every budget in `ssh_config.rs` (`MAX_FILE_BYTES`,
  `MAX_TOTAL_BYTES`, `MAX_TOKEN_ITEMS`, `MAX_HOSTS`, `MAX_RESOLUTION_WORK`,
  and the include-expansion budget) rejects its input before the work runs, so
  a hostile file becomes a content-free diagnostic line through
  `report_ssh_diagnostic` in `main.rs`, not a stall. That closure is history,
  not luck: the parser's first glob matcher was exponential (`87a67b3` replaced
  it with an iterative one) and its first alias scans were quadratic
  (`4f698af` made parsing linear in the number of hosts); FIFOs and symlink
  races are handled at open time (`open_regular_file`) and pinned by bounded
  subprocess regression tests (`top_level_fifo_returns_promptly`,
  `included_fifo_sources_return_promptly`).

It is still **not** the full workspace product that [ADR
0003](adr/0003-noren-zellij-responsibility-boundary.md) describes — several
sessions can be live at once and switching between them is real
(`switch_live_session` in `main.rs` parks the previous surface — its PTY
keeps running and is still drained (`drain_parked_sessions`) — and
re-attaches the selected session's own screen; pinned by
`sidebar_click_switches_the_live_surface_between_sessions` and
`a_parked_session_that_exits_is_observed_and_detached` in
`src/main/tests.rs`), but exactly one session owns the viewport at a time:
there is no split, tiled, or multi-session view. Configured SSH targets are
now discoverable in the sidebar, and selecting one launches the system `ssh`
client for that alias in the live view's PTY, replacing the current session
(the one non-local launch path). See "Session switching exists, within one
viewport" under "What does not work"
below, which remains the more useful list. Milestones 3–8 are open on the
[roadmap](../ROADMAP.md).

## What does not work

Each item states what you would actually see if you ran the build.

- **There is no visible cursor.** The terminal state tracks a cursor position
  and moves it correctly, but the render path never draws it: the
  `glyph_vertices` function (`crates/noren-app/src/renderer.rs`) emits sidebar
  rows, optional per-cell background rectangles, character bitmaps from
  `display_cells`, and an optional status line — never a cursor — and the
  word "cursor" does not appear anywhere in `renderer.rs`. In practice: you type, characters appear,
  and nothing shows you where the insertion point is. This is the first thing
  most people notice.
- **Colours render through a selectable theme; the default palette now
  clears WCAG AA on every slot, with two residual caveats.** `[theme] name`
  in `config.toml` selects one of three built-in palettes — `dark` (the
  default), `light`, and `high-contrast` — and `glyph_vertices_for`
  in `crates/noren-app/src/renderer.rs` resolves every SGR colour, the
  default foreground, and the clear colour through it
  (`resolve_foreground`/`resolve_background` route ANSI and 256-colour
  values through the theme's table; truecolor passes through). **The
  issue-168 fix:** the default `dark` palette used to fail the WCAG AA
  normal-text floor (4.5:1) on five of its sixteen ANSI slots — black at
  1.06:1, blue at 2.10:1, red at 3.38:1, bright blue at 4.16:1, and
  magenta at 4.21:1 — which left `\x1b[30m` text effectively invisible.
  Issue #168 made the decision PR #167 had deliberately frozen and moved
  exactly those five entries the minimum distance that clears 4.5:1
  (black `[0,0,0]`→`[121,121,121]` 1.06→4.53, red `[205,0,0]`→`[243,0,0]`
  3.38→4.52, blue `[0,0,238]`→`[0,113,255]` 2.10→4.52, magenta
  `[205,0,205]`→`[213,0,213]` 4.21→4.50, bright blue
  `[92,92,255]`→`[100,100,255]` 4.16→4.52), preserving ANSI slot
  semantics. The default's measured minimum is now 4.50:1 (magenta),
  pinned by `dark_theme_keeps_aa_on_every_theme_owned_foreground` and
  `the_five_fixed_dark_entries_measure_above_their_old_failures` in
  `crates/noren-app/tests/theme.rs`, and confirmed on drawn pixels by
  `the_issue_168_aa_fixes_reach_the_drawn_pixels` in
  `crates/noren-app/tests/frame_oracle.rs`. `light` (measured minimum
  5.07:1) and `high-contrast` (7.84:1, above WCAG AAA) were untouched and
  pass every slot. **The caveats:** (1) ANSI black and bright black now
  sit close together — `[121,121,121]` vs `[127,127,127]` — because any
  achromatic entry clearing AA on this background must be at least grey
  121 and bright black was already 127; both remain neutral greys, but a
  program relying on a large black/bright-black distinction will not find
  one in the default theme. (2) The contrast contract still covers only
  theme-owned foregrounds on the theme's default background: the shared
  240-colour cube/grayscale tail (`16..=255`), truecolor, and
  program-paired foreground/background combinations remain unchecked and
  can still draw below the floor. Themes are built-in only — there is
  no custom-palette or colour-vision-friendly configuration.
- **The bitmap font is coverage-bounded, and seven glyph pairs are visually
  identical.** Glyphs are a hand-built 5x7 bitmap in `glyph_rows`
  (`crates/noren-app/src/renderer.rs`): printable ASCII keeps distinct
  upper/lower case, the Latin-1 Supplement (`U+00A0..=U+00FF`) and Box
  Drawing (`U+2500..=U+257F`) blocks have per-character bitmaps, and every
  other code point draws a visible replacement glyph rather than `?` — so
  CJK text and emoji still do not render. What **is** true end to end is
  their layout: the terminal state core measures CJK and emoji display
  width correctly (Unicode/CJK display width, recorded in the
  [roadmap](../ROADMAP.md)), and the renderer honours that model — a wide
  character occupies two columns (its lead draws the replacement glyph,
  the continuation column stays blank), the glyph after the pair lands at
  its correct display column, and a combining mark is drawn over its base
  cell without consuming a cell. Japanese output therefore appears as
  unreadable replacement boxes that keep every surrounding column aligned,
  not as corrupted text; the frame oracle pins this width contract through
  real pixels (`cjk_text_occupies_two_cells_per_character_and_fails_visibly_not_corruptingly`,
  `wide_character_then_ascii_keeps_the_ascii_at_its_display_column`,
  `combining_marks_consume_no_cell_through_the_pipeline` in
  `crates/noren-app/tests/frame_oracle.rs`). Rendering actual CJK glyphs
  needs a real font stack, which is its own milestone decision and is
  deliberately not claimed here. Coverage is deliberately **not**
  claimed collision-free: seven pairs of distinct characters are visually
  identical because at 5x7 their pixel grids genuinely coincide —
  space/`U+00A0` (both blank), and hyphen/`─`, slash/`╱`, equals/`═`,
  backslash/`╲`, bar/`│`, broken-bar/`╎` against their box-drawing
  equivalents. The test
  `covered_range_glyph_collisions_match_the_hardcoded_allowlist` in
  `renderer.rs` enumerates every pair across the three covered ranges and
  pins the collision set to exactly that hardcoded allowlist, so a future
  glyph edit cannot reintroduce a case-blind or diacritic-losing collision
  (or invent a new box-drawing alias) silently.
- **IME input is discarded.** `WindowEvent::Ime(_)` is dropped without reaching
  the terminal — the `WindowEvent::Ime(_)` arm in `main.rs`'s event handler
  drops the event without forwarding it. Japanese, Chinese, and
  Korean input methods produce nothing. For a user typing Japanese into
  Noren today this means: composing with an IME inserts no text at all
  (the composition never reaches the PTY), and any Japanese that a running
  program *outputs* (`cat`, `ls`, a build log) draws as unreadable
  replacement boxes — laid out at the correct two-column width so the
  surrounding ASCII stays aligned, but the characters themselves cannot be
  read. Neither half works today; both are Milestone 6 scope, and the width
  half is the only half that is verified.
- **There is no accessibility surface.** Nothing in the tree integrates with
  assistive technology (no AccessKit, AT-SPI, or AppKit accessibility wiring);
  a screen reader has nothing to work with.
- **Selection and scrollback work, but you cannot see them.** Selection is
  tracked and copy extracts it, yet the renderer does not highlight the selected
  region — the comment on the `selection` field in `main.rs` says so in the
  source itself ("The renderer does not highlight it yet"). Scrollback is
  bounded and searchable, but `FrameRowLayout::new` derives
  `terminal_row_count` from `content_rows.min(terminal_capacity)`, then
  `first_terminal_line` as `content_rows - terminal_row_count`. There is no
  scroll-offset input, so rendering stays on the newest suffix and you cannot
  scroll back through it. The data is there; the view onto it is not.
- **Paste is bracketed-paste-only.** `Cmd+V` never sends raw clipboard bytes:
  `paste_bytes` in `main.rs` wraps the text in bracketed-paste markers only
  when the program enabled DEC private mode 2004, and every other case is
  gated with a visible status line (`show_paste_gate`) instead of sending
  something unbracketed — `paste_is_gated_when_mode_2004_is_off_or_terminal_unavailable`
  and `paste_is_bracketed_when_mode_2004_is_enabled` pin it. In practice: you
  cannot paste into a program that has not asked for bracketed paste, and
  oversized or empty clipboard text is gated (`PasteReject::Oversized`,
  `PasteReject::Empty`) rather than truncated or sent.
- **macOS only, one fixed shell.**   `Renderer::new` requests Metal exclusively
  (`instance_descriptor.backends = wgpu::Backends::METAL` in
  `crates/noren-app/src/renderer.rs`), so it does not *run* on other platforms —
  on a non-macOS host it acquires no adapter, the renderer fails to start, and
  the window opens to show the "Noren renderer start failed" status (set in the
  `Renderer::new` error arm of `main.rs`'s `initialize`) rather than a usable
  terminal. (The app crate has no `cfg(target_os)` gating, so it may well
  compile elsewhere; compiling is not the barrier, running is.)
  The PTY launches `/bin/zsh` with a fixed policy and no caller-controlled
  arguments (`ZSH_PROGRAM` in `crates/noren-pty/src/lib.rs`). Linux support is roadmap intent
  (Milestone 6), not current capability.
- **Worktree, project, and agent sessions launch; the SSH kind does not go
  through the spawn gate.** `SessionKind` models
  `Local`, `Project`, `Worktree`, `Ssh`, and `Agent`, and `EntryKind` in
  `sidebar.rs` can describe project, worktree, SSH-connection, and agent rows —
  `Local`, `Project`, `Worktree`, and `Agent` have launch paths (an SSH
  launch runs the system client through its own connection path). The
  running binary reads bounded
  OpenSSH configuration facts and displays configured targets as
  `SidebarEntry::SshConnection` rows. Clicking one validates the alias as an
  `SshDestination` (raw `%h`/`%p`/`%r` tokens are refused with a typed error
  naming the keyword and token) and, if accepted, launches the fixed system
  `/usr/bin/ssh` client in the live view's PTY — argv is exactly
  `ssh -- <alias>`, so no credential, identity, or option is ever
  `ps`-visible — replacing the previous session. Launch, connect, and
  disconnect failures surface as visible per-row and status-row states; the
  alias never enters the persisted registry, so `sessions.toml` cannot carry
  it. Authentication, agent, and config resolution (including the user's own
  `ProxyCommand`) remain entirely ssh's own, executed by the system binary.
  At startup Noren also runs `git worktree list --porcelain` in its launch
  directory and shows the discovered worktrees as `SidebarEntry::Worktree`
  rows (at most 24; a larger list reports the omitted count). Clicking a
  present row starts a real `SessionKind::Worktree` session: a `/bin/zsh`
  PTY whose child's working directory IS that worktree (verified by reading
  the child's own `pwd` back through the terminal; note that the PTY-level
  `spawn_in_dir_runs_the_child_in_that_directory` test cannot by itself
  distinguish an honoured working directory from portable-pty's HOME
  fallback, issue #162 — the app-level `pwd` proof is the guarantee). A
  registered worktree whose directory was deleted from disk is listed with a
  `(missing)` marker
  and refused on selection — no panic, no child. Worktree sessions persist
  and restore through `sessions.toml` exactly like local ones.
  `[[projects]]` configuration entries appear as `EntryKind::Project` rows
  (at most 24, capped like every other list; the fixed `PRJ-` state prefix
  distinguishes them from prefix-less worktree rows), and selecting one
  whose root still exists starts a real `SessionKind::Project` session — the
  same directory-rooted launch shape as a worktree, with the child's own
  `pwd` proof; a configured-but-gone root is refused visibly like a deleted
  worktree. `[[agents]]` entries launch their configured, shell-free argv in
  a PTY (PR #169). Project and agent sessions persist and restore through
  `sessions.toml` exactly like every other kind. In practice: startup owns
  exactly one local `zsh`, the palette's "New Session" spawns another real
  local `zsh`, and every local row can take the live view; a restored row
  cannot (its shell died with the previous launch). Selecting an SSH host
  row launches the system `ssh` client in the terminal's PTY (see the SSH
  section); selecting a worktree, project, or agent row starts a real
  session rooted at that row's identity.
- **The SSH list is not OpenSSH-equivalent discovery.** A readable config can
  legitimately name destinations that do not appear: wildcard or negated
  patterns are matching policy rather than concrete aliases, `Match` and token
  expansion are not evaluated into destinations, and includes outside the
  top-level configuration directory are deliberately ignored even though
  OpenSSH may accept them. A retained `HostName` or `User` can therefore still
  contain literal `%h`, `%p`, or `%r`; it is not safe connection input until a
  future resolver expands it with OpenSSH-equivalent semantics or rejects it.
  The status row therefore never calls the rows a complete host list. It
  shows at most the first `MAX_SSH_SIDEBAR_HOSTS` (64) literal aliases — the
  same bound the sidebar section above states, pinned by
  `many_ssh_hosts_are_bounded_and_report_the_omitted_count` — and reports an
  omitted count; selecting a
  row shows where its first literal declaration came from, but does not prove
  the effective configuration that a future connection will use.
- **Session switching exists, within one viewport.** Clicking a live session
  row in the sidebar (or the palette's `s` command) moves the whole live view
  — terminal surface, input routing, renderer — to that session's own PTY and
  screen; the previous session is parked with its child still running and its
  output still drained and resized in the background, so switching back shows
  current content. Restored or exited rows have no live surface and cannot
  take the live view. There is still no split, tiled, or multi-session view:
  exactly one session owns the viewport at a time. Panes and layout *inside*
  the live session are delegated to Zellij by design — see "What is
  deliberately delegated".
- **Keybindings are configurable for the palette only, within bounds.** The
  `[keys]` table in `config.toml` rebinds the palette opener and the four
  palette command chords (`palette_policy` and `handle_palette_key` in
  `main.rs` read them from `KeymapConfig`), with the pre-configuration
  chords as defaults. The exit leader (`super+escape`), the palette's
  structural keys (Escape, Enter, the vertical arrows), the diagnostics
  chord (Super+D), and the clipboard shortcuts (Super+A/C/V) remain
  compiled in; see [configuration](configuration.md) for the grammar and the
  rejection rules, including the pinned-Zellij-corpus constraint on the
  opener.
- **A restored session's shell is not running.** Sidebar state persists across a
  restart, but a restored entry comes back as `SessionStatus::Restored` — a
  visible row whose PTY does not exist yet. The comment on `teardown` in
  `main.rs` records this as the deliberate meaning of a restored session.

## What is verified, and how

- **Automated tests.** The workspace commits **hundreds of `#[test]` functions
  across three crates**, and the total grows with every merge — reproduce the
  current count with `grep -rh '#\[test\]' crates/ | wc -l` rather than
  trusting any number printed here. The [roadmap](../ROADMAP.md)'s Milestone 2
  completion evidence records the test count at that close as a
  closed-milestone snapshot that does not move; the workspace, mouse-wiring,
  cell-size, persistence, and frame-oracle merges since then each passed CI,
  which is where the later growth is witnessed. The suites include a bounded VT
  compatibility harness, two independent adversarial hostile-input suites, and
  the FR-005 rendered-frame oracle. Workspace behaviour has its own integration
  suites under `crates/noren-app/tests/` — `frame_oracle.rs`,
  `mouse_encoding.rs`, `palette.rs`, `passthrough.rs`,
  `session_adversarial.rs`, `session_domain.rs`, `session_persistence.rs`,
  `session_supervisor.rs`, `sidebar_view.rs`, `verify59_independent.rs`
  (an independently authored verification pass over configuration,
  diagnostics, and scrollback bounds), `security_no_leak.rs` (sentinel
  tests that fail if secret material reaches a log, error, or Debug sink),
  and `zellij_live.rs` (live pass-through evidence against an INSTALLED
  Zellij in a real PTY; each test prints a visible SKIP to the real stderr
  and gathers no live evidence when no `zellij` is on `PATH` — and even
  where Zellij is installed, the suite currently runs on no machine that
  gates a merge, so live pass-through evidence is gathered only where a
  developer happens to run it: issue #153).
- **Four CI gates** required by branch protection (see the
  [roadmap](../ROADMAP.md) and `.github/workflows/`): the Rust workflow
  (`cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` on macOS arm64), the documentation validator
  (`scripts/check_docs.py`), `cargo deny check`, and an MSRV build.
- **A manual macOS check**, re-run at the Milestone 2 close head and recorded
  under the roadmap's manual gate: a release build opened a window, owned a
  direct `zsh` child whose tty reported the expected grid size, and on exit the
  child was reaped and the pty device was gone.

What this evidence now covers that the M2 snapshot did not: PR #89 added the
**FR-005 rendered-frame oracle** (`crates/noren-app/tests/frame_oracle.rs`,
drawing through the shipped pipeline via
`crates/noren-app/src/renderer_capture.rs`). It re-compiles the real `wgpu`
glyph pipeline and drives it offscreen with structural assertions, never a
golden image: a cell the state says is blank contains no lit pixels; distinct
glyphs produce distinct lit patterns; a glyph lights its own cell and not its
neighbours; the drawn grid matches the state grid; and per-cell lit/blank agrees
with `TerminalSnapshot` across the FR-005 fixture classes. Its active
colour-aware assertions also cover distinct SGR foregrounds, unchanged defaults,
ANSI/256-colour and direct RGB resolution, explicit truecolor backgrounds,
background-only spaces, and indexed/background equivalence. The Milestone 6
CJK slice added pixel-level pins of the wide-character width contract through
the whole `feed_bytes` → state → GPU chain: `日本語` and a wide emoji occupy
two cells per character with the follower at its display column, and a
combining mark consumes no cell. It does **not**
assert an `A` looks like an A — only that the structure and resolved pixel
colours are right. Its two former defect specifications
(`lowercase_distinct_from_uppercase`, `non_ascii_glyph_is_not_the_question_mark`)
are no longer `#[ignore]`d and no longer failing: the bitmap font now
distinguishes case, Latin-1 Supplement and Box Drawing have built-in coverage,
and unsupported Unicode draws a replacement glyph instead of `?`. The oracle's
GPU dependence is real and remains: it needs a headless Metal adapter. A
machine with no adapter at all skips each oracle test visibly (a `SKIP`
notice on the real stderr stating that rendered-frame evidence was NOT
gathered — a skip is never reported as a pass), while an adapter that exists
but fails to yield a device stays red as `offscreen=blocked`; the skip policy
itself is exercised by the `NOREN_FRAME_ORACLE_ADAPTER` modes.

What this evidence does **not** cover: there is still **no key injection into
the real window**, so live input is unverified end-to-end — the byte-level
input contract is tested at the `KeyEncoder`, but no test synthesizes a real
key event into a live window and observes the result. The oracle guards
structural shape, grid mapping, and resolved pixel colour, but not glyph
identity or overall perceptual correctness, so the manual check above remains a
smoke test of the window→grid→PTY chain rather than a perceptual proof of what
appears on screen. Theme selection is configurable and its drawing path is
oracle-tested, but the built-in palettes are fixed tables — no custom theme
surface exists to test; IME and accessibility remain absent from both
testing and the build.

## What is deliberately delegated

Panes, tabs, splits, and layout **inside** a terminal session belong to Zellij
(or to whatever runs inside the session), not to Noren. This is a design
boundary recorded in [ADR
0003](adr/0003-noren-zellij-responsibility-boundary.md), not a missing feature:
two layers owning the same abstraction was judged worse than delegating it.
When Zellij is running, correct input pass-through takes priority over Noren
shortcuts. Please do not file the absence of native tabs or panes as a bug —
but do hold Noren to its side of the boundary: a workspace *outside* the
terminal. That side now has a first vertical slice — a drawn sidebar, a command
palette over real local sessions (spawn, switch, close), live switching between
them, and state that survives a restart
— and the gaps that remain there (non-local session
kinds, reattaching a restored session's shell) are
legitimate things to report; keybindings are configurable through
`[keys]` now, with the live winit dispatch gap noted above.

## What this preview is not

No binary, installer, checksum, or release tag is published with this document;
building and running from source is the only way to see the current state, and
publishing any artifact is a reserved owner decision. A local `cargo build`
produces an arm64 binary carrying only macOS's automatic ad-hoc signature — no
signing identity and no notarization (`codesign -dvvv <binary>` reports
`Signature=adhoc` and `TeamIdentifier=not set`). NFR-009's signing,
notarization, and packaging gates are therefore unmet, and the application
shows no first-launch warning about any of this; verify any build yourself
rather than trusting a copy from elsewhere. Nothing here should be
read as "nearly done": the honest summary is that the foundation is tested, a
first workspace slice is now real and visible, and what stands between this and
a usable daily terminal is not workspace plumbing but the display itself — a
real font and a cursor (the default theme's palette cleared WCAG AA on every
slot with issue #168). SGR colour is drawn through selectable
light/dark/high-contrast themes with verified contrast, but the built-in
palettes are not yet a custom theming system.
