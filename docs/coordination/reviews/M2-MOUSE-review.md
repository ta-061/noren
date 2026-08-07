# Independent review — M2 mouse input encoder (`glm-mouse`)

Independent review. I did not author this code; I reviewed the head from scratch
rather than trusting the author handoff. Where the handoff's claims are quoted
below, each was re-verified by actually running the gate, mutating the
implementation, and probing interactions the author did not test.

- Reviewed at: `99a5b788ac23` on `agent/mouse-input-encoder`, off `origin/main` @
  `91a0536` (macOS arm64).
- Authority note: the review prompt names `state/tasks/M2-MOUSE.md` in the fleet
  repo as the spec, but **that file does not exist** (`ls state/tasks/` contains
  only M3 tasks; `grep -rl M2-MOUSE` over the fleet repo finds only the review
  prompt itself and queue/lock metadata). The lane prompt
  `prompts/glm-mouse-m6.md` (Goal + Required tests + Gate) is therefore used as
  the authoritative acceptance criteria; it is the contract the author lane was
  given.
- Scope of diff (`git diff --name-status origin/main...HEAD`): 3 files, all `A`
  (additions only, +1283 / −0):
  `crates/noren-app/src/mouse.rs`, `crates/noren-app/tests/mouse_encoding.rs`,
  `docs/coordination/handoffs/glm-mouse.md`.
- Note on worktree: the review prompt's `git checkout agent/mouse-input-encoder`
  was run from `pool-mouse` because the branch was already checked out there
  (worktree lock; git refuses the branch in two worktrees) — same commit, clean
  tree.

## Gate output (actually run at `99a5b78`)

```
$ cargo fmt --all -- --check
    exit 0 (no diff)
$ touch crates/noren-app/src/mouse.rs crates/noren-app/tests/mouse_encoding.rs
$ cargo clippy --workspace --all-targets -- -D warnings
    Checking noren-app v0.1.0 (.../pool-mouse/crates/noren-app)
    Finished `dev` profile [unoptimized + debuginfo]  → exit 0, 0 warnings
$ cargo test --workspace
    exit 0 — per-target sum: 84+24+36+30+19+10+45+23+20+7+6+3+9+6+6+9+6+17
    +25+6+7+22+4 = 424 passed; 0 failed; 1 ignored (pre-existing, in the
    84-test lib target). mouse_encoding: "test result: ok. 36 passed;
    0 failed".
```

Clippy-genuineness check: because `mouse.rs` is compiled **only** through the
`#[path]` include in the `mouse_encoding` test target, I verified clippy really
lints it — injected `let unused_canary = 1u32;` into `mouse.rs`, and re-ran the
gate: `error: unused variable: unused_canary … could not compile noren-app (test
"mouse_encoding") due to 1 previous error`, exit 101. Canary reverted; tree clean
(`git status --porcelain` empty). The clean gate above is real, not cache.

Totals reconcile with the handoff's arithmetic (388 baseline + 36 = 424), though
the handoff's Identity line says "424 … at branch point" — see Observations.

## Acceptance criteria (lane prompt `glm-mouse-m6.md`), one by one

