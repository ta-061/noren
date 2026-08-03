# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo), lean Design Council integration on Draft
PR [#15](https://github.com/ta-061/noren/pull/15), branch
`docs/m1-lean-design`.

## Current phase

Milestone 1 — scoped gate review for the first macOS local-PTY PoC. Discovery is
complete and integrated. Production implementation is still closed until the
two bounded Issue #6 reviews are addressed and PR #15 merges. That merge may
open only FR-001 through FR-007; all deferred v0.1 features retain their gates.

## GitHub state

Verified on 2026-08-03:

- Closed Issues: [#1](https://github.com/ta-061/noren/issues/1),
  [#3](https://github.com/ta-061/noren/issues/3),
  [#4](https://github.com/ta-061/noren/issues/4),
  [#5](https://github.com/ta-061/noren/issues/5),
  [#8](https://github.com/ta-061/noren/issues/8), and
  [#9](https://github.com/ta-061/noren/issues/9).
- Open Issue: [#6](https://github.com/ta-061/noren/issues/6), lean Design
  Council and scoped Milestone 1 gate.
- Open PR: Draft [#15](https://github.com/ta-061/noren/pull/15), Issue #6.
- PR [#14](https://github.com/ta-061/noren/pull/14) merged Issue #8 as
  `f393bdc9`; `main` is at `f393bdc9537a8ac52b77d932b247bb1d3280ea2d`.
- PRs #2, #7, #10, #11, #12, #13, and #14 are merged. No Discovery PR remains
  open and no new large or duplicate Discovery review is planned.

## Issue #6 checkpoints

| Lane | Saved evidence | State |
| --- | --- | --- |
| GLM core proposal | Signed commit `02ae9d8b`, 898 words | Complete |
| Qwen minimum UI proposal | Signed commit `22a4307c`, 686 words | Complete |
| Codex integration | Requirements, architecture/crates/candidates, threat model, test strategy, ADR 0001/0002 | In progress |
| Claude Code security critique | PTY/process spawn/input boundary only | Pending |
| codex-lab test critique | PTY I/O/resize/requirements testability only | Pending |
| Fugu | Reserved for later SSH design | Not used |

The required integrated artifacts are
[v0.1 requirements](../requirements/v0.1.md),
[minimum architecture](../architecture/minimal-local-pty-poc.md),
[threat model](../security/threat-model.md),
[test strategy](../testing/strategy.md),
[ADR 0001](../adr/0001-rust-toolchain-and-msrv.md), and
[ADR 0002](../adr/0002-local-pty-poc-architecture.md).

## Toolchain and build state

Rust remains absent at this documentation checkpoint. ADR 0001 accepts the
exact Rust/MSRV 1.88.0 pin and required `aarch64-apple-darwin` target. Versioned
crates.io metadata shows the highest declared MSRV in the scoped direct set is
wgpu's 1.87; `portable-pty` and `swash` declare none. R-PORT-01 therefore remains
partly open until the first implementation Issue installs the pin, records
`rustc`/`cargo`/targets/host, commits `Cargo.toml` and the lockfile, and passes
minimal workspace CI.

## Active risks and cut line

The [risk register](../roadmap/risk-register.md) remains authoritative. The first
PoC directly exercises R-PORT-01, R-PTY-01, R-IN-01, R-SEC-01/R-SEC-02,
R-PERF-01, and R-TEST-01. Candidate libraries remain reversible; passing the
PoC is not permanent adoption or a Preview compatibility claim.

Prohibited in the first implementation Issue: SSH, AI-agent state UI, tabs,
panes, themes, remote daemon, persistence, production IME/accessibility, and
broad Linux/cross-platform work.

## Human decisions still required

No repository access control was changed. The owner still must separately
decide branch protection/required CI/merge policy, macOS signing/notarization
identity, and the public support/security contact before Preview publication.

## Next steps

1. Run exactly one bounded Claude security critique and one bounded codex-lab
   PTY/testability critique on the integrated Draft PR; save both results.
2. Codex addresses non-duplicate blockers, records the scoped gate decision,
   and merges PR #15 if current-head documentation CI is green.
3. Create one implementation Issue with the seven PoC steps and explicit
   non-goals, then assign GLM core, Qwen UI, codex-lab tests, Claude review.
4. The first implementation Draft PR must include root `Cargo.toml`, pinned
   toolchain, initial crates, CI, exact version/target evidence, and unfinished
   handoff items before any model limit.

## Production gate

Closed at this checkpoint. It opens only for the first local macOS PTY PoC when
Issue #6 review is addressed and PR #15 merges. Deferred v0.1 features do not
inherit that approval.
