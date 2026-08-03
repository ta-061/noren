# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo), during the Issue #8 integration on
branch `docs/m0-risk-register`.

## Current phase

Milestone 0 — Discovery, evidence integration. Production implementation
has not started, and the production implementation gate remains closed
(D-0001 in [decisions.md](decisions.md); gate criteria in
[design-process.md](design-process.md)). This update makes no
architecture, library, or dependency selection.

## Completed Issues

All verified against GitHub on 2026-08-03:

- [#1](https://github.com/ta-061/noren/issues/1) closed — Discovery and
  governance baseline.
- [#3](https://github.com/ta-061/noren/issues/3) closed — terminal
  landscape and library comparison.
- [#4](https://github.com/ta-061/noren/issues/4) closed — cmux parity and
  Zellij compatibility matrices.
- [#5](https://github.com/ta-061/noren/issues/5) closed — OpenSSH and
  CLI-agent integration evidence.
- [#9](https://github.com/ta-061/noren/issues/9) closed — OpenCode
  executable provenance correction.

Open:

- [#8](https://github.com/ta-061/noren/issues/8) (this Issue) — risk
  register draft complete, independent reviews pending; integrator is
  Codex.
- [#6](https://github.com/ta-061/noren/issues/6) — Milestone 1 proposals,
  critiques, and integrated design; not started.

## Completed Pull requests and CI

All six Discovery PRs are merged into `main`; verified against GitHub on
2026-08-03:

| PR | Scope | Merge commit | Merged (UTC) |
| --- | --- | --- | --- |
| [#2](https://github.com/ta-061/noren/pull/2) | Discovery and governance baseline | `1288c909` | 2026-08-02 18:47 |
| [#7](https://github.com/ta-061/noren/pull/7) | Design council execution protocol | `40588c90` | 2026-08-02 19:20 |
| [#10](https://github.com/ta-061/noren/pull/10) | OpenCode provenance correction | `f76fbd46` | 2026-08-02 19:44 |
| [#11](https://github.com/ta-061/noren/pull/11) | cmux parity and Zellij compatibility | `7049908a` | 2026-08-03 00:24 |
| [#12](https://github.com/ta-061/noren/pull/12) | OpenSSH and agent integration evidence | `b37126cd` | 2026-08-03 00:36 |
| [#13](https://github.com/ta-061/noren/pull/13) | Terminal and library landscape | `38db222d` | 2026-08-03 00:10 |

`main` is at `b37126cd0f40c350f0ea4e28661aa7bdcd3dd3ac` (PR #12 merge).
Open PRs at this check: none; the Issue #8 Draft PR from
`docs/m0-risk-register` is opened as part of this handoff.

Review records for Issues #3–#5 exist as independent `codex-lab` and
`Claude Code` follow-up comments on PRs #13, #11, and #12 respectively,
mirrored in the merged report/matrix review sections. The final
dispositions of those findings are also recorded in those comments; this
statement is a pointer, not a new verdict.

CI evidence (`gh run list`, 2026-08-03):

- `Documentation` workflow succeeded after every merge into `main`
  (push runs `30761919676` for PR #2, `30763153886` for PR #7,
  `30764039824` for PR #10, `30773838402` for PR #13, `30774419605` for
  PR #11, `30774876022` for PR #12) and on every Discovery branch
  pull_request run; all listed conclusions are `success`. It runs
  `python3 scripts/check_docs.py` and
  `python3 -m unittest scripts/test_check_docs.py` on Ubuntu.
- Dependabot: the `github_actions` ecosystem updates succeed; the `cargo`
  ecosystem runs fail (for example runs `30761922192` and `30763205825`,
  conclusion `failure`, step `Run Dependabot`) because the repository has
  no Cargo manifest or Rust toolchain yet. This failure is expected
  Discovery-stage state, not a documentation regression, and is tracked in
  R-PORT-02.

## In progress

- Issue #8 integration on `docs/m0-risk-register`:
  [risk-register.md](../roadmap/risk-register.md) drafted with 20
  evidence-linked risk rows covering the Issue #8 categories; independent
  `codex-lab` (testability/gates) and `Claude Code`
  (security/maintainability) reviews are assigned but have not happened.
- Milestone 0 completion audit for the design council protocol: the risk
  register is a required shared input before Round 1 execution
  ([protocol-codex-lab.md](reviews/protocol-codex-lab.md)).

## Blocked

- Rust compilation remains impossible until a Rust toolchain is installed
  (R-PORT-02). This blocks every executable gate in the risk register but
  does not block Discovery, design, or documentation work.

## Risks

- See [risk-register.md](../roadmap/risk-register.md) (Issue #8
  deliverable). Highest-pair rows: R-IN-01 (input loss, L3/I5), R-SEC-01
  and R-SEC-02 (command injection and untrusted terminal control,
  L3–L4/I5), R-SSH-02 (host-key weakening, L2/I5), R-REL-01 (release
  integrity, L2/I5).
- Repository access controls remain unchanged; branch protection decisions
  require explicit human confirmation.
- No CI currently proves Rust buildability (no toolchain, no manifest).

## Human decisions required

From [open-questions.md](open-questions.md); none have been answered:

- Whether `main` should require pull requests and successful CI, block
  force pushes and deletion, and prefer squash merging.
- Which signing/notarization identity, if any, may be used for macOS
  Preview artifacts (none is assumed; R-REL-01).
- Which public support/security contact should be published before Preview.

## Next integration work

1. `codex-lab` testability/gate review and `Claude Code`
   security/maintainability review of the risk register; Codex integrates
   findings, then closes Issue #8 only if the acceptance criteria hold.
2. Start Milestone 1 ([#6](https://github.com/ta-061/noren/issues/6))
   Round 1 independent proposals using the merged Discovery evidence and
   the risk register as the shared evidence pack
   ([design-process.md](design-process.md)).
3. Close the human decisions above before any access-control, signing, or
   publication work.

## Production gate

Closed. D-0001 stands: production implementation is prohibited until the
Milestone 1 gate in [design-process.md](design-process.md) passes
(requirements, architecture, threat model, test strategy, release plan,
risk register, and addressed independent reviews). This update neither
opens it nor selects production architecture, libraries, or dependencies.
