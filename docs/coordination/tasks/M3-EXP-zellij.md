# Task M3-EXP-zellij

- Issue / task ID: M3-EXP-zellij (Milestone 3)
- Goal: Zellij session lifecycle experiment: detect, new, list, attach, exit, stale, attach failure, fallback.
- Base main SHA: `1d329a5`
- Depends on: none
- Assigned engine / lane: Fugu / `fugu-a`
- Independent verifier: GLM

## Exact file lease

Only these paths may be created or edited.

- `docs/experiments/zellij-session-lifecycle.md`

## Forbidden files

- `crates/*/src/lib.rs` — export wiring is a separate serial integration commit
- `crates/noren-app/src/main.rs`
- `Cargo.toml`, `Cargo.lock`
- `docs/coordination/status.md`
- any file leased by another task

## Public API contract

Experiment only. No production code, no parser/state, no credentials.

Contract source: `docs/coordination/decisions/D-M3-001-session-api.md`. A lane
needing a contract change escalates instead of forking it.

## Acceptance criteria

- Every claim carries an exact command and its exit status.
- Behavior when zellij is absent is recorded as a fallback path.
- A failure matrix distinguishes attach-failure from stale-session.
- No SSH key or credential is created, read, or stored.

## Required tests

- executed command transcripts with exit statuses, recorded in the doc

## Stop conditions

Stop and escalate rather than deciding, if:

- the work needs a pane, tab, or layout type — that is the ADR 0003 boundary;
- it needs a persistence format (owner decision);
- it needs a change to a contract type owned by another task;
- a required file falls outside this lease;
- a verifier reports a BLOCKER.
