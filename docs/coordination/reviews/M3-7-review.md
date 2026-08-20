# Independent review — M3-7 sidebar persistence (`agent/m3-sidebar-persistence`)

Independent review. I did not author this code and reviewed head from scratch
rather than trusting the handoff. Commands below were actually run; output is
quoted.

- Reviewed at: `d3e136c990d0` on `agent/m3-sidebar-persistence`, off
  `origin/main` @ `91a0536` (macOS arm64).
- The lane prompt cites `state/tasks/M3-7.md` in the fleet repo as the
  authority. **That file is absent from this branch** (`git ls-files` has no
  `state/` tree). The acceptance criteria below are taken from the M3-7 entry
  in `docs/roadmap/milestone-3-breakdown.md`, which is the authoritative
  in-repo decomposition of M3-7 and matches the handoff's claims verbatim. If
  the fleet task spec carries stricter criteria, this review should be
  reconciled against it.
- Scope of diff (`git diff --stat origin/main...HEAD`): 3 files, all `A`
  (additions only, +1453 / −0). No file is modified or deleted — the file
  lease is intact and there are no unintended removals. `git checkout` of the
  branch was run from this sibling worktree because the branch was already
  checked out at `pool-persist` (worktree lock); same commit, clean tree.

## Gate output (actually run at `d3e136c`)

```
$ cargo fmt --all -- --check
    exit 0 (no diff)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] → exit 0, 0 warnings

$ cargo test --workspace
    PASSED=422  FAILED=0  IGNORED=1
    (session_persistence target: ok. 34 passed; 0 failed)
```

422 passed reconciles with the handoff (388 baseline + 34 new). The 34 break
down as 29 integration tests in `tests/session_persistence.rs` plus 5 unit
tests in the module's `#[cfg(test)]` block. (The handoff text says "33 total";
the actual runner reports 34 — a harmless miscount in the handoff.)

## Acceptance criteria (M3-7, `milestone-3-breakdown.md`), one by one

1. **"A sidebar with multiple sessions and external-context entries
   round-trips through save/reload with the selected session and entry list
   restored." — MET.** All five `SessionKind` variants (Local, Project,
   Worktree, Ssh, Agent) round-trip through `save`→`load` with the selection
   preserved as a positional index (`round_trip_preserves_every_entry_kind_…`,
   `hostile_strings_round_trip_through_the_escaper`, `encoding_is_deterministic_…`).
   Hostile payloads (embedded quotes, backslashes, newlines, tabs, ESC, CJK)
   survive the hand-rolled TOML escaper and parse back identically.

2. **"A corrupted/partial file is rejected without data loss; the last valid
   sidebar state is retained." — MET.** Truncated, malformed, non-UTF-8,
   wrong-type, wrong-version, oversized, directory, and wrong-shape inputs
   each yield a typed `SessionPersistenceError` and leave the registry
   untouched. Whole-document validation runs before `apply` mutates
   (`load_errors_never_mutate_a_populated_registry` proves it against a
   pre-populated registry).

3. **"No secrets, raw commands, terminal content, or layout are persisted." —
   MET.** The serialized surface is exactly `{ version, selected?, [[sessions]]
   { kind + one payload } }`. No `SessionId`, status, title, command, or
   terminal byte is written. The decoder enforces the same surface: unknown
   top-level keys and unknown session-table keys are rejected, not ignored.

## ADR 0003 boundary — enforced in the parser (sound)

This is the most important check for this lane and it is solid. The boundary
is enforced on **both** sides:

- **Write:** `encode` emits only `kind` and the single payload per session;
  no pane/tab/layout/split/cwd/content key is ever produced.
- **Read:** `parse_session` → `require_exact_keys`/`payload` rejects any key
  beyond `kind` and the kind's one payload. A session table carrying `panes`,
  `tabs`, `layout`, or `cwd` lands in `UnknownKey`, not a silent drop
  (`session_interior_keys_are_rejected_by_the_boundary`). Unknown top-level
  keys (`theme`, etc.) are likewise rejected.

No Zellij-internal structure is read or persisted. No blocker.

## Mutation testing (tests genuinely test the behavior)

Three mutations were applied to `src/session_persistence.rs`, rebuilt, and the
suite re-run. Each was caught:

- **M1 — disable the version guard** (`if version != …` → `let _ = version;`):
  3 tests FAILED (`wrong_versions_are_rejected_whole`,
  `a_future_version_is_rejected_without_partial_state`, and a populated-load
  regression). Restored.
- **M2 — drop `require_exact_keys` on the `local` arm**: 1 test FAILED
  (`session_interior_keys_are_rejected_by_the_boundary` — the ADR 0003 guard).
  Restored.
- **M3 — create-before-bounds-check in `apply`** (move the `MAX_SESSIONS`
  guard below the `create` loop): 1 test FAILED
  (`restoring_into_a_populated_registry_keeps_the_bound`), proving the
  no-partial-mutation guarantee is tested. Restored.

