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
| 8 — Public Preview | Honest docs/site, binaries, checksums, release review, known limitations and `0.1.0-preview` | Not started |

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
contract is covered by tests instead. Mouse reporting is implemented as an input
encoder (PR #79, `crates/noren-app/src/mouse.rs`). Truecolor is modelled in
terminal state but not yet wired to drawing. IME and accessibility remain
deferred.

No milestone date is promised. Implementation advances through scoped Issues,
Draft PRs, and current-head CI evidence.
