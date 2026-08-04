# Roadmap

Status terms: **Not started**, **In progress**, **Gate review**, **Complete**.
Only evidence-backed work is marked complete.

| Milestone | Scope | Status |
| --- | --- | --- |
| 0 — Discovery | Landscape, feature/library matrices, risks, agent inventory and calibration | Complete |
| 1 — Requirements and design | Independent proposals, critiques, integrated requirements, architecture, threat model, tests, RFCs, ADRs | Complete |
| 2 — Terminal foundation | Window, PTY, shell, terminal state/rendering, input, resize, scrollback, selection, copy/paste/search, configuration and diagnostics | In progress |
| 3 — Workspace | Tabs, panes, workspaces, persistence, sidebar, palette, configurable keybindings, Zellij pass-through | Not started |
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

This is not a VT100/xterm or vim/tmux/zellij compatibility claim. Known
non-conformance is tracked as Issues
[#35](https://github.com/ta-061/noren/issues/35) (renderer and PTY grids
disagree above 160x60), [#36](https://github.com/ta-061/noren/issues/36)
(Delete, navigation, function, and modifier keys are not encoded), and
[#37](https://github.com/ta-061/noren/issues/37) (DECSTBM clamping, embedded C0
in CSI). Origin mode and query/reply remain deferred, Unicode/CJK width and IME
remain later, and no hostile-input robustness claim is made yet.

No milestone date is promised. Implementation advances through scoped Issues,
Draft PRs, and current-head CI evidence.
