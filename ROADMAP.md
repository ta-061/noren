# Roadmap

Status terms: **Not started**, **In progress**, **Gate review**, **Complete**.
Only evidence-backed work is marked complete.

| Milestone | Scope | Status |
| --- | --- | --- |
| 0 — Discovery | Landscape, feature/library matrices, risks, agent inventory and calibration | Complete |
| 1 — Requirements and design | Independent proposals, critiques, integrated requirements, architecture, threat model, tests, RFCs, ADRs | Complete |
| 2 — Terminal foundation | Window, PTY, shell, terminal state/rendering, input, resize, scrollback, selection, copy/paste/search, configuration and diagnostics | Complete |
| 3 — Workspace | External workspace management (sidebar: projects, git worktrees, SSH connections, agents, terminal sessions), single-session view, session lifecycle, sidebar-state persistence, palette, configurable keybindings, Zellij pass-through — no native tabs/panes/layout (delegated to Zellij per [ADR 0003](docs/adr/0003-noren-zellij-responsibility-boundary.md)) | Not started |
| 4 — SSH and remote | OpenSSH configuration, connections, reconnect, remote panes, daemon decision/PoC and recovery | Not started |
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

**What this does not establish.** There is still no rendered-frame oracle and no key
injection into the real window, so glyph correctness and live input remain unverified
by automation; the byte-level input contract is covered by tests instead. Mouse
reporting is unimplemented and, per Issue #46, belongs in an input encoder rather than
output-side parsing. Truecolor is modelled in terminal state but not yet wired to
drawing. IME and accessibility remain deferred.

No milestone date is promised. Implementation advances through scoped Issues,
Draft PRs, and current-head CI evidence.

## What blocks a public preview

Two independent specification reviews, run without sight of each other, both
concluded that the current tree cannot honestly be released as "0.1.0-preview of
the Noren terminal." The reasoning and the decision are recorded in
[D-M8-001](docs/coordination/decisions/D-M8-001-preview-scope.md). In short:

- **The workspace is not on `main`.** Noren's defining feature is the external
  sidebar (ADR 0003). Of the Milestone 3 modules, only `session.rs` has landed;
  the sidebar, palette, pass-through, supervisor, and persistence exist only in
  unmerged PRs. What `main` contains today is a terminal foundation.
- **The renderer is monochrome.** `renderer.rs:35` returns a constant colour and
  the vertex layout carries no colour channel, so `ls --color`, `vim`, and
  Zellij's status bar all draw in one shade. Truecolor is modelled in terminal
  state and never reaches drawing.
- **The font is ASCII-only and case-blind.** Non-ASCII renders as `?`, and
  `renderer.rs:457` asserts `glyph_rows('a') == glyph_rows('A')`.
- **FR-005's rendered-frame oracle does not exist**, so glyph correctness has
  never been mechanically verified. Waiving the project's own PoC gate in silence
  is the one path that would make a release claim dishonest.
- **NFR-009 requires release-integrity gates** — signing, notarization,
  packaging — to pass before any Preview claim.

Milestone 8 therefore stops at a release candidate. Signing keys, Apple
certificates, tagging, and publication are owner decisions and are not taken
autonomously.
