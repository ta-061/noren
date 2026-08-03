# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo). PR
[#19](https://github.com/ta-061/noren/pull/19) is merged into `main` as
`c695920d8bc99990447d0b451754ea96c91181fc`. Draft PR
[#21](https://github.com/ta-061/noren/pull/21) remains in review for Issue
[#20](https://github.com/ta-061/noren/issues/20). Draft PR
[#23](https://github.com/ta-061/noren/pull/23), branch
`agent/terminal-alternate-screen`, is stacked on #21 for Issue
[#22](https://github.com/ta-061/noren/issues/22). Compatibility Issues
[#24](https://github.com/ta-061/noren/issues/24),
[#25](https://github.com/ta-061/noren/issues/25),
[#26](https://github.com/ta-061/noren/issues/26), and
[#27](https://github.com/ta-061/noren/issues/27) now have complete Draft PRs
[#31](https://github.com/ta-061/noren/pull/31),
[#29](https://github.com/ta-061/noren/pull/29),
[#30](https://github.com/ta-061/noren/pull/30), and
[#32](https://github.com/ta-061/noren/pull/32); PRs #29–#33 all remain Draft
and review waiting, and none is merged. Issue
[#28](https://github.com/ta-061/noren/issues/28) and Draft PR
[#33](https://github.com/ta-061/noren/pull/33) document the parallel development
model. Historical sections below
preserve the PR #17 record at implementation/test head
`c1f66dc27ddce37a60665d319a7ca061c300947e`, corrected only where an active
claim went stale; coordination head `ac410a82` also passed both GitHub checks.

## Current phase

Terminal foundation, alternate-screen slice. PR #19 merged the Noren-owned,
renderer-independent `TerminalState`; Draft PR #21 adds scrolling regions.
Stacked Draft PR #23 adds primary/alternate screen ownership, DEC private mode
1049, cursor save/restore, mode snapshots, and both-buffer resize behavior. See
[terminal core foundation](../architecture/terminal-core-foundation.md). This
is not a VT100/xterm or vim/tmux/zellij compatibility claim.

Compatibility lanes follow the roadmap priority — vim first, then tmux/zellij,
then SSH, then agent integration — under the parallel development model in
[CONTRIBUTING.md](../../CONTRIBUTING.md): isolated worktrees, non-overlapping
file leases, stacked Draft PRs, exact-head CI, DCO signoff, and checkpoint
handoffs.

## GitHub state

Verified on 2026-08-03:

- The open Issues are [#20](https://github.com/ta-061/noren/issues/20), the
  scrolling-region behavior slice, and stacked alternate-screen Issue
  [#22](https://github.com/ta-061/noren/issues/22). Their open Draft PRs are
  [#21](https://github.com/ta-061/noren/pull/21) and
  [#23](https://github.com/ta-061/noren/pull/23), respectively.
- Compatibility Issues [#24](https://github.com/ta-061/noren/issues/24)
  (erase/insert/delete), [#25](https://github.com/ta-061/noren/issues/25)
  (SGR and cell attributes),
  [#26](https://github.com/ta-061/noren/issues/26) (application cursor/keypad
  modes), and [#27](https://github.com/ta-061/noren/issues/27) (bounded VT
  compatibility test suite) each have a complete Draft PR, all remaining Draft
  and review waiting:
  - [#31](https://github.com/ta-061/noren/pull/31) for #24 at
    `a630c93605e309c2fd23558c8807500ac12a684e`, with exact-head macOS and docs
    CI green.
  - [#29](https://github.com/ta-061/noren/pull/29) for #25 at
    `0daa7d6aff2dbcdc547358288346a9804fa35011`, stacked on branch
    `agent/terminal-erase-ops`; both CI green and Claude `BLOCKER: NONE`.
  - [#30](https://github.com/ta-061/noren/pull/30) for #26 at
    `fd1ea69584acbfdf2d0c08debbd148989f3f9f6b`, stacked on
    `agent/terminal-sgr-attributes`; 96 local tests, both CI green, and Claude
    `BLOCKER: NONE`.
  - [#32](https://github.com/ta-061/noren/pull/32) for #27 at
    `c03e8b30ec82597b32b597b7b8961c30d61c6556`; both CI green and Claude
    `BLOCKER: NONE`.

  The central parser/state file lease sequence #24 -> #25 -> #26 is complete
  and released; none of these PRs is merged and none is a vim/tmux/zellij
  compatibility claim. Issue [#28](https://github.com/ta-061/noren/issues/28)
  and Draft PR [#33](https://github.com/ta-061/noren/pull/33) track the
  documentation of the parallel development model.
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
| Codex integration | Exact-head local checks, CI, and bounded local-zsh launch smoke | Complete |

## Draft PR #23 alternate-screen handoff

| Lane | Saved evidence | State |
| --- | --- | --- |
| Codex core | Signed `84da736`: screen ownership, mode 1049, cursor save/restore, and resize behavior | Complete |
| codex-lab tests | Signed `454d995`: seven public-API alternate/mode/cursor/resize tests | Complete |
| GLM helper | PR #23 review: parser/enum/module organization is clean; no change needed | Complete |
| Claude Code review | PR #23 review: `BLOCKER: NONE` | Complete |
| Qwen documentation | Signed `29f7362`: focused terminal-core architecture update | Complete |
| Codex integration | Exact-head local checks, CI, and bounded local-zsh launch smoke | Complete |

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

PR #21 remains Draft for owner review and acceptance. Stacked PR #23 has passed
local checks and CI, and the existing local-zsh application launch was
rechecked; it remains Draft and based on `agent/terminal-scroll-regions` until
#21 is accepted and merged. PR #19 has no remaining gate. This does not
authorize deferred terminal features in either Draft PR. Compatibility Issues
#24–#27 started only from the exact PR #23 head and did not modify Draft PRs
#21 or #23; their central parser/state file lease sequence #24 -> #25 -> #26
is complete and released, and Draft PRs #29–#32 are complete, review waiting,
and unmerged. Draft PR #33 is this documentation update and also remains Draft.

Full VT/xterm behavior, non-ASCII glyph quality, swash/font trials, production
IME/accessibility, Linux, SSH, agent integration, tabs, panes, themes, and a
remote daemon remain deferred behind their existing risk gates.

## Human decisions still required

No repository access control was changed. The owner still must separately
decide branch protection/required CI/merge policy, macOS signing/notarization
identity, and the public support/security contact before Preview publication.

## Next steps

1. Review and accept Draft PR #21, then promote and merge its exact tested head.
2. Retarget Draft PR #23 to `main` only after #21 merges, then review its exact
   tested head.
3. Review the complete compatibility Draft PRs in stack order — #31, then #29
   (stacked on `agent/terminal-erase-ops`), then #30 (stacked on
   `agent/terminal-sgr-attributes`), plus #32 — keeping erase, SGR,
   application modes, the compatibility test suite, and later VT work in
   separate Issues and PRs.
4. No central parser/state file lease queue remains: the #24 -> #25 -> #26
   lease sequence is complete and released. Review Draft PR #33 (Issue #28)
   as the documentation record of this parallel model.
