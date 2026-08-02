# Design council protocol independent QA

- Issue: [#6](https://github.com/ta-061/noren/issues/6)
- Pull request: [#7](https://github.com/ta-061/noren/pull/7)
- Commit reviewed: `5d630ae`
- Reviewer: codex-lab, configured `gpt-5.6-sol`
- Command: `codex-lab review --base main`
- First follow-up: same session via `codex-lab exec resume`
- Second follow-up: same session via `codex-lab exec resume`
- Review mode: read-only

## Findings and resolution

| Priority | Finding | Resolution |
| --- | --- | --- |
| P1 | Reviewers did not receive the governing proposal task, so they could not reliably distinguish an allowed deferral from a missed requirement. | Every review pack now includes the exact executed proposal-task revision as a required input. |
| P1 | Prompt-only restrictions did not enforce independent proposal/review runs when a CLI could access a worktree, persisted session, network, or other outputs. | Both protocols now require fresh minimal temporary snapshots, new non-resumed sessions, verified CLI-specific controls, recorded residual limitations, and invalidation/retry if hidden outputs were visible. |
| P2 | The Discovery risk register was absent from the Round 1 evidence list. | `docs/roadmap/risk-register.md` is now a mandatory shared input and must exist at the recorded commit before execution. |
| P2 | A failed or timed-out proposer could satisfy the outcome gate but left the required five-label review pack undefined. | Failed, timed-out, or unavailable outcomes now use provenance-only `Unavailable` placeholders; another model's text cannot replace them. |

During review, two repository-checker tests could not create temporary fixtures
because the reviewer sandbox was read-only. This is an expected limitation of
the review environment, not a passing test result. The integrator reruns the
checks in the normal repository environment after every correction.

## Additional integrator correction

The project goal requires every Noren shortcut to be both changeable and
disableable. The proposal task previously said "rebindable or disableable"; it
now states both requirements explicitly.

## First follow-up findings and resolution

| Priority | Finding | Resolution |
| --- | --- | --- |
| P1 | Putting the shared-input commit inside the task made the provenance requirement self-referential because inserting the hash changes the commit. | Run-specific evidence commit and final task blob hash now live in an external run manifest; the prompt contains no self-hash placeholder. |
| P1 | Detailed command/model/failure provenance inside an unavailable placeholder disclosed the hidden author-label mapping. | Reviewer-visible placeholders now contain only the opaque label and normalized `Unavailable`; detailed provenance is withheld until every review is captured. |

## Follow-up status

> All prior findings resolved.

The second follow-up inspected only commit `f4e9c7a` and confirmed that the
external-manifest design removes the self-reference, the reviewer-visible
placeholder preserves anonymity, and neither correction introduced a new
actionable finding.
