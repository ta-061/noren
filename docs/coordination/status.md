# Coordination status

Last updated: 2026-08-05 (Asia/Tokyo). `main` is at
`22c985e`, the merge commit for PR
[#29](https://github.com/ta-061/noren/pull/29), which landed the whole parallel
Terminal Core stack. Historical sections below preserve the earlier PR #17/#19
records at their own heads, corrected only where an active claim went stale.

## Current phase

Terminal foundation. The parallel Terminal Core stack is merged: scrolling
regions, alternate screen and mode state, erase/edit operations, SGR and cell
attributes, and application cursor/keypad modes are all on `main`, wired into
the key encoder. See
[terminal core foundation](../architecture/terminal-core-foundation.md).

This is **not** a VT100/xterm or vim/tmux/zellij compatibility claim. Known
non-conformance is recorded in [reviews](reviews/) and summarized under
[accepted follow-ups](#accepted-follow-ups) below.

Development now runs as an agent fleet; see [fleet](fleet.md) for lane
ownership, quota-gated dispatch, and failover.

## GitHub state

Verified on 2026-08-05 with `gh pr list` and `gh issue list`:

- PR [#29](https://github.com/ta-061/noren/pull/29) is merged as `22c985e` and
  subsumes PRs [#21](https://github.com/ta-061/noren/pull/21),
  [#23](https://github.com/ta-061/noren/pull/23),
  [#30](https://github.com/ta-061/noren/pull/30), and
  [#31](https://github.com/ta-061/noren/pull/31). #21 and #30 report merged;
  #23 and #31 were closed after verifying `git log origin/main..<branch>` was
  empty for each, so neither had unlanded commits.
- Issues [#22](https://github.com/ta-061/noren/issues/22),
  [#24](https://github.com/ta-061/noren/issues/24), and
  [#26](https://github.com/ta-061/noren/issues/26) are closed as delivered on
  `main`.
- The open PRs are [#32](https://github.com/ta-061/noren/pull/32) (bounded VT
  compatibility harness, Issue
  [#27](https://github.com/ta-061/noren/issues/27)) and
  [#33](https://github.com/ta-061/noren/pull/33) (parallel-delivery
  documentation, Issue [#28](https://github.com/ta-061/noren/issues/28)). Both
  predate the stack landing and **must be rebased onto `main` before merge**:
  their current diffs would delete the erase-ops, SGR, and application-mode
  test files that are now on `main`.
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
- No Discovery PR remains open and no repeated large research or review is
  planned.

## Merged Terminal Core stack evidence

Landed by PR #29 as `22c985e`. Verified at the merged head `c21a5a2`:
`cargo fmt --all --check` clean, `cargo clippy --workspace --all-targets
-- -D warnings` clean, and **108 workspace tests pass** (previously 96). Both
GitHub checks reported SUCCESS against `main` before merge.

| Lane | Saved evidence | State |
| --- | --- | --- |
| GLM core review | Found a BLOCKER in the escape state machine plus one MAJOR, two MINOR; [review](reviews/terminal-stack-glm.md) | Complete |
| GLM core fix | Signed `5e266d4`: `EscapeIntermediate` parser state and HT tab stops, with 12 regression tests | Complete |
| Qwen application review | 0 BLOCKER, 2 MAJOR, 2 MINOR on the window/renderer/input layer; [review](reviews/terminal-stack-qwen.md) | Complete |
| codex-lab merge mechanics | Simulated all merge steps in a disposable clone and compared tree OIDs; [plan](reviews/terminal-stack-merge-plan.md) | Complete |
| Coordinator verification | Independently reproduced the BLOCKER (`ESC ( B -> "B"`) and MAJOR (`a TAB b -> "ab"`), then re-verified the fix | Complete |

The BLOCKER is why the stack landed as one merge rather than seven bottom-up
merges: `ESC ( B` — the SCS charset sequence emitted by essentially every
terminfo-driven program — leaked a printable `B` onto the screen, and Horizontal
Tab was silently dropped. The fix exists only at the stack tip, so merging
bottom-up would have placed four knowingly output-corrupting commits on `main`.
Recorded as decision D-0011 in [decisions](decisions.md).

## Accepted follow-ups

Reviewed, reproduced where noted, and accepted as **not** blocking the stack.
None is closed; each needs its own Issue and evidence.

| Finding | Severity | Source |
| --- | --- | --- |
| Renderer clamps the drawn grid to 160x60 while the PTY/terminal grid is capped only at `u16::MAX`, so on a Retina display the PTY is told a geometry that is never drawn (confirmed by code reading) | MAJOR | [Qwen](reviews/terminal-stack-qwen.md) |
| Delete/Home/End/PageUp/PageDown/Insert/F1-F12, Alt+char, Ctrl+named keys, and Shift combinations are silently dropped instead of sending xterm bytes | MAJOR | [Qwen](reviews/terminal-stack-qwen.md) |
| DECSTBM rejects an out-of-range bottom margin instead of clamping it as xterm does | MINOR | [GLM](reviews/terminal-stack-glm.md) |
| C0 controls embedded inside a CSI are swallowed rather than executed | MINOR | [GLM](reviews/terminal-stack-glm.md) |

Unicode/CJK width, IME, non-ASCII glyph quality, and the adversarial
hostile-input sweep remain outstanding: the Kimi robustness lane was rate-limited
before it produced evidence, so no hostile-input claim is made.

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

Open PRs #32 and #33 remain Draft. Neither may merge until it is rebased onto
`main` and its diff is confirmed to delete no landed test file; both branched
before the erase-ops, SGR, and application-mode tests existed.

The owner has not yet repeated a manual macOS startup/output/resize/exit smoke
check against the merged stack. The stack's automated evidence is green, but no
rendered-frame or real-window-injection oracle exists, so the smoke check remains
the outstanding manual gate.

Full VT/xterm behavior, non-ASCII glyph quality, swash/font trials, production
IME/accessibility, Linux, SSH, agent integration, tabs, panes, themes, and a
remote daemon remain deferred behind their existing risk gates.

## Human decisions still required

No repository access control was changed. The owner still must separately
decide branch protection/required CI/merge policy, macOS signing/notarization
identity, and the public support/security contact before Preview publication.

## Next steps

1. Rebase PR #32 onto `main` so its only contribution is the VT compatibility
   harness, and confirm the diff deletes no landed test file.
2. Rebase PR #33 onto `main` and reconcile its documentation with this file.
3. Repeat the bounded macOS startup/output/resize/exit smoke check against the
   merged stack without changing OS security settings.
4. Open an Issue per accepted follow-up above; land the renderer/PTY geometry
   agreement and the missing key encodings as separate scoped PRs.
5. Re-run the adversarial hostile-input sweep once the Kimi lane's quota resets,
   since that evidence is still missing.
