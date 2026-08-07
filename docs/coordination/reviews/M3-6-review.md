# Independent review: M3-6 command palette (`agent/m3-command-palette`)

- Reviewer lane: `qwen-rv1-6` (independent review; did not author the code).
- Branch: `agent/m3-command-palette` at head `5c0a67a16157af754361f46301503713670e6609`.
- Base: `origin/main` (`1d329a5`). Exactly 1 commit ahead; branch **not pushed**
  (`git ls-remote origin agent/m3-command-palette` returns nothing).
- Diff: `crates/noren-app/src/palette.rs` (+282), `crates/noren-app/tests/palette.rs`
  (+232), `docs/coordination/handoffs/glm-palette.md` (+115). Total +629/−0.

## Spec authority note

`state/tasks/M3-6.md` does **not exist** in the fleet repo (`state/tasks/` holds
M3-1a/1b/3/4/ADV/EXP only; the fleet queue entry for M3-6 has `"spec": ""`). The
effective acceptance criteria are those in `prompts/glm-palette-m3.md` (fleet repo),
which is what the author lane received. This review scores against that prompt.

## Gate (run by reviewer, not quoted from the handoff)

All three commands were run in the worktree `/Users/yoshinagatatsuya/Documents/apps/noren-worktrees/pool-palette`.

```
$ cargo fmt --all -- --check
(exit 0, no output)

$ cargo clean -p noren-app && cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (.../pool-palette/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.86s
(exit 0, zero warnings — verified from a clean package cache, not reused artifacts)

$ cargo test --workspace
test result: ok. 79 passed; 0 failed; 1 ignored   (noren-app lib)
test result: ok. 24 passed; 0 failed              (noren-app bin)
test result: ok. 14 passed; 0 failed              (tests/palette.rs ← this lane)
test result: ok. 19 passed; 0 failed              (tests/verify59_independent)
test result: ok. 10 passed; 0 failed              (noren-pty)
test result: ok. 45+176 passed; 0 failed          (noren-terminal lib + 16 test files)
TOTAL: 367 passed; 0 failed; 1 ignored; exit 0
```

The single ignored test is `clipboard::tests::system_clipboard_round_trips_user_text`,
`#[ignore]`d on `origin/main` already (verified via `git show origin/main:...`), unrelated
to this lane. The handoff's gate claims reproduce exactly.

## Acceptance criteria (per `prompts/glm-palette-m3.md`)

| Criterion | Met? | Evidence |
| --- | --- | --- |
| Session + sidebar commands only; never pane/tab/split/layout (ADR 0003) | **Met** | `Palette::noren` (palette.rs:193-200) builds exactly `session.create/select/close`, `sidebar.focus`. Enforced by two tests (exact ID list + keyword scan). Reviewer mutation (sidebar ID → `pane.split`) failed 3 tests. No pane/tab/split/layout type or state exists in the module. |
| Stable IDs for keybindings/config | **Met** | `CommandId(&'static str)` newtype with const constants, `as_str`/`AsRef`/`Display`; asserted against literal strings in `command_ids_are_stable_strings`. |
| Fuzzy **or** substring matching — pick one, document it, imply nothing unbuilt | **Met** | ASCII case-insensitive **substring**, documented in module docs (palette.rs:30-40); non-ASCII folding explicitly declared out of scope (verified true by probe: `É` does not match `é`). |
| Renderer-independent (no colors, geometry, widget types) | **Met** | Module contains only model types (`CommandId`, `Command`, `SearchHit`, `Palette`); zero rendering/geometry/color types. |
| Empty / no-match / all-escaped queries behave definedly, no panic | **Met** | Dedicated tests for all three; the escaped-char query runs under `catch_unwind`. Reviewer probes extended this to emoji-only, ZWJ-only, combining marks, C0 controls, U+10FFFF, ZWSP-prefixed, and a 1,000,000-char query — all defined, all panic-free, all fast. |
| Reuse `SessionAction` from D-M3-001; no parallel action enum | **Met (with documented deferral)** | `docs/coordination/decisions/D-M3-001-session-api.md` does not exist on this branch's base; no `SessionAction` exists anywhere in the tree (`grep` confirms the only mention is palette.rs's own doc comment). The author's claim that the dependency did not exist is **true**. `Command<A>` is generic and the module defines **no** action enum of its own — the negative constraint ("do not define a parallel action enum") is satisfied; the positive binding to `SessionAction` is deferred to the wiring lane the prompt itself mandates. |
| File lease: only the two palette files; no touching lib.rs/main.rs/actions.rs/sidebar.rs/Cargo.toml/Cargo.lock | **Met** | `git diff --stat origin/main...HEAD` shows exactly the two lease files + the mandated handoff doc. `lib.rs`/`main.rs`/`Cargo.toml`/`Cargo.lock` untouched; `actions.rs`/`sidebar.rs` do not exist on base. The module is genuinely unwired: no reference outside its own `#[path]` shim in `tests/palette.rs` (the other "palette" grep hits are the unrelated noren-terminal ANSI *color* palette, unchanged). |
| Handoff written, commit `-s`, not pushed | **Met** | `docs/coordination/handoffs/glm-palette.md` present; commit message carries `Signed-off-by: ta-061 <...>`; `git ls-remote` confirms the branch is not on origin. |

