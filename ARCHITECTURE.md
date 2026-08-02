# Architecture

> **Status: not yet selected.** Noren is in Discovery. This file intentionally
> does not present a speculative architecture as approved.

The design council process is documented in
[`docs/coordination/design-process.md`](docs/coordination/design-process.md).
The authoritative architecture, repository/API contracts, threat model, and
numbered ADRs will be added during Milestone 1 after independent proposals and
cross-review.

Current fixed boundaries are limited to these principles:

- terminal input preservation is a first-class contract;
- local failures, remote failures, and UI state must be isolated;
- secrets and SSH private-key material do not belong in Noren state/logs;
- untrusted terminal/config/IPC input is bounded and validated;
- crates should have single responsibilities and no circular dependencies;
- replaceable third-party integrations sit behind Noren-owned contracts.

See [open questions](docs/coordination/open-questions.md) for unresolved choices.
