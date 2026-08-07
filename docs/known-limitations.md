# Known limitations

This document exists so that the first thing a reader meets is what Noren
**cannot** do today, not what it hopes to do. Decision D-M8-001 settled that the
first artifact is an explicitly dated developer preview, not a
`0.1.0-preview` of the product; this page is the substance behind that framing.
Every claim below was verified against the tree on 2026-08-07; citations point
at the file, function, or test that establishes them, and a line number appears
only where a name would not pin the evidence. If anything here has drifted,
treat the code as correct and this page as a bug.

## What Noren is today

Noren today is a terminal **foundation**: a macOS window backed by a working
PTY that spawns a local `/bin/zsh`, a renderer-independent terminal state core
(scroll regions, alternate screen, SGR attributes, Unicode display width,
bounded scrollback, selection, clipboard, search), and an input encoder that
covers xterm modifier parameters and application cursor/keypad modes. Several of
those state features are not yet visible on screen — see "What does not work"
below, which is the more useful list. It is
**not yet** the workspace product that [ADR
0003](adr/0003-noren-zellij-responsibility-boundary.md) describes: no workspace
sidebar is drawn, and what runs is a single window onto one local shell
(a single `PtySession::spawn` call in `main.rs`'s `initialize`). Milestones 3–8 are open on the
[roadmap](../ROADMAP.md).

## What does not work

Each item states what you would actually see if you ran the build.

- **There is no visible cursor.** The terminal state tracks a cursor position
  and moves it correctly, but the render path never draws it: the
  `glyph_vertices` function (`crates/noren-app/src/renderer.rs`) emits only
  character bitmaps from `display_lines` plus an optional status line, and the
  word "cursor" does not appear anywhere in `renderer.rs`. In practice: you type, characters appear,
  and nothing shows you where the insertion point is. This is the first thing
  most people notice.
- **Everything renders in one colour.** The fragment shader returns a constant
  pale green — the `fs_main` entry point returns a constant
  `vec4<f32>(0.80, 0.92, 0.82, 1.0)` (`crates/noren-app/src/renderer.rs`) — and
  the pipeline's vertex `buffers` slice carries a single `Float32x2` position
  attribute on an 8-byte stride, no colour channel. SGR
  colours — including 256-colour and truecolor — are parsed and modelled in
  terminal state but never reach drawing (`ROADMAP.md:71-72`). In practice:
  `ls --color`, `vim` syntax highlighting, and Zellij's status bar all appear
  in one shade of green.
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
  display width correctly (`ROADMAP.md:38`).
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
  bounded and searchable, but `glyph_vertices` always roots the layout at
  `total_lines.saturating_sub(visible_rows)` with no scroll offset, so the
  viewport always draws the bottom `visible_rows` and you cannot scroll it
  back through it. The data is there; the view onto it is not.
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
- **No workspace sidebar is drawn.** Noren's defining feature per ADR 0003 and
  FR-009 (`docs/requirements/v0.1.md`) is not yet visible on screen. A
  renderer-independent sidebar view model exists — `sidebar.rs` (M3-3) defines
  `EntryKind`, `SidebarRow`, and `SessionViewport`, describing *what* the
  workspace shows without *how* to paint it — but nothing renders it: the
  `sidebar` module is declared only in `lib.rs`, and neither `main.rs` nor
  `renderer.rs` imports it, so the render path still emits only terminal cells
  plus an optional status line (`glyph_vertices`). Sibling workspace
  models are present in the tree (`session.rs`, `session_supervisor.rs`,
  `session_persistence.rs`, `palette.rs`, `passthrough.rs`) but are models and
  persistence, not a painted workspace, so what runs is still one window on
  one local shell.

## What is verified, and how

- **Automated tests.** The workspace commits **hundreds of `#[test]` functions
  across three crates**, and the total grows with every merge — reproduce the
  current count with `grep -rh '#\[test\]' crates/ | wc -l` rather than
  trusting any number printed here. `ROADMAP.md:44` records **353 workspace
  tests passing** at the Milestone 2 close (`1d329a5`), a closed-milestone
  snapshot that does not move; the M2-MOUSE, M3-ADVFIX, M3-5/6/7, M3-3, and
  FR-005 frame-oracle (PR #89) merges since then each passed CI, which is where
  the later growth is witnessed. The suites include a bounded VT compatibility
  harness, two independent adversarial hostile-input suites (`ROADMAP.md:58-60`),
  and the FR-005 rendered-frame oracle added by PR #89.
- **Four CI gates** required by branch protection (`ROADMAP.md:46-47`,
  `.github/workflows/`): the Rust workflow (`cargo fmt --check`, `cargo clippy
  --workspace --all-targets -- -D warnings`, `cargo test --workspace` on macOS
  arm64), the documentation validator (`scripts/check_docs.py`), `cargo deny
  check`, and an MSRV build.
- **A manual macOS check**, re-run at the M2 close head (`ROADMAP.md:62-65`):
  a release build opened a window, owned a direct `zsh` child whose tty
  reported the expected grid size, and on exit the child was reaped and the pty
  device was gone.

What this evidence now covers that the M2 snapshot did not: PR #89 added the
**FR-005 rendered-frame oracle** (`crates/noren-app/tests/frame_oracle.rs`,
drawing through the shipped pipeline via
`crates/noren-app/src/renderer_capture.rs`). It re-compiles the real `wgpu`
glyph pipeline and drives it offscreen to assert **structural** properties,
never a golden image: a cell the state says is blank contains no lit pixels;
distinct glyphs produce distinct lit patterns; a glyph lights its own cell and
not its neighbours; the drawn grid matches the state grid; and per-cell
lit/blank agrees with `TerminalSnapshot` across the FR-005 fixture classes. It
does **not** assert an `A` looks like an A — only that the structure is right.
Its two `#[ignore]`d tests (`lowercase_distinct_from_uppercase`,
`non_ascii_glyph_is_not_the_question_mark`) are the executable specifications of
the two font defects below, left failing rather than weakened. The oracle needs
a headless Metal adapter and reports `offscreen=blocked` honestly when the host
has none.

What this evidence does **not** cover: there is still **no key injection into
the real window**, so live input is unverified end-to-end — the byte-level
input contract is tested at the `KeyEncoder`, but no test synthesizes a real
key event into a live window and observes the result. The oracle guards shape
and grid mapping, not glyph identity or colour, so the manual check above
remains a smoke test of the window→grid→PTY chain rather than a perceptual
proof of what appears on screen. Colour, IME, and accessibility are absent
from testing because they are absent from the build.

## What is deliberately delegated

Panes, tabs, splits, and layout **inside** a terminal session belong to Zellij
(or to whatever runs inside the session), not to Noren. This is a design
boundary recorded in [ADR
0003](adr/0003-noren-zellij-responsibility-boundary.md), not a missing feature:
two layers owning the same abstraction was judged worse than delegating it.
When Zellij is running, correct input pass-through takes priority over Noren
shortcuts. Please do not file the absence of native tabs or panes as a bug —
but do hold Noren to its side of the boundary: a workspace *outside* the
terminal, which is precisely the part not built yet.

## What this preview is not

No binary, installer, checksum, or release tag is published with this document;
building and running from source is the only way to see the current state, and
publishing any artifact is a reserved owner decision. Nothing here should be
read as "nearly done": the honest summary is that the foundation is tested and
the product is not yet present.