## Unintended deletions

None. `git diff origin/main...HEAD | grep -E '^- '` returns only the three `--- /dev/null`
new-file headers; zero content lines deleted. Nothing was removed, moved, or renamed.

## ADR 0003 boundary check

ADR 0003 (`docs/adr/0003-noren-zellij-responsibility-boundary.md`, currently on the
unmerged branch `docs/noren-zellij-boundary` — it is not on this branch's base either)
assigns tabs/panes/splits/layout/focus *inside* the terminal to Zellij and
sidebar/sessions *outside* to Noren. The palette introduces no pane, tab, split, layout
tree, or focus-movement command, performs no I/O, and reads/persists no Zellij layout.
**No BLOCKER.** One coordination observation, not a finding against this lane: both
ADR 0003 and D-M3-001 are referenced by this branch while still unmerged on `main`.

## Resource, panic, and unbounded-growth audit

- No `unsafe`, no I/O, no handles, no timers, no global state — nothing to leak.
- The catalog has **no mutation API** (`Palette` is immutable after construction), so
  unbounded growth is structurally impossible for the shipped type; `Palette::noren` is
  fixed at 4.
- Per-search allocations (`Vec<char>` folds) are bounded by catalog×label size and freed
  on return. A 1M-char query against the canonical palette completes in ~0 ms because
  `windows(n)` on a shorter haystack yields nothing.

## Mutation testing (do the tests test the behavior?)

Five mutations applied to `palette.rs`, run against `tests/palette.rs`, then reverted
(file verified byte-identical to HEAD via `diff` after each; worktree left clean):

| Mutation | Result |
| --- | --- |
| Drop ASCII folding of the query needle | **KILLED** — `case_insensitive_substring_matches_a_subset_of_labels` FAILED |
| Empty query returns empty instead of all commands | **KILLED** — `empty_query_returns_every_command_in_catalog_order` FAILED |
| Rank by latest match (`Reverse(index)`) | **KILLED** — `ranking_prefers_earlier_match_position_then_catalog_order` FAILED |
| Canonical sidebar ID → `pane.split` | **KILLED** — 3 tests FAILED (exact catalog, keyword guard, empty-query order) |
| Adding a 5th command reusing an action value | Does not compile for generic `A` (no `Copy`/`Clone`) — a structural guard |
| Canonical label `"Focus Sidebar"` → `"Tab Bar"` | **SURVIVED** — see MINOR-2 below |

## Interactions the author did not test (reviewer probes, temporary file, deleted after use)

The module is deliberately isolated (not wired), so the available interaction surface is
internal. All probes passed:

- Case folding **combined with** ranking: mixed-case query `"new Session"` against
  labels `"NEW SESSION"` / `"zz NEW session"` → correct order `[0, 3]`.
- Non-ASCII label **combined with** char-index semantics: `"Café Spotlight"` + query
  `"spotlight"` → `match_index == 5` (chars), not 6 (bytes) — the documented char-index
  guarantee is **true** (`é` is 2 bytes but counts once).
- Duplicate IDs via `from_commands` → `get` returns the first, `search("")` returns both.
- Empty-label command: searchable with empty query, never matches non-empty queries.
- `get(hit.command().id())` is `Some` for every hit of 8 query shapes including `""`,
  `"\\."`, `"café"`, `"CAFÉ"`, `"ü"`.

## Findings

No BLOCKER. No MAJOR. Three MINOR.

### MINOR-1 Documented char-index/UTF-8 guarantee has no shipped test
- Location: guarantee at `crates/noren-app/src/palette.rs:133-138` ("match index counts
  `char`s, not bytes") and handoff line 71-72.
- Reproduction: every label in `tests/palette.rs` is ASCII; no shipped test exercises a
  multibyte label.
- Expected vs actual: behavior is currently **correct** (reviewer probe verified
  char-index 5 vs byte 6 for `"Café Spotlight"`), but a regression to byte indexing
  would pass the whole suite while violating the module's documented contract.
- Suggested fix: add one test with a multibyte label, e.g. assert
  `match_index() == 5` for query `"spotlight"` against label `"Café Spotlight"`.

### MINOR-2 Label keyword guard is bypassable at label start; handoff overstates enforcement
- Location: `crates/noren-app/tests/palette.rs:66` — `!lower.contains(" tab")`.
- Reproduction: mutate the canonical label `"Focus Sidebar"` → `"Tab Bar"` (ID
  unchanged). `no_pane_tab_split_or_layout_command_is_offered` **passes** — the
  leading-space heuristic never matches a tab-word at position 0, and the ID keyword
  check does not see labels.
- Expected vs actual: expected — per handoff line 44-45, "no canonical command id or
  label contains `pane`, `tab`, `split`, or `layout` ... enforced by the suite". Actual
  — enforcement is real for IDs (exact list + substring scan both hold) but only partial
  for labels; a tab-named label on a legitimate ID escapes the suite.
- Real-world impact: low — dispatch keys on IDs, and any genuine tab command would carry
  a tab-like ID which **is** caught; the hole requires a deliberately misleading label.
- Suggested fix: check label words, e.g.
  `lower.split_whitespace().all(|w| !["pane","tab","split","layout"].contains(&w))`,
  keeping the "establish"/"table" false-positive protection without the position hole.

### MINOR-3 Handoff test-count arithmetic is wrong (totals are right)
- Location: `docs/coordination/handoffs/glm-palette.md:88-91` — "the previously-passing
  177 on `main` are unaffected".
- Reproduction: `git grep -c '#\[test\]' origin/main -- 'crates/**/*.rs'` → **354** test
  functions (HEAD: 368 = 354 + 14 new; runtime total 367 passed + 1 ignored = 368).
- Expected vs actual: expected 353 previously-passing (367 − 14). Actual claim: 177.
  The 177 figure matches nothing reachable (`noren-app` alone is 122, terminal suite
  221). All other handoff numbers (14 new, 367 total, 1 pre-existing ignored) verified
  accurate.
- Suggested fix: correct the sentence to "the previously-passing 353 on `main` are
  unaffected" (or drop the number). Documentation-only; no code impact.

## Sound areas (verified, no findings)

- Matcher core (`substring_index`, palette.rs:275-282): earliest-position semantics,
  correct handling of needle longer than haystack (empty `windows` → `None`, no panic),
  empty needle short-circuits in `search` before reaching it.
- Ranking: `filter_map` preserves catalog order and `sort_by_key` is stable, so the
  documented tie-break is real; verified by mutation (reverse sort detected).
- `Command<A>` being generic with no `Copy` bound accidentally makes the canonical
  4-command set hard to extend without touching signatures — a nice property.
- Empty palette, `get` for out-of-boundary IDs, oversized queries: all covered and
  confirmed by run.

## Verdict

PASS_WITH_FOLLOWUPS. All acceptance criteria from the lane prompt are met; every gate
was re-run by the reviewer from clean state and passes (367 passed, 0 failed, 1
pre-existing ignore); mutation testing confirms the suite bites; no ADR 0003 violation,
no deletions, no leaks, no panics, no unbounded growth. Three MINOR findings — two test
tightenings and one handoff arithmetic slip — none blocking; MINOR-1 and MINOR-2 are
natural candidates for the wiring lane or a follow-up.
