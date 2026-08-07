# Handoff — Milestone 3 session-domain wiring

Status: merge candidate. Updated 2026-08-07.

## Purpose

The session-domain implementation landed separately in PR #74 so its pure
state model could be reviewed without a crate-root file conflict. This serial
integration change makes that module reachable through the `noren-app` crate.

## Exact wiring

Two code/test edits are intentional:

1. `crates/noren-app/src/lib.rs` adds:

   ```rust
   pub mod session;
   ```

2. `crates/noren-app/tests/session_domain.rs` imports the module from the
   crate:

   ```rust
   use noren_app::session::{ /* contract types */ };
   ```

   The previous `#[path = "../src/session.rs"]` shim was valid only while the
   module was unwired. Keeping it after the crate export would compile a second,
   unrelated copy and let the integration test pass without exercising the
   actual public module.

No test body or session-domain implementation is changed by this PR. The merge
with current `main` preserved PR #74's monotonic lifecycle regression test and
public [session API](../session-api.md).

## Scope and boundary

- The session module remains the pure, in-memory registry reviewed in PR #74.
- No supervisor, process launch, sidebar, persistence, SSH, or agent launch path
  is wired here.
- No pane, tab, split, or layout type is introduced. The
  [Noren/Zellij boundary](../../adr/0003-noren-zellij-responsibility-boundary.md)
  remains intact.
- The public type shapes and lifecycle rules are unchanged.

## Verification

The merge candidate must pass:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/check_docs.py
python3 -m unittest scripts/test_check_docs.py
```

Expected workspace result after PR #74 is **388 passed, 0 failed, 1
pre-existing ignored**. Exporting the module places its 5 unit tests in the
`noren-app` library target, while `session_domain` runs 30 integration tests
through the crate import; together they provide 35 session-domain tests.

Before merge, the diff against current `main` must contain only the crate-root
export, the import-root switch, and this integration evidence. All four required
CI checks must pass on the current head, a review must cover that exact head,
and no review thread may remain unresolved.

## Follow-up

Future supervisor and sidebar modules receive their own implementation and
integration reviews. Their absence here is deliberate; this PR wires only the
session domain that is already present on `main`.