1. **"Track which mouse modes the application turned on" (Zellij client enables
   1000/1002/1003/1006/1015). — MET.** `MouseModes` (`mouse.rs:88-95`) tracks
   exactly 1000/1002/1003/1005/1006/1015; `set(mode, on)` (`mouse.rs:161-171`)
   is the DECSET/DECRST entry point, unknown numbers return unchanged
   (`set_toggles_modes_off_and_ignores_unrelated_numbers`, mode 1049). The mode
   set matches `docs/compatibility/zellij.md:296` ("client requests modes
   1000/1002/1003/1015/1006", inner grid uses encodings 1005/1006).
2. **"Tracking modes decide *whether*; encoding modes decide *how*." — MET.**
   `is_tracked()` gates emission at the top of `encode` (`mouse.rs:403-405`);
   `is_motion_tracked()`/`is_any_event()` gate motion (`mouse.rs:427-435`); the
   format is chosen afterwards (`mouse.rs:441-451`). Mutation M3 below proves the
   1002 hover gate bites; `no_tracking_emits_nothing_for_every_kind` proves the
   outer gate.
3. **"Prefer SGR (1006) when enabled; X10 fallback only when no parameterized
   encoding is active." — MET.** Precedence chain SGR → urxvt → X10
   (`mouse.rs:441-451`); fixed so callers cannot pick a broken combo.
   `sgr_takes_precedence_over_urxvt_when_both_enabled` plus scratch probes C5
   (urxvt beats 1005 fall-through) confirm the order.
4. **"Report press, release, drag (only under 1002/1003), and wheel." — MET.**
   Byte-exact for all four kinds in SGR, urxvt, and X10 forms; drag gating
   verified in both directions (`plain_one_thousand_reports_no_motion_at_all`,
   `button_event_one_thousand_two_drops_motion_without_a_button`,
   `any_event_one_thousand_three_reports_motion_without_a_button`).
5. **"Coordinates 1-based, clamped to the grid; an out-of-range coordinate must
   never be emitted." — MET.** `clamp_to_grid` (`mouse.rs:487-491`) clamps 0-based
   input to `[0, cols-1]×[0, rows-1]` then adds 1. Edge tests
   (`coordinates_clamp_at_the_right_and_bottom_edges`,
   `one_by_one_grid_clamps_everything_to_cell_one_one`) plus my hostile probes
   C1 (`u32::MAX` coordinates) and C9 (65535×65535 grid, 5-digit SGR
   coordinates) all clamp and never panic. The chosen rule for the X10 223 limit
   is **drop** (`X10_MAX_COORD = 223`, `mouse.rs:447-449`), documented in the
   module header (`mouse.rs:43-48`) per the lane prompt's "say which rule you
   chose" requirement; SGR/urxvt are decimal and unaffected.
6. **"With no tracking mode enabled, emit nothing at all." — MET.** Gate at
   `mouse.rs:403-405`; tested with encodings on but tracking off for every event
   kind (`no_tracking_emits_nothing_for_every_kind`).

Required tests from the lane prompt: all six present and byte-exact
(`tests/mouse_encoding.rs`, 36 tests; spot-verified against xterm forms —
SGR `CSI < Cb;Cx;Cy M`/`m`, urxvt `CSI Cb;Cx;Cy M`, X10 `CSI M` + three
`+32`-offset bytes; modifier bits 4/8/16; motion 32; wheel 64/65).

## Mutation testing (each reverted; tree clean after every revert)

| Mutation | Result |
| --- | --- |
| M1: SGR release terminator forced to `M` (`mouse.rs:502`) | **2 FAILED** (`sgr_release_left_uses_lowercase_m_and_keeps_button_code`, `sgr_release_middle_keeps_button_one`) |
| M2: column clamp removed (`clamp_to_grid` emits `col + 1`) | **2 FAILED** (`coordinates_clamp_at_the_right_and_bottom_edges`, `one_by_one_grid_clamps_everything_to_cell_one_one`) |
| M3: 1002 hover gate removed (`is_any_event` check → `false`) | **2 FAILED** (`button_event_one_thousand_two_drops_motion_without_a_button`, `one_thousand_two_without_sgr_drops_hover_in_x10_form`) |
| M4: legacy release keeps the button code instead of collapsing to 3 | **2 FAILED** (`x10_release_collapses_every_button_to_cb_three`, `urxvt_release_collapses_to_cb_three`) |
| M5: motion flag 32 → 64 | **6 FAILED** (all drag and hover tests across SGR/X10/urxvt) |
| M6: Shift bit 4 → 2 | **3 FAILED** (`sgr_modifier_bits_fold_into_cb`, `sgr_drag_with_modifier_combines_motion_and_modifier_bits`, `x10_middle_press_and_shift_modifier_byte_forms`) |
| M7: X10 drop rule replaced by saturation at 223 | **3 FAILED** (both drop tests + `x10_reports_at_the_two_hundred_twenty_three_boundary`) |

Seven independent mutations, seven bites. The suite is not vacuous.

## Regressions, boundaries, and combinations the author did not test

I compiled a scratch `#[path]` harness
(`tests/reviewer_scratch_mouse.rs`, created then deleted; tree left clean) with
11 interaction probes — `ok. 11 passed; 0 failed`:

- **C1 hostile coordinates:** press at `(u32::MAX, u32::MAX)` on 80×24 clamps to
  `Cx=80, Cy=24`, no panic.
- **C2 largest X10 `Cb`:** wheel-down + Shift/Alt/Ctrl → `Cb = 65+28 = 93`,
  byte `125` (`\x1b[M\x7d\x2a\x25`); the handoff's "93 fits a byte" claim holds
  and the `debug_assert` (`mouse.rs:526-527`) does not fire.
- **C3 mid-stream mode transition (DECSET → event → DECRST → event):** SGR on,
  press emits `\x1b[<0;10;5M`; then `set(MODE_SGR, false)`; the release emits
  the X10 form `\x1b[M\x23\x2a\x25` (Cb 3). The stateless encoder composes
  correctly with the future `set()`-driven wiring across a transition — the
  author's tests only exercised static mode sets.
- **C4 urxvt + wide grid:** col 230 on 300×50 reports `\x1b[0;231;5M` — the
  223 limit is X10-only; the author proved this only for SGR.
- **C5 urxvt beats the 1005 fall-through** on a wide grid: `\x1b[0;231;5M`,
  no drop.
- **C6 hover under combined 1002+1003** (X10 form): reports, `Cb = 35`
  (`\x1b[M\x43…`), because `any_event` wins.
- **C7 release under 1003-only** carries no motion bit (`Cb = 3`).
- **C8 press under 1003-only** (no 1000/1002) reports — 1003 subsumes press.
- **C9 maximum grid:** 65535×65535 with SGR emits 5-digit coordinates
  `\x1b[<0;65535;65535M`; `push_decimal` handles the full u32/u16 range.
- **C10 exact boundary grid:** 224-wide grid — col 222 (Cx 223) emits byte
  `\xff`; col 223 (Cx 224) drops. Confirms the drop rule keys off the emitted
  1-based coordinate, not the grid width.
- **C11 statelessness:** 10,000 repeated encodes of the same event return
  byte-identical results; there is no hidden state to drift.

Existing-workspace regressions: none — `cargo test --workspace` passes every
pre-existing target (424 total incl. the 36 new; the 84-test lib target with its
1 pre-existing ignore is unaffected).

## Panics, leaks, unbounded growth

- No `unwrap`/`expect`/`panic!`/`unreachable!`/`todo!`/`unsafe` in `mouse.rs`
  (the only match is a non-panicking `unwrap_or`, `mouse.rs:436`).
- `MouseGrid::new` rejects zero dimensions (`mouse.rs:294-300`), so
  `grid.cols - 1` in `clamp_to_grid` cannot underflow; input coordinates are
  unsigned, so `clamp(0, …)` cannot panic.
- The encoder is a pure function: no fields, no statics, no I/O, no retained
  buffers. Each call allocates one report `Vec` (≤ ~20 bytes; at most one
  reallocation for 5-digit coordinates over the 16-byte capacity) that the
  caller owns. Nothing grows with call count — the 10k-iteration probe (C11)
  confirms stable behavior and bounded output.
- `push_decimal` (`mouse.rs:542-555`): 10-byte stack buffer is exact for u32;
  the loop terminates for `value = 0` via the early branch and for any u32 via
  division. No overflow (`remaining % 10` ≤ 9).

## Unintended deletions, lease, and forbidden files

`git diff --name-status origin/main...HEAD` → three `A` entries, **zero
deletions, zero modifications**. The forbidden files (`lib.rs`, `main.rs`,
`actions.rs`, `passthrough.rs`, `Cargo.toml`, `Cargo.lock`) are untouched, and
`lib.rs` contains no `mod mouse` — the module is unwired exactly as the lease
requires, reachable only through the `#[path]` test. The handoff is a lane
coordination artifact permitted by the prompt.

## Noren/Zellij boundary (ADR 0003)

Respected. `grep -inE 'pane|layout|split|zellij'` over `mouse.rs` and
`mouse_encoding.rs` matches only doc comments about the Zellij client's mode
numbers. The module introduces no pane, tab, layout tree, or split; it neither
reads nor persists any terminal-internal state — it converts app-owned pointer
events into PTY **input** bytes, which is Noren's outside-the-terminal side of
the wire (and the documented inverse of issue #46's output-parser confusion).
No BLOCKER.

## Observations (not defects)

1. The review prompt's spec `state/tasks/M2-MOUSE.md` is absent from the fleet
   repo; the lane prompt was used as authority. If a formal spec exists
   elsewhere, a follow-up re-check against it is cheap (the module surface is
   small).
2. Handoff internal inconsistency: the Identity line says "424 workspace tests
   passing at branch point" while the Commands section says 388 baseline + 36.
   The measured reality is 388 + 36 = 424; the Identity line misstates the
   branch-point number. Doc-only.
3. Mode 9 (X10 press-only) and 1007 (scroll) are real xterm mouse modes that
   `MouseModes::set` treats as unknown (no-op). That is conservative — unknown
   modes can only silence output, never misreport — and both are outside the
   documented Zellij client set (`zellij.md:296`). Worth a line in the future
   DECSET/DECRST wiring lane's notes, nothing more.
4. The `#[path]` include compiles `mouse.rs` only into the test target until the
   serial wiring commit; the handoff's warning ("replace `#[path]` with
   `use noren_app::mouse;`") is correct and necessary then. Not a defect on this
   branch.

## Verdict

PASS — all six acceptance criteria met and byte-verified; 7/7 mutations bite;
11 adversarial/combination probes beyond the author's suite pass; no panics,
leaks, or unbounded growth; zero deletions; lease and ADR 0003 intact. No
BLOCKER, MAJOR, or MINOR defect found; four observations recorded.

`REVIEW_M2-MOUSE verdict=PASS blockers=0 majors=0 minors=0 tests=PASS total=424`
