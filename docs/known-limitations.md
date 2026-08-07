# Known limitations

This document exists so that the first thing a reader meets is what Noren
**cannot** do today, not what it hopes to do. Decision D-M8-001 settled that the
first artifact is an explicitly dated developer preview, not a
`0.1.0-preview` of the product; this page is the substance behind that framing.
Every claim below was verified against the tree on 2026-08-07 and cites
`file:line`. If anything here has drifted, treat the code as correct and this
page as a bug.

## What Noren is today

Noren today is a terminal **foundation**: a macOS window backed by a working
PTY that spawns a local `/bin/zsh`, a renderer-independent terminal state core
(scroll regions, alternate screen, SGR attributes, Unicode display width,
bounded scrollback, selection, clipboard, search), and an input encoder that
covers xterm modifier parameters and application cursor/keypad modes. Several of
those state features are not yet visible on screen — see "What does not work"
below, which is the more useful list. It is
**not yet** the workspace product that [ADR
0003](adr/0003-noren-zellij-responsibility-boundary.md) describes: there is no
workspace sidebar, and what runs is a single window onto one local shell
(`crates/noren-app/src/main.rs:118`). Milestones 3–8 are open on the
[roadmap](../ROADMAP.md).

## What does not work

Each item states what you would actually see if you ran the build.

- **There is no visible cursor.** The terminal state tracks a cursor position
  and moves it correctly, but the render path never draws it: `glyph_vertices`
  (`crates/noren-app/src/renderer.rs:275-326`) emits only character bitmaps from
  `display_lines` plus an optional status line, and the word "cursor" does not
  appear anywhere in `renderer.rs`. In practice: you type, characters appear,
  and nothing shows you where the insertion point is. This is the first thing
  most people notice.
- **Everything renders in one colour.** The fragment shader returns a constant
  pale green (`crates/noren-app/src/renderer.rs:35`) and the vertex layout
  carries only a position, no colour channel (`renderer.rs:117-125`). SGR
  colours — including 256-colour and truecolor — are parsed and modelled in
  terminal state but never reach drawing (`ROADMAP.md:71-72`). In practice:
  `ls --color`, `vim` syntax highlighting, and Zellij's status bar all appear
  in one shade of green.
- **The font cannot distinguish case.** Glyphs are a hand-built 5x7 ASCII
  bitmap indexed through `to_ascii_uppercase()` (`renderer.rs:354`); a test
  asserts `glyph_rows('a') == glyph_rows('A')` (`renderer.rs:457`). `a` and `A`
  are pixel-identical, so code, filenames, and password prompts lose case
  visually.
- **All non-ASCII renders as `?`.** Every character outside the bitmap table
  falls through to the question-mark glyph (`renderer.rs:424`, asserted at
  `renderer.rs:458`). CJK text, accented characters, box-drawing output, and
  emoji all appear as `?` — even though the terminal state core measures their
  display width correctly (`ROADMAP.md:38`).
- **IME input is discarded.** `WindowEvent::Ime(_)` is dropped without reaching
  the terminal (`crates/noren-app/src/main.rs:612-614`). Japanese, Chinese, and
  Korean input methods produce nothing.
- **There is no accessibility surface.** Nothing in the tree integrates with
  assistive technology (no AccessKit, AT-SPI, or AppKit accessibility wiring);
  a screen reader has nothing to work with.
- **Selection and scrollback work, but you cannot see them.** Selection is
  tracked and copy extracts it, yet the renderer does not highlight the selected
  region — `main.rs:44-46` says so in the source itself. Scrollback is bounded
  and searchable, but the renderer has no scroll offset and always draws the
  bottom `visible_rows` (`renderer.rs:296`), so you cannot scroll the viewport
  back through it. The data is there; the view onto it is not.
- **macOS only, one fixed shell.** The renderer requests Metal exclusively
  (`renderer.rs:70`), so it does not *run* on other platforms — it acquires no
  adapter and fails at startup. (The app crate has no `cfg(target_os)` gating,
  so it may well compile elsewhere; compiling is not the barrier, running is.)
  The PTY launches `/bin/zsh` with a fixed policy and no caller-controlled
  arguments (`crates/noren-pty/src/lib.rs:33`). Linux support and SSH/remote
  sessions are roadmap intent (Milestones 4 and 6), not current capability.
- **No workspace sidebar.** Noren's defining feature per ADR 0003 and FR-009
  (`docs/requirements/v0.1.md:25`) is absent from the build: there is no
  `sidebar.rs` in `crates/noren-app/src/`. Sidebar-adjacent modules *are* on
  `main` — `session.rs`, `session_supervisor.rs`, `session_persistence.rs`,
  `palette.rs`, `passthrough.rs` — but they are models and persistence, not a
  visible workspace, and `ROADMAP.md:11` still lists Milestone 3 as **Not
  started**.

## What is verified, and how

- **Automated tests.** The workspace commits 546 `#[test]` functions across the
  three crates (count them: `grep -rc '#\[test\]' crates/`). `ROADMAP.md:44`
  records **353 workspace tests passing** at the Milestone 2 close (`1d329a5`);
  the M2-MOUSE and M3-5/6/7 merges since then each passed CI, which is where
  the remaining tests are witnessed. The suites include a bounded VT
  compatibility harness and two independent adversarial hostile-input suites
  (`ROADMAP.md:58-60`).
- **Four CI gates** required by branch protection (`ROADMAP.md:46-47`,
  `.github/workflows/`): the Rust workflow (`cargo fmt --check`, `cargo clippy
  --workspace --all-targets -- -D warnings`, `cargo test --workspace` on macOS
  arm64), the documentation validator (`scripts/check_docs.py`), `cargo deny
  check`, and an MSRV build.
- **A manual macOS check**, re-run at the M2 close head (`ROADMAP.md:62-65`):
  a release build opened a window, owned a direct `zsh` child whose tty
  reported the expected grid size, and on exit the child was reaped and the pty
  device was gone.

What this evidence does **not** cover (`ROADMAP.md:67-72`): there is no
rendered-frame oracle and no key injection into the real window, so **glyph
correctness and live input are unverified by automation** — the byte-level
input contract is tested, but no test has ever looked at a rendered frame. The
manual check above is a smoke test of the window→grid→PTY chain, not a
correctness proof of what appears on screen. Colour, IME, and accessibility are
absent from testing because they are absent from the build.

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
