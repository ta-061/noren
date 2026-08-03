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
Scrolling regions are in progress as Draft PR
[#21](https://github.com/ta-061/noren/pull/21), and the stacked alternate
screen is Draft PR [#23](https://github.com/ta-061/noren/pull/23) for Issue
[#22](https://github.com/ta-061/noren/issues/22); neither Draft PR is merged.

Compatibility development priority is vim first, then tmux and zellij, then
SSH, then agent integration. The compatibility slices are implemented in
complete Draft PRs that all remain Draft and review waiting; none is merged
and none is a vim/tmux/zellij compatibility claim: erase and insert/delete
operations (Issue [#24](https://github.com/ta-061/noren/issues/24), Draft PR
[#31](https://github.com/ta-061/noren/pull/31) at
`a630c93605e309c2fd23558c8807500ac12a684e`); SGR and cell attributes (Issue
[#25](https://github.com/ta-061/noren/issues/25), Draft PR
[#29](https://github.com/ta-061/noren/pull/29) stacked on
`agent/terminal-erase-ops`); application cursor/keypad modes (Issue
[#26](https://github.com/ta-061/noren/issues/26), Draft PR
[#30](https://github.com/ta-061/noren/pull/30) stacked on
`agent/terminal-sgr-attributes`); and a bounded current-core VT compatibility
harness (Issue [#27](https://github.com/ta-061/noren/issues/27), Draft PR
[#32](https://github.com/ta-061/noren/pull/32)). The central parser/state
file lease sequence #24 -> #25 -> #26 is complete and released. Issue
[#28](https://github.com/ta-061/noren/issues/28) and Draft PR
[#33](https://github.com/ta-061/noren/pull/33) document the parallel development
model behind these lanes.

Tabs, origin mode, query/reply, Unicode/IME, and the later milestone scope
remain deferred. No milestone date is promised. Implementation advances
through scoped Issues, Draft PRs, and current-head CI evidence.
