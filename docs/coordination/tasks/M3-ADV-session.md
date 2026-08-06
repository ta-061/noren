# Task M3-ADV-session

- Issue / task ID: M3-ADV-session (Milestone 3)
- Goal: Adversarial session lifecycle: mass create/close, rapid switching, crashes, stale selection, races.
- Base main SHA: `1d329a5`
- Depends on: M3-1a
- Assigned engine / lane: Kimi / `kimi-a`
- Independent verifier: GLM or Qwen

## Exact file lease

Only these paths may be created or edited.

- `crates/noren-app/tests/session_adversarial.rs`

## Forbidden files

- `crates/*/src/lib.rs` — export wiring is a separate serial integration commit
- `crates/noren-app/src/main.rs`
- `Cargo.toml`, `Cargo.lock`
- `docs/coordination/status.md`
- any file leased by another task

## Public API contract

Attack the public API or a fake supervisor only. Do not couple to lane internals.

Contract source: `docs/coordination/decisions/D-M3-001-session-api.md`. A lane
needing a contract change escalates instead of forking it.

## Acceptance criteria

- Attacks target public API surface, not private functions.
- Unbounded growth from repeated create/close is checked explicitly.
- A found defect keeps its reproducer, marked ignored, and is reported not fixed.

## Required tests

- thousands of create/close cycles stay bounded
- rapid selection switching never leaves two or zero-when-nonempty selections
- duplicate or invalid ids are rejected without panic
- shutdown race does not leak or double-reap

## Stop conditions

Stop and escalate rather than deciding, if:

- the work needs a pane, tab, or layout type — that is the ADR 0003 boundary;
- it needs a persistence format (owner decision);
- it needs a change to a contract type owned by another task;
- a required file falls outside this lease;
- a verifier reports a BLOCKER.
