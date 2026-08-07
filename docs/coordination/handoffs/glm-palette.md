# Handoff: M3-6 command palette (GLM lane)

Branch: `agent/m3-command-palette` (off `origin/main`). Not pushed.
Lane: command palette (M3-6). Engine: GLM 5.2 via opencode.

## Summary

A renderer-independent command palette **model** in `crates/noren-app/src/palette.rs`
plus independent verification in `crates/noren-app/tests/palette.rs`. The model is a
searchable catalog of commands identified by **stable IDs**, their labels, and their
dispatchable actions. It describes *what* to show, never *how* to paint it: no colors,
geometry, or widget types cross this boundary.

## Files (file lease respected)

| File | Status |
| --- | --- |
| `crates/noren-app/src/palette.rs` | new |
| `crates/noren-app/tests/palette.rs` | new |

No other files were touched. In particular `lib.rs`, `main.rs`, `actions.rs`,
`sidebar.rs`, `Cargo.toml`, and `Cargo.lock` are untouched — the export wiring
(`pub mod palette;` in `lib.rs`, replacing the test's `#[path]` shim with a
`noren_app::palette` path, and binding the action type) is an explicitly separate
serial commit owned by another lane.

Because the module is not yet wired into `lib.rs`, the integration test reaches it
with `#[path = "../src/palette.rs"] mod palette;`. That line is the **only** edit
needed at wire-up time, after which the test switches to `use noren_app::palette;`.

## Boundary (ADR 0003)

The palette offers **session and sidebar** commands only. The canonical catalog is
assembled by `Palette::noren(..)` and contains exactly four stable commands; there
is, by construction, **no** pane, tab, split, or layout command:

| Stable ID | Label | Action |
| --- | --- | --- |
| `session.create` | New Session | create a session |
| `session.select` | Switch Session | select an existing session |
| `session.close` | Close Session | close a session |
| `sidebar.focus` | Focus Sidebar | focus a sidebar entry |

A test asserts that no canonical command id or label contains `pane`, `tab`,
`split`, or `layout`, so the boundary is enforced by the suite, not only by prose.

## Actions — no parallel enum

`Command<A>` is **generic over the action type**. The palette defines no session or
sidebar action enum of its own, so it cannot drift into a parallel vocabulary. At
wire-up, `A` is bound to the shared action type that reuses `SessionAction` from
`docs/coordination/decisions/D-M3-001-session-api.md` (which did not yet exist on
`main` when this lane ran). `Palette::noren(create, select, close, sidebar_focus)`
takes the four dispatchable actions from the caller — the wiring layer supplies the
real `SessionAction`/sidebar values, and the palette only assigns the stable IDs and
labels.

## Matching

**ASCII case-insensitive substring**, chosen over fuzzy and documented in the module.
Ranked by earliest match position in the label, ties broken by catalog order (stable
sort). The query is matched **literally** — never regex or glob — so the three
required edge cases behave definedly:

- **Empty query** → every command, in catalog order (match index 0).
- **No-match query** → empty result.
- **All-escaped / all-special-char query** (e.g. `\( \)\[\]\.\*\+\\`) → searched
  verbatim; matches nothing here and cannot panic (asserted under `catch_unwind`).

Non-ASCII case folding is intentionally out of scope; only ASCII letters fold, so no
unbuilt capability is implied. Match indices count `char`s, not bytes, so they stay
valid under UTF-8.

## Gate (real output, this branch)

```
$ cargo fmt --all --check
(exit 0)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile ... (exit 0)

$ cargo test --workspace
... test result: ok. (367 passed; 0 failed; 1 ignored) across the workspace
(exit 0)
```

`tests/palette.rs` contributes 14 new passing tests; the previously-passing 177 on
`main` are unaffected (full workspace now 367 passed + 1 ignored). The single ignored
test is the pre-existing `system_clipboard_round_trips_user_text`, ignored because it
touches the real macOS clipboard — unrelated to this lane.

## Public API delivered

- `CommandId` — opaque stable ID newtype (`&'static str`), with `SESSION_CREATE`,
  `SESSION_SELECT`, `SESSION_CLOSE`, `SIDEBAR_FOCUS` constants, plus `as_str`,
  `AsRef<str>`, and `Display`.
- `Command<A>` — `{ id, label, action }` with `id()`, `label()`, `action()`,
  `into_action()`.
- `SearchHit<'a, A>` — `{ command, match_index }`.
- `Palette<A>` — `new`, `default`, `from_commands`, `noren`, `len`, `is_empty`,
  `iter`, `get(id)`, `search(query)`.
- Private `substring_index` — the matcher.

## Notes for the wiring commit (separate lane)

1. Add `pub mod palette;` to `crates/noren-app/src/lib.rs`.
2. In `crates/noren-app/tests/palette.rs`, drop the `#[path]` line and `use
   noren_app::palette;` instead.
3. Bind `A` to the shared session/sidebar action type once `SessionAction`
   (`D-M3-001`) lands; construct the catalog via `Palette::noren(..)` with the real
   actions.
4. Dynamic per-session entries (selecting/closing a *specific* session id) attach
   their id to the **action**, not to the stable `CommandId` — the id stays
   `"session.select"`/`"session.close"` so keybindings remain stable.
