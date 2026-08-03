# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo). PR
[#17](https://github.com/ta-061/noren/pull/17) is merged into `main` as
`b2bb87c20365dec5d1ab6b0122b2a987ec768744`. The only open PR is Draft
[#19](https://github.com/ta-061/noren/pull/19), branch
`agent/terminal-core-foundation`. The sections below preserve the PR #17
record at implementation/test head
`c1f66dc27ddce37a60665d319a7ca061c300947e`, corrected only where an active
claim went stale; coordination head `ac410a82` also passed both GitHub checks.

## Current phase

Terminal foundation. The first macOS local-zsh PTY PoC (PR #17) is merged.
Draft PR #19, not merged, replaces the `avt` dependency with a
renderer-independent Noren `TerminalState`: bounded screen cells, cursor,
resize, printable ASCII/LF/CR/backspace, and minimal CSI cursor movement. The
renderer consumes `TerminalSnapshot`; see
[terminal core foundation](../architecture/terminal-core-foundation.md). This
is not VT100/xterm compatibility. Deferred order: scroll regions, alternate
screen, SGR/erase plus cell attributes, mode state; Unicode/IME remain later.

## GitHub state

Verified on 2026-08-03:

- The only open Issue is [#16](https://github.com/ta-061/noren/issues/16), the
  scoped macOS local-zsh PTY PoC.
- Draft [#17](https://github.com/ta-061/noren/pull/17),
  `feat/macos-local-pty-poc` into `main`, had a clean merge state and is now
  merged.
- Issues [#6](https://github.com/ta-061/noren/issues/6) and
  [#8](https://github.com/ta-061/noren/issues/8) are closed. PR
  [#14](https://github.com/ta-061/noren/pull/14) merged the bounded Discovery
  integration and PR [#15](https://github.com/ta-061/noren/pull/15) merged the
  lean Design Council.
- `main` was at `54ed3cfc9abaab97bd45ad8dedac71070832b54e`, the merge commit
  for PR #15, at this verification. No Discovery PR remains open and no
  repeated large research or review is planned.

## Implementation checkpoints

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

## Current PoC behavior and evidence

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

## Remaining scoped gate

Before making PR #17 ready, save one local macOS checkpoint that captures a
rendered frame containing a deterministic shell marker and exercises key input,
non-zero window resize, and the close action. The automated Computer Use path
timed out on local Accessibility/Screen Recording access; no OS permission or
security setting was changed. This is an evidence gap, not authorization to add
features or repeat architecture/security review.

Full VT/xterm behavior, non-ASCII glyph quality, swash/font trials, production
IME/accessibility, Linux, SSH, agent integration, tabs, panes, themes, and a
remote daemon remain deferred behind their existing risk gates.

## Human decisions still required

No repository access control was changed. The owner still must separately
decide branch protection/required CI/merge policy, macOS signing/notarization
identity, and the public support/security contact before Preview publication.

## Next steps

1. Capture the one remaining local window checkpoint without changing OS
   security settings; if unavailable, leave the PR Draft with this exact gap.
2. Save that evidence in Issue #16 and PR #17. If code changes, require new
   exact-head CI; otherwise preserve the already-passing implementation gates.
3. Mark ready and merge only after the scoped rendered-frame/window checkpoint;
   do not open deferred feature work in this PR.
