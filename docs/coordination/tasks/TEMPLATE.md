# Task <task-id>

- Issue / task ID:
- Goal:
- Base main SHA:
- Depends on:
- Assigned engine / lane:
- Independent verifier:

## Exact file lease

Files this task may create or edit. Anything not listed is forbidden.

-

## Forbidden files

Never edit these here, even incidentally.

- `crates/*/src/lib.rs` (export wiring is a separate integration commit)
- `crates/noren-app/src/main.rs`
- `Cargo.toml`, `Cargo.lock`
- `docs/coordination/status.md`

## Public API contract

The types and signatures this task must match. See
`docs/coordination/decisions/D-M3-001-session-api.md` where applicable. A task may not
redefine a contract type it does not own.

## Acceptance criteria

-

## Required tests

-

## Stop conditions

Stop and escalate rather than deciding, if:

- the work needs a change to a public API contract owned elsewhere;
- it needs a pane, tab, or layout type (that is the ADR 0003 boundary);
- it needs a persistence format;
- a required file is outside the lease;
- a verifier reports a BLOCKER.
