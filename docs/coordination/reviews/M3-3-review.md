# Review M3-3 — sidebar view model and visual skeleton

- Reviewer: GLM (independent verifier per `state/tasks/M3-3.md`)
- Branch: `agent/m3-sidebar-view`
- Head SHA: `4db39103ee65` (code commit `6017b7c`; `4db3910` is the handoff)
- Base main SHA: `1d329a5`
- Task spec: `state/tasks/M3-3.md` (fleet repo)
- Author handoff: `docs/coordination/handoffs/qwen-a.md`

I did not write this code. Every command below was actually run; output is
quoted, not paraphrased. The branch was checked out in worktree `pool-m3c`; the
mandated gate was run there. Because the branch's own test target cannot compile
until the serial integration commit wires the modules (see MAJOR-1), I also
assembled a scratch integration in
`/var/folders/rm/g0367ctn3l93wc8vdr61nyjm0000gn/T/opencode/m3-3-rev-scratch`
that mirrors the planned integration commit, and ran the full gate there.

## Verdict
**FINDINGS** — the lane's design and tests are sound (all five acceptance
criteria met, all four required tests present and mutation-tested, ADR 0003
respected, lease clean). The single material issue is that the mandated gate is
**red on the branch by lease design**, not by any defect in the lane's code:
`lib.rs` is a forbidden file for this lane, so `pub mod session;`/`pub mod sidebar;`
cannot be added here, and the sidebar test target therefore does not compile
until the serial integration commit. I verified in scratch that once those two
lines are added the full workspace gate goes green.

`REVIEW_M3-3 verdict=FINDINGS blockers=0 majors=1 minors=2 tests=FAIL total=0`

(`tests=FAIL total=0` is the branch reality: `cargo test --workspace` exits 101
before any test binary runs. In the scratch integration the sidebar suite runs
**10/10 PASS**; see MAJOR-1.)

## Gate — real output on the branch (`pool-m3c`)

```
$ cargo fmt --all -- --check; echo "EXIT=$?"
EXIT=0

$ cargo clippy --workspace --all-targets -- -D warnings >/tmp/m3c-clippy.log 2>&1; echo $?
101
error[E0432]: unresolved import `noren_app::session`
  --> crates/noren-app/tests/sidebar_view.rs:12:16
12 | use noren_app::session::{SessionDescriptor, SessionId, SessionRegistry, SessionStatus};
   |                ^^^^^^^ could not find `session` in `noren_app`
error[E0432]: unresolved import `noren_app::sidebar`
  --> crates/noren-app/tests/sidebar_view.rs:13:16
error: could not compile `noren-app` (test "sidebar_view") due to 2 previous errors

$ cargo test --workspace >/tmp/m3c-test.log 2>&1; echo $?
101
# identical E0432 x2; no test binary runs
```

So: fmt PASS, clippy FAIL (101), test FAIL (101, 0 tests executed). This matches
the handoff exactly (`qwen-a.md:36-43`). It is not a green gate, and I am not
treating it as one.

### Scratch integration (mirrors the planned serial integration commit)

Copy of this worktree + M3-1a's `session.rs`/`session_domain.rs` (read from
`pool-m3a`) + two lines added to the scratch `lib.rs` only:

```
 mod clipboard;
 pub mod config;
 pub mod diagnostics;
 mod input;
+pub mod session;
+pub mod sidebar;
```

Result, all real:

```
$ cargo fmt --all -- --check          # EXIT 0
$ cargo clippy --workspace --all-targets -- -D warnings   # EXIT 0, no warnings
$ cargo test --workspace              # EXIT 0
  noren-app lib unit ........... 84 passed; 1 ignored
  noren-app bin unit ............ 24 passed
  tests/session_domain.rs ....... 32 passed
  tests/sidebar_view.rs ........ 10 passed
  tests/verify59_independent.rs . 19 passed
  (+ full noren-terminal / noren-pty suites, all green)
```

The sidebar suite (`tests/sidebar_view.rs`) runs **10/10 PASS** once wired.

## Findings

### MAJOR-1 — The mandated gate is red on the branch (clippy + test exit 101)
- Location: `crates/noren-app/src/lib.rs:11-14` (the missing `pub mod session;`
  / `pub mod sidebar;`), manifesting as the two E0432s in
  `crates/noren-app/tests/sidebar_view.rs:12-13`.
- Reproduction: `cargo clippy --workspace --all-targets -- -D warnings` on the
  branch → exit 101 (output above).
- Expected vs actual: the task's mandated gate expects PASS; the branch produces
  clippy FAIL and test FAIL with zero tests executed.
