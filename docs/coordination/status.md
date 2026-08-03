# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo). PR
[#19](https://github.com/ta-061/noren/pull/19) is merged into `main` as
`c695920d8bc99990447d0b451754ea96c91181fc`. The only open PR is Draft
[#21](https://github.com/ta-061/noren/pull/21), branch
`agent/terminal-scroll-regions`, for Issue
[#20](https://github.com/ta-061/noren/issues/20). Historical sections below
preserve the PR #17 record at implementation/test head
`c1f66dc27ddce37a60665d319a7ca061c300947e`, corrected only where an active
claim went stale; coordination head `ac410a82` also passed both GitHub checks.

## Current phase

Terminal foundation, scrolling-region slice. PR #19 merged the Noren-owned,
renderer-independent `TerminalState`. Draft PR #21 adds DECSTBM margins,
region-scoped LF/VT/FF/IND/NEL/RI and CSI S/T scrolling, CNL/CPL/VPA, delayed
autowrap, and resize-reset behavior. See
[terminal core foundation](../architecture/terminal-core-foundation.md). This
is not a VT100/xterm or vim/tmux/zellij compatibility claim.

## GitHub state

Verified on 2026-08-03:

- The only open Issue is [#20](https://github.com/ta-061/noren/issues/20), the
  scrolling-region behavior slice. The only open PR is its Draft
  [#21](https://github.com/ta-061/noren/pull/21).
- PR [#19](https://github.com/ta-061/noren/pull/19) and Issue
  [#18](https://github.com/ta-061/noren/issues/18) are complete after owner
  acceptance of the renderer-independent Terminal Core foundation.
- PR [#17](https://github.com/ta-061/noren/pull/17) and its Issue
  [#16](https://github.com/ta-061/noren/issues/16) are complete after the owner
  accepted the macOS local-PTY PoC.
- Issues [#6](https://github.com/ta-061/noren/issues/6) and
  [#8](https://github.com/ta-061/noren/issues/8) are closed. PR
  [#14](https://github.com/ta-061/noren/pull/14) merged the bounded Discovery
  integration and PR [#15](https://github.com/ta-061/noren/pull/15) merged the
  lean Design Council.
- `main` is at `c695920d8bc99990447d0b451754ea96c91181fc`, the merge commit
  for PR #19. No Discovery PR remains open and no repeated large research or
  review is planned.

## Merged PR #19 Terminal Core handoff

| Lane | Saved evidence | State |
| --- | --- | --- |
| Codex core | Signed `d62b36e`: Noren-owned state/parser and renderer snapshot integration | Complete |
| codex-lab tests | Signed `053216f`: seven public-API state/cursor/buffer regression tests | Complete |
| GLM boundaries | PR #19 review: module, error, and crate boundaries pass without changes | Complete |
| Claude Code architecture | PR #19 review: ACCEPT, no VT-evolution blocker | Complete |
| Qwen documentation | Signed `b390531`: architecture, roadmap, contributor, and status updates | Complete |

## Draft PR #21 scrolling-region handoff

| Lane | Saved evidence | State |
| --- | --- | --- |
| Codex core | Signed `9ffe79c`: margins, region scrolling, cursor/index behavior, delayed autowrap | Complete |
| codex-lab tests | Signed `6bbac87`: eight public-API scroll/cursor/resize regression tests | Complete |
| GLM helper | Signed `32ca7da`: behavior-preserving parser-state idiom cleanup | Complete |
| Claude Code review | PR #21 review: `BLOCKER: NONE` | Complete |
| Qwen documentation | Signed `328c013`: architecture, roadmap, and contributor guidance | Complete |
| Codex integration | Exact-head local checks, CI, and launch smoke | In progress |

## PR #17 historical implementation checkpoints

| Lane | Saved evidence | State |
| --- | --- | --- |
| GLM core | Signed baseline `e0cc031b`: root workspace, pinned toolchain, three crates, lockfile, and CI | Complete |
| GLM PTY continuation | Two bounded attempts ended before editing; the clean no-diff handoff is recorded on PR #17 | Stopped to conserve limits |
| Qwen UI | Two bounded attempts ended before editing; the clean no-diff handoff is recorded on PR #17 | Stopped to conserve limits |
| Codex integration fallback | Signed `9c912c38`, `d6d15c93`, `e44d946d`, and `2cdd64ba`: fixed zsh PTY, single-deadline teardown, winit/wgpu UI, streaming terminal input, and crate-owned launch policy | Complete |
| codex-lab PTY tests | Signed `c1f66dc2`: partial I/O and UTF-8, real `stty size` final-resize oracle, output-pressure shutdown; five repeated PTY runs | Complete |
| Claude Code security gate | Read-only review at `c1f66dc2`: `SECURITY_GATE_CLEAN`, zero BLOCKER/MAJOR findings | Complete |
| Fugu | Reserved for later SSH design | Not used |

The stopped GLM/Qwen lanes left no uncommitted or competing implementation.
Codex performed only the minimum fallback integration on the same Draft PR;
codex-lab and Claude reviewed distinct concerns.

## Merged PR #17 behavior and evidence

| Required step | Evidence | State |
| --- | --- | --- |
| Rust workspace and CI | Rust/MSRV 1.88.0, edition 2024, resolver 3, exact lockfile, macOS arm64 workflow | Implemented |
| Open a window | `winit` `ApplicationHandler` creates one 900×600 resizable window; a local app run remained in the AppKit event loop | Implemented; capture pending |
| Start local zsh PTY | Fixed `/bin/zsh`, no caller argv or `-c`; local run observed the owned child plus supervisor/reader | Implemented and exercised |
| Pass key input | Pure byte-contract tests cover printable UTF-8, Enter, Backspace, Tab, Escape, arrows, Ctrl, repeats, releases, and unsupported modifiers | Implemented; real window injection pending |
| Display PTY output | Live PTY exact-marker tests, streaming UTF-8 tests, bounded AVT snapshots, wgpu glyph-batch tests, and local Metal initialization | Implemented; rendered-frame oracle pending |
| Propagate resize | Duplicate/zero geometry tests plus live duplicate/storm resize with final `37×113` confirmed by `/bin/stty size` | Implemented and exercised |
| Exit and clean up | Child kill/reap ownership, EOF/exit events, idempotent single 2 s deadline, and saturated-output close test | Implemented and exercised; real close-button injection pending |

At `c1f66dc2`, local formatting and Clippy with warnings denied pass, 31 Rust
tests pass, the PTY crate passes five consecutive stress runs, and the
documentation validator and its seven tests pass. Coordination head `ac410a82`
then passed both GitHub checks: runs `30783618745` (Rust) and `30783618743`
(documentation).

## Toolchain and candidate status

- `rustc 1.88.0 (6b00bc388 2025-06-23)`, `cargo 1.88.0
  (873a06493 2025-05-10)`, active/installed target
  `aarch64-apple-darwin`; local host arm64 on macOS 26.4.1.
- At this head, direct implementation candidates were exact `portable-pty
  0.9.0`, `avt 0.18.0`, `unicode-width 0.2.2`, `winit 0.30.13`, and `wgpu
  30.0.0` with Metal/WGSL features. On branch `agent/terminal-core-foundation`,
  `avt` is removed and the terminal state is Noren-owned. The first view uses a
  bounded built-in ASCII raster fallback; `swash 0.2.10` remains an accepted
  but unmeasured replacement seam, not a claimed implementation dependency.
- No async runtime, SSH, agent-state UI, tabs, panes, themes, persistence, or
  remote daemon was added.

## Current Draft gate

PR #21 remains Draft until exact-head local checks and CI pass and the existing
local-zsh application launch is rechecked. PR #19 has no remaining gate. This
does not authorize deferred terminal features in the scrolling-region PR.

Full VT/xterm behavior, non-ASCII glyph quality, swash/font trials, production
IME/accessibility, Linux, SSH, agent integration, tabs, panes, themes, and a
remote daemon remain deferred behind their existing risk gates.

## Human decisions still required

No repository access control was changed. The owner still must separately
decide branch protection/required CI/merge policy, macOS signing/notarization
identity, and the public support/security contact before Preview publication.

## Next steps

1. Save exact-head local checks and GitHub CI on Draft PR #21.
2. Repeat the bounded macOS launch smoke without changing OS security settings.
3. Keep alternate screen, SGR/erase, attributes, modes, and later VT work in
   separate Issues and PRs.
