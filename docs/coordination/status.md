# Coordination status

Last updated: 2026-08-03 (Asia/Tokyo)

## Current phase

Milestone 0 — Discovery. Production implementation has not started.

## Completed

- Audited the local repository and GitHub baseline.
- Verified GitHub CLI authentication without storing credentials.
- Created Issue #1 for the Discovery/governance baseline.
- Created branch `docs/discovery-baseline`; no work was written to `main`.
- Verified versions, help, non-interactive commands, resume commands, and
  non-secret model configuration for the installed AI CLIs.
- Completed the shared calibration task for all six roles, preserved their
  unedited responses, scored the evidence, and adjusted initial role ownership.
- Created scoped labels and Discovery/design/Preview milestones; assigned Issue
  #1 to Milestone 0.
- Enabled GitHub private vulnerability reporting.
- Added the initial community policies, dual-license texts, Issue/PR templates,
  dependency update configuration, and dependency-free documentation CI.
- Completed independent codex-lab review, resolved all six findings, and obtained
  a clean follow-up verdict.

## In progress

- Discovery research plan and source-backed landscape/library comparisons.

## Blocked

- Rust compilation is unavailable until a Rust toolchain is installed.

## Pull requests and CI

- Open PRs: Draft PR #2, `docs/discovery-baseline` → `main`.
- CI: `Validate repository documentation` is pending on PR #2; no workflow
  exists on `main` until the PR is merged.
- Release: none.

## Next integration work

1. Complete source-backed Discovery reports.
2. Run independent proposal and critique rounds.
3. Integrate Milestone 1 artifacts and obtain independent security/testability
   reviews.

## Risks

- The repository has no branch protection; changing access controls requires
  explicit human confirmation.
- No Rust toolchain or CI currently proves buildability.
- Upstream APIs, default keymaps, licenses, and maintenance state remain
  unverified until the Discovery reports cite authoritative sources.
