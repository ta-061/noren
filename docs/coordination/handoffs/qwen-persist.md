# Handoff — Milestone 3 sidebar persistence (M3-7)

Status: merge candidate. Updated 2026-08-07. Lane: `agent/m3-sidebar-persistence`.

## Purpose

Last Milestone 3 feature: Noren now persists its sidebar state — which
projects, worktrees, SSH targets, agents, and sessions exist, and which one
is selected. Per
[ADR 0003](../../adr/0003-noren-zellij-responsibility-boundary.md), nothing
inside a session is persisted: no tabs, no panes, no splits, no layout tree,
no terminal content. The parser enforces this boundary, not just the writer:
a session table carrying an interior key (`panes`, `layout`, `cwd`, ...) is
rejected as unknown rather than parsed and ignored.

## What landed

File lease honored; exactly two code files were created:

1. `crates/noren-app/src/session_persistence.rs` — the persistence module.
2. `crates/noren-app/tests/session_persistence.rs` — 29 integration tests
   plus 4 module unit tests exercised through the compile shim (33 total
   test bodies in the target).

No changes to `lib.rs`, `main.rs`, `session.rs`, `sidebar.rs`, `Cargo.toml`,
or `Cargo.lock`. The module reuses the D-M3-001 vocabulary
([`SessionKind`], [`SessionRegistry`]) from `src/session.rs`; it defines no
parallel session model and adds no dependencies (the workspace already pins
`toml_edit` with parse-only features, so the write side emits TOML text by
hand and the read side goes through the TOML parser).

## The format is versioned but not final

The on-disk document is TOML and carries `version = 1` from the first write:

```toml
version = 1
selected = 2          # positional index into [[sessions]]; omitted when none

[[sessions]]
kind = "local"        # or project/root, worktree/path, ssh/target, agent/name
```

Shipping a persistence format and then changing it breaks every existing
user's state, so this lane deliberately treats version 1 as a **starting
point with a migration path, not a public commitment**. The version is
checked before any other key is interpreted; a document claiming any other
version (past or future) is rejected whole — never partially parsed. Future
schema changes bump the version and migrate.

Deliberate omissions in v1, recorded here so the next format decision is
explicit rather than accidental:

- **No `SessionId` on disk.** D-M3-001 says IDs are registry-local and not
  persistence keys, so entries are stored in registry order and the
  selection is a positional index into that list.
- **No status.** Status is an observed runtime fact; restored entries
  re-enter through `SessionRegistry::create` as `Starting`.
- **No titles.** Titles are generated display ids today; a rename feature
  would need a setter that does not exist yet. A later format version can
  add the key.
- **Non-UTF-8 paths refuse to save** (`NonUtf8Path`) rather than persist a
  lossy transliteration that would silently name a different path.

## Untrusted input and bounds

A state file is treated as hostile input, mirroring `config.rs`:

- Absent file is the first run: empty state, no error, behavior identical
  to today.
- Truncated, malformed, non-UTF-8, wrong-version, oversized, and
  wrong-shaped files each produce a clean `SessionPersistenceError` with a
  clipped message; the registry is never mutated on the error path
  (validation is whole-document before any entry is created).
- Reads are streamed under `MAX_SESSION_STATE_BYTES` (512 KiB) and refuse
  more than `MAX_SESSIONS` (512) entries; writes refuse past either bound
  and never create the file on refusal.
- Writes are temp-file + fsync + rename: the file is the old state or the
  new state, never a truncation. Directory fsync is not performed.

## Restoration: re-spawn only; reattach-vs-respawn is unresolved

`load` re-populates the registry and selects the saved entry; it spawns
nothing. The M3 breakdown lists reattach-by-Zellij-session-name versus
re-spawn as an open question, and this lane does not answer it: the
implemented behavior is the simple case (restoring a session will re-spawn a
shell when the spawn lane adopts it). **Whether restoration reattaches to an
existing Zellij session or re-spawns remains an open question.**

## Not wired yet

The module is not declared in `lib.rs` — that one-line change belongs to the
M3 integration lane, matching the file-lease convention used for
`session`. Until then the integration test compiles the module through a
`#[path = "../src/session_persistence.rs"]` shim with a local `session`
mirror, exactly as `tests/session_domain.rs` did before PR #75. The wiring
lane should add `pub mod session_persistence;` to `lib.rs` and replace the
shim with crate imports. The sidebar UI still needs its own lane to adopt
save-on-change and load-on-startup; `SESSION_STATE_FILE_NAME`
(`sessions.toml`) is the recommended location inside the Noren data
directory.

## Verification

Gate output (macOS, `cargo 1.88.0`):

```text
cargo fmt --all                     # clean
cargo clippy --workspace --all-targets -- -D warnings   # clean
cargo test --workspace              # 422 passed, 0 failed, 1 pre-existing ignored
```

The workspace baseline before this lane was 388 passed / 0 failed / 1
ignored; this lane adds 34 test bodies, all green.

[`SessionKind`]: ../session-api.md
[`SessionRegistry`]: ../session-api.md
