# Known limitations

This document exists so that the first thing a reader meets is what Noren
**cannot** do today, not what it hopes to do. Decision D-M8-001 settled that the
first artifact is an explicitly dated developer preview, not a
`0.1.0-preview` of the product; this page is the substance behind that framing.
This page retains a 2026-08-08 verification baseline and was re-verified clause
by clause against the working tree at the 2026-08-20 milestone-gate sync.
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
- **The session palette is present, but only one PTY is live.** `Super+p` opens
  the command palette (claimed by `palette_policy` in `main.rs` as
  `PassthroughAction::OpenCommandPalette`). Its `c` command adds a model row;
  it does not start another shell. The `s` and `x` commands cannot move or
  remove the startup PTY's input owner, while `f` dispatches sidebar focus —
  currently a no-op, since the sidebar is always visible. Arrow
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
  aliases written in `Host` directives become browseable targets. `HostName`,
  `User`, and `Port` participate in bounded first-value resolution, but
  `HostName` and `User` remain literal: `%h`, `%p`, `%r`, and other percent
  tokens are not expanded. Those values are discovery metadata and must be
  resolved with OpenSSH-equivalent semantics or rejected before any future
  connection use. Root-relative `Include` files are followed in lexical order
  only when their canonical targets remain below the top-level config
  directory. That canonical-root confinement is intentionally stricter than
  OpenSSH: absolute, `~`, `..`, and symlinked targets outside the root are
  ignored. `Match`, wildcard-only destinations, system configuration, token
  expansion, and other dynamic OpenSSH behaviour cannot make this a complete
  host inventory. The UI says `partial literal aliases`, retains at most
  `MAX_SSH_SIDEBAR_HOSTS` (24), and shows the selected alias's stable source tag
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
0003](adr/0003-noren-zellij-responsibility-boundary.md) describes — one local
shell at a time; configured SSH targets are now discovered and selectable in
the sidebar, but no non-local session can launch. See "What does not work"
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
- **Colours render, but the palette and theme are fixed.** `glyph_vertices`
  reads each terminal cell's attributes: `resolve_foreground` and
  `resolve_background` route ANSI and 256-colour values through
  `DEFAULT_PALETTE`, while direct RGB truecolor passes through unchanged
  (`crates/noren-app/src/renderer.rs`). Explicit backgrounds emit a full-cell
  rectangle before the glyph. Each vertex now carries a `Float32x2` position
  and `Float32x3` resolved colour on a 20-byte stride, and `fs_main` returns
  that per-vertex colour. The defaults and xterm-style palette are compiled in;
  `config.rs` exposes no palette or theme setting. In practice, SGR foreground
  and explicit background colours appear, but users cannot select or customise
  a light, dark, high-contrast, or colour-vision-friendly theme.
- **The font cannot distinguish case.** Glyphs are a hand-built 5x7 ASCII
  bitmap indexed through `character.to_ascii_uppercase()` inside the
  `glyph_rows` function (`crates/noren-app/src/renderer.rs`); the test
  `ascii_glyphs_are_distinct_and_unknown_is_question_mark` asserts
  `glyph_rows('a') == glyph_rows('A')`. `a` and `A`
  are pixel-identical, so code, filenames, and password prompts lose case
  visually.
- **All non-ASCII renders as `?`.** Every character outside the bitmap table
  falls through to the question-mark glyph — the final `_ =>` default arm of
  `glyph_rows` (`crates/noren-app/src/renderer.rs`) — asserted by the same
  `ascii_glyphs_are_distinct_and_unknown_is_question_mark` test. CJK text,
  accented characters, box-drawing output, and
  emoji all appear as `?` — even though the terminal state core measures their
  display width correctly (Unicode/CJK display width, recorded in the
  [roadmap](../ROADMAP.md)).
- **IME input is discarded.** `WindowEvent::Ime(_)` is dropped without reaching
  the terminal — the `WindowEvent::Ime(_)` arm in `main.rs`'s event handler
  drops the event without forwarding it. Japanese, Chinese, and
  Korean input methods produce nothing.
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
  arguments (`ZSH_PROGRAM` in `crates/noren-pty/src/lib.rs`). Linux support and SSH/remote
  sessions are roadmap intent (Milestones 4 and 6), not current capability.