After all restores the tree is clean and the suite is green again.

## Hostile-input / panic / unbounded-growth probing

A throwaway fuzz probe fed the decoder deeply nested tables, wrong-typed
`version`/`selected` (float/bool/array/inline-table/huge integer), an
inline-array-of-tables `sessions`, an interior inline-table (`panes = {…}`),
a 50 000-char `kind` string, 512 sessions, a null-byte path, and selected-
without-sessions. **Every case produced a typed error or a clean apply; zero
panics.** Error messages stayed bounded (hostile keys are clipped to 120 chars
+ `…`; parser detail strings are clipped). The probe was deleted; it is not
part of the deliverable.

## Findings

### MAJOR-1 — The write-side byte bound is documented but not enforced

`encode` (`crates/noren-app/src/session_persistence.rs:235-277`) checks
`MAX_SESSIONS` but **never checks `MAX_SESSION_STATE_BYTES`**. Three places
claim otherwise:

- Module doc, lines 39–42: *"writes refuse to encode past either bound."*
- `save` docstring, lines 188–191: *"Refuses to encode … a document larger
  than `MAX_SESSION_STATE_BYTES`."*
- Handoff (`qwen-persist.md`): *"writes refuse past either bound and never
  create the file on refusal."*

**Reproduction (run, not inferred):** a single session whose payload alone
exceeds the cap is encoded without error:

```
let huge = "h".repeat(MAX_SESSION_STATE_BYTES as usize + 1); // 524 289
let mut registry = SessionRegistry::new();
let _ = registry.create(SessionKind::Ssh { target: huge });
encode(&registry)  // → Ok(524 478-byte document); bound is 524 288
```

Runner output:

```
encode SUCCEEDED with a 524478 byte document (bound is 524288):
WRITE BYTE BOUND NOT ENFORCED
```

**Consequences.**

1. Contract violation: the documented bound does not hold.
2. Loss-of-state failure mode: `save` writes a file that `load` then rejects
   with `TooLarge`, so the app persists state it can never read back — exactly
   the "without data loss" property acceptance criterion 2 demands.
3. Unbounded `String` growth on the write path (the defect class this review
   targets): nothing prevents a multi-megabyte payload from being formatted
   into memory before any check. The registry bounds *count*, not *size*.

The existing write-bound test (`the_maximum_session_list_stays_bounded_on_disk`,
line 662) only exercises 512 short-path entries (~26 KB total); it cannot
catch a single oversized payload, so the gap is unguarded by tests too.

**Minimal fix.** After building `text` in `encode`, reject on overflow:

```rust
if text.len() > MAX_SESSION_STATE_BYTES {
    return Err(SessionPersistenceError::TooLarge);
}
```

(A tighter fix bounds the accumulator incrementally and avoids formatting a
document known to be too large; either is acceptable for v1.)

### MINOR-1 — "Reload-on-launch" is in scope but not delivered

The M3-7 scope lists *"reload-on-launch"*; the module implements `load`/`save`
but is **not wired** — `lib.rs` has no `pub mod session_persistence;`, so the
code is dead until the integration lane adds the declaration. The handoff
states this is deliberate (file-lease convention, matching how `session` was
landed via shim). Given the lane model this is a reasonable sequencing choice,
not a defect in the module under review; recorded so the integration lane
knows the one-line wiring plus the sidebar save-on-change hook are still
outstanding. No `lib.rs`/`Cargo.toml`/`Cargo.lock` changes were made, so no
new dependency was added (the breakdown anticipated a serialization
dependency; the module avoids one by hand-encoding TOML and parsing with the
already-pinned `toml_edit`).

## Areas that are sound (no finding invented)

- **Atomic write.** `write_atomic` (temp sibling → `write_all` → `sync_all`
  → `rename`; cleanup on every error path) is correct for the "old-or-new,
  never truncated" contract on Unix. Directory fsync is acknowledged as open.
- **Determinism.** `encode` is deterministic (entries in id-sorted order,
  `format!` only); `encoding_is_deterministic_across_saves` and a
  save→load→re-encode byte-equality check both pass.
- **No partial mutation on error.** `decode` fully validates before `apply`
  creates a single entry; `apply`'s only error path precedes any `create`.
  Proven by M3 above.
- **Non-UTF-8 path refusal.** `path_text` returns `NonUtf8Path` rather than a
  lossy conversion; a refused save leaves no file behind (tested).
- **Error-message clipping.** Hostile keys and parser details are clipped to
  120 chars; message size verified under 1 KiB against 100 KB input.

## Verdict

FINDINGS. One MAJOR (write-side byte bound not enforced, contradicting the
module's own contract and creating a save-then-fail-to-load state-loss path)
and one MINOR (reload-on-launch deferred to the integration lane). The ADR
0003 boundary is correctly enforced in the parser and the writer; no blocker.

```
REVIEW_M3-7 verdict=FINDINGS blockers=0 majors=1 minors=1 tests=PASS total=422
```
