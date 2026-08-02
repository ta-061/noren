# Governance

Noren is currently a maintainer-led project in its founding phase.

## Roles

- **Maintainer/integrator:** owns repository administration, scope integration,
  architecture decisions, release gates, and final merges.
- **Contributors:** propose and implement scoped changes through Issues and PRs.
- **Reviewers:** independently evaluate requirements, code, tests, security,
  compatibility, and documentation. An implementer cannot be the final reviewer
  of the same change.

## Decisions

Routine reversible decisions may proceed with rationale in the Issue or PR.
Material architecture/API/security decisions require an RFC or ADR. Decisions
prioritize correctness, data preservation, security, compatibility,
verifiability, maintainability, feasibility, performance, extensibility, and
appearance—in that order. Recorded dissent is retained.

Changes to repository visibility, credentials, access controls, destructive
history operations, or deletion require explicit owner approval.

## Releases

Only the maintainer/integrator may authorize a release. A tag or artifact is not
an approved release unless all documented gates have current evidence, the CI
revision matches the release revision, checksums exist, and the release notes
state known limitations.

## Contributions

Contributions are accepted under the repository's dual license and Developer
Certificate of Origin policy in [CONTRIBUTING.md](CONTRIBUTING.md). Governance may
evolve through a public RFC as the contributor base grows.