- **Only the startup local session is actually launched.** `SessionKind` models
  `Local`, `Project`, `Worktree`, `Ssh`, and `Agent`, and `EntryKind` in
  `sidebar.rs` can describe project, worktree, SSH-connection, and agent rows —
  but only `Local` has a launch path. The running binary now reads bounded
  OpenSSH configuration facts, constructs `SessionKind::Ssh`, and displays
  configured targets as `SidebarEntry::SshConnection` rows. Clicking one only
  records a pending target; it opens neither an SSH connection nor a PTY.
  Project and worktree kinds remain modelled, while agent entries remain
  reserved fixtures and no agent is launched. In practice: startup owns exactly
  one local `zsh`. The palette's "New Session" currently records another local
  model row but does not spawn a PTY, and an inactive or restored row cannot
  take the live PTY's selection/input ownership. There is no way to open an SSH
  host, a git worktree, or an agent from the workspace. Milestones 4 and 5 own
  the remaining work.
- **The SSH list is not OpenSSH-equivalent discovery.** A readable config can
  legitimately name destinations that do not appear: wildcard or negated
  patterns are matching policy rather than concrete aliases, `Match` and token
  expansion are not evaluated into destinations, and includes outside the
  top-level configuration directory are deliberately ignored even though
  OpenSSH may accept them. A retained `HostName` or `User` can therefore still
  contain literal `%h`, `%p`, or `%r`; it is not safe connection input until a
  future resolver expands it with OpenSSH-equivalent semantics or rejects it.
  The status row therefore never calls the rows a complete host list. It shows
  only the first 24 literal aliases and reports an omitted count; selecting a
  row shows where its first literal declaration came from, but does not prove
  the effective configuration that a future connection will use.
- **There is one live session, not session switching.** The sidebar may list
  restored or palette-created model entries, but only the startup session owns
  the terminal viewport and input. Clicking an inactive row cannot move that
  ownership. There is no split, tiled, or multi-session view. Panes and layout
  *inside* the live session are delegated to Zellij by design — see "What is
  deliberately delegated".
- **Keybindings are not configurable.** The palette opener (`Super+p`), the exit
  leader (`Super+Escape`), and the `c`/`s`/`x`/`f` command keys are compiled in:
  `palette_policy` and `handle_palette_key` in `main.rs` hard-code them, and the
  config parser (`config.rs`) exposes no keybinding or keymap surface. Rebinding
  requires editing source.
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
  `session_supervisor.rs`, `sidebar_view.rs`, and `verify59_independent.rs`
  (an independently authored verification pass over configuration,
  diagnostics, and scrollback bounds).
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
background-only spaces, and indexed/background equivalence. It does **not**
assert an `A` looks like an A — only that the structure and resolved pixel
colours are right. Its two `#[ignore]`d tests (`lowercase_distinct_from_uppercase`,
`non_ascii_glyph_is_not_the_question_mark`) are the executable specifications of
the two font defects listed above — case-blindness and non-ASCII falling
through to `?` — left failing rather than weakened, and their `#[ignore]`
attributes say so in the source. The oracle needs
a headless Metal adapter and reports `offscreen=blocked` honestly when the host
has none.

What this evidence does **not** cover: there is still **no key injection into
the real window**, so live input is unverified end-to-end — the byte-level
input contract is tested at the `KeyEncoder`, but no test synthesizes a real
key event into a live window and observes the result. The oracle guards
structural shape, grid mapping, and resolved pixel colour, but not glyph
identity or overall perceptual correctness, so the manual check above remains a
smoke test of the window→grid→PTY chain rather than a perceptual proof of what
appears on screen. The palette/theme has no user-configurable surface to test;
IME and accessibility remain absent from both testing and the build.

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
palette over model rows, one live local PTY, and state that survives a restart
— and the gaps that remain there (real session switching, non-local session
kinds, configurable keybindings) are legitimate things to report.

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
a usable daily terminal is not workspace plumbing but the display itself —
a user-configurable theme, a real font, and a cursor. SGR colour is now drawn,
but its fixed defaults are not yet a usable theming system.
