# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo), Issue #8 compression checkpoint on
branch `docs/m0-risk-register` (Draft PR [#14](https://github.com/ta-061/noren/pull/14)).

## Current phase

Milestone 0 — Discovery, evidence integration. Production implementation has not
started, and the production gate remains closed (D-0001 in [decisions.md](decisions.md);
criteria in [design-process.md](design-process.md)) pending Issue
[#6](https://github.com/ta-061/noren/issues/6). No architecture, library, or dependency
selection is made here.

## Issues (verified against GitHub 2026-08-03)

Closed: [#1](https://github.com/ta-061/noren/issues/1) Discovery/governance baseline;
[#3](https://github.com/ta-061/noren/issues/3) terminal/library landscape;
[#4](https://github.com/ta-061/noren/issues/4) cmux parity and Zellij matrices;
[#5](https://github.com/ta-061/noren/issues/5) OpenSSH and CLI-agent evidence;
[#9](https://github.com/ta-061/noren/issues/9) OpenCode provenance correction.

Open: [#8](https://github.com/ta-061/noren/issues/8) (this Issue) — compressed risk
register drafted, independent reviews pending, integrator is Codex;
[#6](https://github.com/ta-061/noren/issues/6) — Milestone 1 proposals, critiques, and
integrated design; not started.

## Pull requests and CI (verified against GitHub 2026-08-03)

All six Discovery PRs are merged into `main`; the only open PR at this check is
Draft PR [#14](https://github.com/ta-061/noren/pull/14) for Issue #8.

| PR | Scope | Merge commit |
| --- | --- | --- |
| [#2](https://github.com/ta-061/noren/pull/2) | Discovery and governance baseline | `1288c909` |
| [#7](https://github.com/ta-061/noren/pull/7) | Design council execution protocol | `40588c90` |
| [#10](https://github.com/ta-061/noren/pull/10) | OpenCode provenance correction | `f76fbd46` |
| [#11](https://github.com/ta-061/noren/pull/11) | cmux parity and Zellij compatibility | `7049908a` |
| [#12](https://github.com/ta-061/noren/pull/12) | OpenSSH and agent integration evidence | `b37126cd` |
| [#13](https://github.com/ta-061/noren/pull/13) | Terminal and library landscape | `38db222d` |

`main` is at `b37126cd0f40c350f0ea4e28661aa7bdcd3dd3ac` (PR #12 merge).
CI: the `Documentation` workflow (`python3 scripts/check_docs.py` plus its unittest
suite) has succeeded after every merge into `main`
([workflow runs](https://github.com/ta-061/noren/actions)); Dependabot `github_actions`
updates succeed, while Dependabot `cargo` runs fail (no Cargo manifest or Rust
toolchain yet) — expected Discovery-stage state, tracked as R-PORT-01 in the risk
register.

## In progress

Issue #8 integration on `docs/m0-risk-register`: [risk-register.md](../roadmap/risk-register.md)
compressed to 12 evidence-linked rows covering all ten Issue #8 categories, plus the
disposition table for architecture-changing unknowns. The register is the required
shared input before Round 1 execution ([protocol-codex-lab.md](reviews/protocol-codex-lab.md)).

## Blocked

No Rust toolchain is installed, so compilation is impossible and every executable
gate in the risk register is blocked (R-PORT-01). Discovery, design, and
documentation work are not blocked.

## Risks

See [risk-register.md](../roadmap/risk-register.md). Highest-pair rows: R-IN-01 (input
loss, L3/I5), R-SEC-01/R-SEC-02 (injection and untrusted bytes/secrets, L3–L4/I5),
R-SSH-01 (OpenSSH config/host-key semantics, L4/I5), R-REL-01 (release integrity,
L4/I5 envelope). Repository access controls remain unchanged; branch protection
requires explicit human confirmation. No CI proves Rust buildability yet (no
toolchain, no manifest).

## Human decisions required

From [open-questions.md](open-questions.md); none answered: whether `main` should
require PRs and successful CI, block force pushes/deletion, and prefer squash
merging; which signing/notarization identity for macOS Preview artifacts (R-REL-01);
which public support/security contact before Preview.

## Next integration work

1. `codex-lab` testability/gate review and `Claude Code` security/maintainability
   review of the risk register; Codex integrates findings and closes Issue #8 only if
   the acceptance criteria hold.
2. Start Milestone 1 ([#6](https://github.com/ta-061/noren/issues/6)) Round 1
   independent proposals with the merged evidence and risk register as the shared
   pack ([design-process.md](design-process.md)).
3. Close the human decisions above before any access-control, signing, or publication
   work.

## Production gate

Closed. D-0001 stands: production implementation is prohibited until the Milestone 1
gate in [design-process.md](design-process.md) passes (requirements, architecture,
threat model, test strategy, release plan, risk register, addressed independent
reviews). This update neither opens it nor selects production architecture,
libraries, or dependencies.
