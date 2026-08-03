# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo), implementation/test head
`c1f66dc27ddce37a60665d319a7ca061c300947e` on Draft PR
[#17](https://github.com/ta-061/noren/pull/17), branch
`feat/macos-local-pty-poc`. This coordination-only update follows that tested
head.

## Current phase

Milestone 1 — first macOS local-zsh PTY PoC implementation. Discovery and the
lean Design Council are merged. The Rust workspace, CI, local PTY, window,
input adapter, terminal state, bounded GPU view, resize path, and shutdown path
now exist on one Draft PR. The PR stays Draft until the remaining local rendered
frame and real window-interaction checkpoint is saved; deferred product features
remain closed.

## GitHub state

Verified on 2026-08-03:

- The only open Issue is [#16](https://github.com/ta-061/noren/issues/16), the
  scoped macOS local-zsh PTY PoC.
- The only open PR is Draft [#17](https://github.com/ta-061/noren/pull/17),
  `feat/macos-local-pty-poc` into `main`; GitHub reports the merge state clean.
- Issues [#6](https://github.com/ta-061/noren/issues/6) and
  [#8](https://github.com/ta-061/noren/issues/8) are closed. PR
  [#14](https://github.com/ta-061/noren/pull/14) merged the bounded Discovery
  integration and PR [#15](https://github.com/ta-061/noren/pull/15) merged the
  lean Design Council.
- `main` is at `54ed3cfc9abaab97bd45ad8dedac71070832b54e`, the merge commit for
  PR #15. No Discovery PR remains open and no repeated large research or review
  is planned.

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
tests pass, the PTY crate passes five consecutive stress runs, the documentation
validator and its seven tests pass, and both GitHub checks pass. The current
GitHub Actions runs are `30783344844` (Rust) and `30783344827` (documentation).

## Toolchain and candidate status

- `rustc 1.88.0 (6b00bc388 2025-06-23)`, `cargo 1.88.0
  (873a06493 2025-05-10)`, active/installed target
  `aarch64-apple-darwin`; local host arm64 on macOS 26.4.1.
- Direct implementation candidates are exact `portable-pty 0.9.0`, `avt
  0.18.0`, `unicode-width 0.2.2`, `winit 0.30.13`, and `wgpu 30.0.0` with
  Metal/WGSL features. The first view uses a bounded built-in ASCII raster
  fallback; `swash 0.2.10` remains an accepted but unmeasured replacement seam,
  not a claimed implementation dependency.
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
2. Re-run current-head CI after this coordination commit and update the Issue
   #16 / PR #17 checklists to the actual evidence.
3. Mark ready and merge only after the scoped rendered-frame/window checkpoint;
   do not open deferred feature work in this PR.