- Why this is MAJOR and not BLOCKER: the lease (`state/tasks/M3-3.md` "Forbidden
  files") explicitly forbids this lane from editing `lib.rs` — "export wiring is
  a separate serial integration commit." The lane cannot make its own gate green
  without violating the lease, so the red gate is not a defect the lane can fix
  or can be held at fault for. The handoff is honest about it, and I confirmed
  the code is correct when wired: scratch clippy is clean (exit 0, no warnings)
  and the full workspace test suite passes (sidebar 10/10, session_domain 32/32,
  everything else green). The fix is the serial integration commit; until it
  lands, this branch is not independently verifiable and not mergeable green.
- Suggested fix (for the integration commit, not this lane): after merging M3-1a
  and M3-3, add `pub mod session;` and `pub mod sidebar;` to
  `crates/noren-app/src/lib.rs` (both must be `pub`, not private `mod`, because
  `tests/sidebar_view.rs` and `tests/session_domain.rs` are integration tests
  and link to the crate as an external dependency). Re-run the three gate
  commands.

### MINOR-1 — Test fixtures ship as permanent public API
- Location: `crates/noren-app/src/sidebar.rs:338` (`pub mod fixtures`), exposing
  `session_registry()`, `entries()`, and the sample data ("noren", "pool-m3c",
  "agent/m3-sidebar-view", "web1.internal:22", "claude-code") as
  `noren_app::sidebar::fixtures`.
- Why it is only a minor: this is forced by the test placement. Integration
  tests under `tests/` cannot see `#[cfg(test)]` items defined inside `src/`, so
  the fixtures must be `pub` and unconditionally compiled to be reachable from
  `tests/sidebar_view.rs`. The lane made the right tradeoff given the lease
  (which forbids editing `Cargo.toml` to add a `test-support` feature/crate). It
  does, however, mean sample/host data is part of the crate's public surface.
- Suggested fix (later, not this lane): move fixtures behind a `test-support`
  Cargo feature or a dedicated `noren-app-test-support` crate so they do not
  ship in release builds.

### MINOR-2 — Sidebar codes against M3-1a's implementation, which diverges from the D-M3-001 sketch (integration-time risk)
- Location: `crates/noren-app/src/sidebar.rs:23` (the `use crate::session::…`),
  `:317-332` (`session_kind_text`/`session_status_text` match arms), and the
  `SessionKind::{Local, Ssh, Agent}` / `label: Option<String>` assumptions.
- The D-M3-001 *sketch* (`state/D-M3-001-session-api.md:40-62`) draws
  `SessionKind { Local, Project{..}, Worktree{..}, Ssh{..}, Agent{..} }` and
  `SessionDescriptor.title: String`. M3-1a's *implementation*
  (`pool-m3a/.../session.rs:57-65, 107`) is `SessionKind { Local, Ssh, Agent }`
  and `label: Option<String>`. The sidebar correctly codes against the
  implementation and models Project/Worktree as sidebar-level `EntryKind`s
  instead — it does **not** fork or redefine the contract (the lease's
  "Imports SessionDescriptor … must not redefine it" is honored).
- Why it is only a minor: the handoff already flags this
  (`qwen-a.md:93-113`), and the divergence is a coordinator decision, not this
  lane's. The risk is purely mechanical: if M3-1a's *committed* shape differs
  from the snapshot I read (it was untracked when the sidebar was written), the
  `match` arms in `session_kind_text`/`session_status_text` and the
  `label()`/`id()` calls need a one-line adjustment at integration. No action
  for this lane; flagged so the integration commit does not assume "compiles in
  scratch ⇒ compiles after M3-1a lands."

## Acceptance criteria — one by one (all met)

Source: `state/tasks/M3-3.md:32-38`.

1. **Renderer-independent (no colors, geometry, or widget types).** MET.
   Every public type in `sidebar.rs` carries only text (`String`/`&str`), the
   `EntryKind` enum, `bool`, and `Option`. No color, geometry, or widget type
   appears; scratch clippy under `-D warnings` is clean.
2. **Entry kinds render distinguishably for project/worktree/SSH/agent/session.**
   MET. `EntryKind` (`sidebar.rs:32-43`) has five distinct variants, mapped 1:1
   in `build` (`:218-255`). `each_entry_kind_maps_to_a_distinct_view_row`
   asserts both the exact kind sequence and `distinct.len() == 5`.
3. **Empty state is representable.** MET. `EmptyState`
   (`:127-147`), `EMPTY_SIDEBAR_MESSAGE` (`:133`), `empty_state()` accessor
   (`:282`); set iff `rows.is_empty()` (`:258-260`).
   `empty_sidebar_yields_an_empty_state_view_not_a_panic` covers it.
4. **Exactly one selected session shown; hidden sessions not rendered.** MET.
   `build` selects the first session whose id matches `selected`
   (`:243-248`) and builds exactly one `SessionViewport`; subsequent matches are
   deselected by the `viewport.is_none()` guard. Unselected sessions appear as
   sidebar rows (per ADR 0003 the sidebar lists sessions) but produce no
   viewport. `one_selected_session_among_many_describes_exactly_one_viewport`
   and `unselected_sessions_produce_no_viewport` cover both halves.
5. **No native tabs, panes, or layout.** MET. No `Pane`/`Tab`/`Layout`/split
   type exists in the file. `SessionViewport` (`:154-185`) carries only a
   `SessionDescriptor` and exposes `descriptor()`/`session_id()`/`label()`/`title()`
   — identity only, "nothing about what is displayed inside it" (`:12-14`).

## Required tests — all four present and behavioral

All four named cases from `state/tasks/M3-3.md:41-45` exist in
`tests/sidebar_view.rs` and PASS in scratch; six extra invariant tests
accompany them. To confirm the tests actually test behavior (not just pass), I
mutated the scratch `sidebar.rs` and re-ran the suite. Every behavioral mutation
was caught:

| Mutation on `sidebar.rs` | Test that failed |
|---|---|
| drop `viewport.is_none()` first-match guard (all dup ids selected) | `duplicate_session_descriptions_keep_exactly_one_selection` |
| never construct `SessionViewport` | 4 failures (incl. the two viewport-required cases) |
| wrong empty-state message (`"MUTATED"`) | `empty_sidebar_yields_an_empty_state_view_not_a_panic`, `each_entry_kind…` |
| collapse `Worktree`→`Project` kind | `each_entry_kind_maps_to_a_distinct_view_row` |
| omit status from `session_detail` | `session_rows_report_observed_status`, `each_entry_kind…` |
| mark all matching selected but keep viewport single | `duplicate_session_descriptions_keep_exactly_one_selection` |

A test that passes against broken code is worse than none; these do not.

## Panics, resource leaks, unbounded growth

`SidebarView::build` allocates `Vec::with_capacity(entries.len())` and pushes
exactly one row per entry — output size is linearly bounded by input, nothing
accumulates across calls. There is no indexing, `unwrap`, or `expect` in `build`,
`session_label`, or `session_detail`; the only `expect`/`debug_assert` live in
the test-only `fixtures` module. I fed hostile/degenerate input through a
reviewer probe (`tests/probe_degenerate.rs` in scratch):

| Probe | Result |
|---|---|
| 200 000 `Project` entries | 200 000 rows, no panic, no viewport, `!is_empty` |
| empty-string + CJK/emoji labels | renders without panic, labels preserved |
| `selected` pointing only at non-`Session` entries | viewport `None`, 0 selected rows (dangling dropped) |
| 50 000 duplicate selected session descriptors | exactly 1 selected row + 1 viewport |

No panics, no unbounded growth, no leaks observed.

## Unintended deletions / lease compliance

```
$ git diff --stat origin/main...HEAD
 crates/noren-app/src/sidebar.rs        | 389 ++++++++++
 crates/noren-app/tests/sidebar_view.rs | 259 ++++++++++
 docs/coordination/handoffs/qwen-a.md   | 164 ++++++++
 3 files changed, 812 insertions(+)
$ git diff --name-status origin/main...HEAD
A	crates/noren-app/src/sidebar.rs
A	crates/noren-app/tests/sidebar_view.rs
A	docs/coordination/handoffs/qwen-a.md
```

Purely additive: three new files, 812 insertions, **zero deletions**, no
modifications to existing files. Forbidden-file check (`state/tasks/M3-3.md:17-23`):
`lib.rs`, `main.rs`, `Cargo.toml`, `Cargo.lock`, `docs/coordination/status.md`
all untouched (verified by `git diff --name-only origin/main...HEAD -- <path>`).
Only the two leased paths plus the handoff were created. Lease compliance is
exact.

## Noren/Zellij boundary (ADR 0003)

Respected. ADR 0003 (`docs/adr/0003-noren-zellij-responsibility-boundary.md`,
read from `pool-p16`) puts the sidebar, project/worktree/SSH/agent/session
listing, and single-session selection on Noren's side, and forbids Noren-side
tabs/panes/layout/splits. `sidebar.rs` introduces no pane/tab/layout/split type;
`SessionViewport` names the visible session and carries nothing about what is
displayed inside it (`:12-14`, `:149-153`). The module doc explicitly anchors
itself to ADR 0003 (`:10-14`). No boundary violation.

## Areas that are genuinely sound

- The build/selection invariant logic (first-match-wins, dangling-selection
  dropped, empty-state iff no rows) is correct and well-tested.
- Public encapsulation is tight: struct fields private, accessors `#[must_use]`,
  no mutability handed out.
- Doc comments accurately describe behavior and cite the governing ADR/contract.
- Lease discipline is perfect (additive only; forbidden files untouched).

## Recommendation

Land via the serial integration commit: merge M3-1a + M3-3, add
`pub mod session;` and `pub mod sidebar;` (both `pub`) to `lib.rs`, re-run the
three gate commands. If M3-1a's committed `session.rs` differs from the snapshot
reviewed here, adjust the two `match` arms in `session_kind_text`/
`session_status_text` and the `label()`/`id()` call sites in `sidebar.rs` —
expected to be mechanical. No changes to this lane's code are required for
correctness; the two MINORs are cleanup/risk notes, not defects.
