# Coordination status

Last updated: 2026-08-07 (Asia/Tokyo). The product-code baseline on `main` is
`1d329a5` and passes **353 workspace tests**. Historical sections below preserve
the earlier PR #17/#19 records at their own heads, corrected only where an
active claim went stale.

## Current phase

Milestone 2 terminal foundation is complete. On `main`: window and local zsh
PTY, renderer-independent terminal state, scrolling regions, alternate screen
and mode state, erase/edit operations, SGR and cell attributes, application
cursor/keypad modes, escape-intermediate and string-sequence handling, DECSTBM
clamping, bounded scrollback, Unicode/CJK display width, complete staged key
encoding, selection with clipboard copy/paste, scrollback search, configuration,
and diagnostics. See
[terminal core foundation](../architecture/terminal-core-foundation.md).

Quality gates: 353 workspace tests, a VT compatibility harness, two independent
adversarial hostile-input suites, plus `cargo deny` and MSRV verification in CI.

This is **not** a VT100/xterm or vim/tmux/zellij compatibility claim. The largest
outstanding presentation/input gaps are a rendered-frame oracle, mouse input,
truecolor drawing, IME, and accessibility; the ranked, evidence-based list is
the [Zellij gap analysis](../compatibility/zellij-gap-analysis.md). Known
non-conformance is recorded in [reviews](reviews/) and under
[accepted follow-ups](#accepted-follow-ups) below.

Development runs through parallel AI coding lanes with independent verification;
the rules that apply to every change are in
[development model](development-model.md).

## GitHub state

Verified on 2026-08-07 with `gh pr list` and `gh issue list`:

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
- PR [#32](https://github.com/ta-061/noren/pull/32) (VT compatibility harness,
  Issue [#27](https://github.com/ta-061/noren/issues/27)) is merged as
  `aa41530` after being rebased. PR
  [#33](https://github.com/ta-061/noren/pull/33) was **closed rather than
  merged**: its text described every stack PR as an unmerged Draft that is "not
  yet supported", which became false when the stack landed. Its durable content
  was carried into PR [#34](https://github.com/ta-061/noren/pull/34) instead.
  Both branches predated the stack landing, and their unrebased diffs would have
  deleted the erase-ops, SGR, and application-mode test files.
- Subsequent merges on `main`. The audit workflow did not exist until PR
  #44 introduced it, so only #45 onward could have run `cargo deny` and MSRV;
  earlier entries passed the two checks that existed at the time:
  [#34](https://github.com/ta-061/noren/pull/34) fleet contract and status
  correction; [#38](https://github.com/ta-061/noren/pull/38) renderer/PTY grid
  agreement (Issue #35); [#39](https://github.com/ta-061/noren/pull/39)
  adversarial hostile-input suite;
  [#40](https://github.com/ta-061/noren/pull/40) and
  [#48](https://github.com/ta-061/noren/pull/48) key encoding stages 1-3 of
  Issue [#36](https://github.com/ta-061/noren/issues/36);
  [#42](https://github.com/ta-061/noren/pull/42) fleet scaling and the Kimi
  sweep; [#43](https://github.com/ta-061/noren/pull/43) string sequences and
  private CSI markers (Issue #41);
  [#44](https://github.com/ta-061/noren/pull/44) dependency audit, license
  policy, and MSRV verification; [#45](https://github.com/ta-061/noren/pull/45)
  DECSTBM clamping and C0 inside CSI (Issue #37);
  [#47](https://github.com/ta-061/noren/pull/47) Zellij gap analysis;
  [#49](https://github.com/ta-061/noren/pull/49) lane stall diagnosis;
  [#50](https://github.com/ta-061/noren/pull/50) bounded scrollback;
  [#53](https://github.com/ta-061/noren/pull/53) Unicode/CJK display width;
  [#56](https://github.com/ta-061/noren/pull/56) indexed/truecolor terminal
  state; [#60](https://github.com/ta-061/noren/pull/60) modifier parameters;
  [#62](https://github.com/ta-061/noren/pull/62) wide-cell edge handling;
  [#64](https://github.com/ta-061/noren/pull/64) the per-cell grapheme cap;
  [#65](https://github.com/ta-061/noren/pull/65) scrollback search;
  [#66](https://github.com/ta-061/noren/pull/66) selection and clipboard; and
  [#67](https://github.com/ta-061/noren/pull/67) configuration and diagnostics.
- Issues #22, #24, #26, #27, #28, #35, #36, #37, and #41 are closed as
  delivered. Issue [#46](https://github.com/ta-061/noren/issues/46) is also
  closed after review showed its output-channel reproduction was valid Delete
  Line behavior, not an X10 input report; mouse input remains unimplemented.
- CI runs four checks per PR: the Rust build/lint/test job, the documentation
  validator, `cargo deny check`, and MSRV verification. These became *required*
  when branch protection was enabled on 2026-08-05 (see [resolved
  decisions](#human-decisions--resolved-2026-08-05)); before that they ran
  automatically but did not block a merge.
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

Every finding from the stack reviews, with its current disposition. Findings are
tracked to closure rather than left implicit.

| Finding | Severity | Source | State |
| --- | --- | --- | --- |
| Renderer clamped the drawn grid to 160x60 while the PTY/terminal grid was capped only at `u16::MAX` | MAJOR | [Qwen](reviews/terminal-stack-qwen.md) | Fixed, PR #38 (Issue #35) |
| Delete/navigation/function keys, Alt+char, and Ctrl+named keys were silently dropped | MAJOR | [Qwen](reviews/terminal-stack-qwen.md) | Fixed, PRs #40 and #48 |
| Shift and general modifier parameters were not encoded, so Shift+Arrow was indistinguishable from a bare arrow | MAJOR | [Qwen](reviews/terminal-stack-qwen.md) | Fixed, PR #60 (Issue #36 closed) |
| DECSTBM rejected an out-of-range bottom margin instead of clamping | MINOR | [GLM](reviews/terminal-stack-glm.md) | Fixed, PR #45 (Issue #37) |
| C0 controls embedded inside a CSI were swallowed rather than executed | MINOR | [GLM](reviews/terminal-stack-glm.md) | Fixed, PR #45 (Issue #37) |
| DCS/SOS/PM/APC payloads rendered as screen text, allowing a program to spoof screen content | MAJOR | [Kimi](reviews/terminal-stack-kimi.md) | Fixed, PR #43 (Issue #41) |
| CSI private markers `<` and `=` executed as destructive plain CSI | MAJOR | [Kimi](reviews/terminal-stack-kimi.md) | Fixed, PR #43 (Issue #41) |
| Legacy X10 `CSI M` "misparse" on the output channel | — | [Zellij gap analysis](../compatibility/zellij-gap-analysis.md) | **Not a bug** — Issue #46 and PR #52 closed; see below |
| Unicode/CJK character width was not modeled, so wide characters misaligned the grid | MAJOR | [Zellij gap analysis](../compatibility/zellij-gap-analysis.md) | Fixed, PR #53 |
| No mouse support: modes untracked and no pointer events reach the PTY | MAJOR | [Zellij gap analysis](../compatibility/zellij-gap-analysis.md) | **Open** — no Issue yet |

The X10 `CSI M` item was withdrawn after review. `TerminalState::feed_bytes`
parses PTY **output**, where parameterless `CSI M` is the valid Delete Line
command; an X10 mouse report shares the prefix but travels the other way
(terminal to PTY **input**), and no mouse input path exists at all. The
coordinator's original reproduction was a misreading: the deleted line was DL
working correctly, and the following bytes printed as text because on that
channel they *are* text. PR #52 would have made a legitimate DL discard itself
plus three output bytes; it was closed before merge. Mouse-mode tracking remains
useful for a future mouse *input* encoder and will be re-proposed on that basis.

An adversarial hostile-input sweep now exists (PR #39, 23 tests) and was
mutation-tested to confirm it detects breakage rather than passing vacuously. An
independent second sweep from the Kimi lane (PR #43's provenance) found two real
parser defects the first sweep had reported clean, which is why both are kept.

IME, non-ASCII glyph quality beyond modeled display width, truecolor rendering,
and mouse input remain outstanding, and no compatibility claim is made for any
of them.

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

## Outstanding gate

Every branch must be rebased onto `main` before merge, with its diff confirmed to
delete no landed test file. This is not a formality: two branches cut before a
sibling's tests existed proposed deleting roughly 1,800 lines of passing tests,
and `mergeable=MERGEABLE`/`CLEAN`/green CI does **not** catch it — a clean delete
is still a clean merge. The check is `git diff --stat origin/main...HEAD`.

The macOS smoke check is **done**. Re-run on 2026-08-07 against the product-code
baseline `1d329a5` on macOS Apple Silicon, from a release build (`cargo build --release -p
noren-app`, finished clean):

| Step | Observed evidence |
| --- | --- |
| Open a window | Process alive after launch with 7 threads (supervisor and reader present) |
| Start local zsh PTY | Owned child `zsh` present as a direct child of the app |
| Window -> grid -> PTY geometry | The child's tty reported `30 90` via `stty size`, exactly the 900x600 window divided by the 10x20 cell, so the whole chain agrees |
| Exit and clean up | On termination the app exited, the `zsh` child was reaped, and the pty device itself was gone — no orphan process and no leaked descriptor |

What this does **not** establish: there is still no rendered-frame oracle and no
real key-injection into the window, so glyph correctness and live input remain
unverified by automation. The byte-level input contract is covered by tests
instead. Treat the smoke check as evidence the process/PTY/geometry chain works,
not as evidence the renderer draws correctly.

Full VT/xterm behavior, non-ASCII glyph quality, swash/font trials, production
IME/accessibility, Linux, SSH, agent integration, tabs, panes, themes, and a
remote daemon remain deferred behind their existing risk gates.

## Human decisions — resolved 2026-08-05

The owner decided all four outstanding items:

- **Branch protection: enabled** on `main`, requiring all four checks (Rust
  build/lint/test, documentation validator, `cargo deny check`, MSRV
  verification), with force-pushes and branch deletion blocked and
  `strict` (up-to-date-before-merge) on. `enforce_admins` is deliberately left
  off so the owner retains an emergency path. Conversation resolution is required.
- **Branch cleanup: approved.** 27 fully-merged remote branches were deleted
  after verifying `git rev-list --count origin/main..<branch>` was 0 for each.
  Three were kept because they carry unmerged commits.
- **Signing and notarization: deferred** until immediately before Preview
  binaries are distributed to anyone else. No signing identity is configured, and
  none is claimed.
- **Security contact: GitHub private vulnerability reporting** is the official
  channel. Verified enabled on the repository, so the policy in
  [`SECURITY.md`](../../SECURITY.md) is backed by a working intake rather than
  being aspirational.

## Next steps

1. Begin Milestone 3 with the external-session boundary recorded in ADR 0003;
   do not introduce native tabs, panes, splits, or a layout model.
2. Specify and implement mouse input as an application-side encoder without
   reintroducing the withdrawn Issue #46 output-parser premise.
3. Build a rendered-frame oracle so glyph correctness stops depending on a human
   looking at the window. The smoke check covers the process/PTY/geometry chain
   but cannot see what is drawn.
4. Wire modeled indexed/truecolor state into drawing and retain the existing
   byte- and state-level regression coverage.
