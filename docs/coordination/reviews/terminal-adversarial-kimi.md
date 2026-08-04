# Terminal adversarial sweep — Kimi (independent second attacker)

Reviewed at: `kimi-adversarial` off `origin/main` @ `e83d8ed`.
Independence note: I did not write the parser code under attack (the
`EscapeIntermediate` state in `5e266d4` is GLM's). I read GLM's report
(`terminal-adversarial-glm.md`) first and deliberately went elsewhere; no GLM
attack was re-run.

Test command and result:
`cargo test -p noren-terminal` → **167 passed, 0 failed, 1 ignored**
(suite `tests/adversarial_kimi.rs` contributes 19 passed + 1 ignored;
pre-existing baseline was 148). `cargo clippy -p noren-terminal --all-targets
-- -D warnings` clean. `cargo fmt --all` clean.
`cargo test --workspace` cannot run on this Linux host because the
winit/wgpu `noren-app` crate is macOS-first — expected, not a finding.

The ignored test is the KBUG-01 reproducer. It was executed explicitly with
`cargo test -p noren-terminal --test adversarial_kimi -- --ignored` and
**fails** (i.e. the bug is real): 0 passed, 1 failed.

## Bugs found

### KBUG-01 — unbounded per-cell memory growth via zero-width combining marks

`TerminalState::attach_zero_width` (`src/state.rs`) appends every zero-width
character to the target cell's owned `String` with **no length cap**. A
hostile PTY stream of `a` followed by N × U+0301 grows a single cell's text
linearly in N while every documented bound holds: cell count stays
`rows*cols`, grid ≤ `MAX_SCREEN_CELLS`, scrollback ≤ `MAX_SCROLLBACK_LINES`.
The memory ceilings documented on `MAX_SCROLLBACK_LINES` ("a hostile program
emitting unbounded output cannot grow history without limit", "~40 bytes/cell
is a safe upper bound") are defeated: bytes/cell is in fact unlimited.

Measured (executed, not extrapolated): `a` + 200 000 × U+0301 leaves
`cell(0,0).text().len() == 400_001` bytes; on a 2×2 grid with wrap pending,
the same flood lands on the armed last-column cell (`cell(0,1)` = 400 001
bytes). The vector is reachable both in normal cursor flow and via the
wrap-pending attach path, and the inflated cells are also copied into
scrollback and snapshots, multiplying the cost.

Reproducer: `tests/adversarial_kimi.rs`,
`combining_marks_grow_a_single_cell_without_bound`,
marked `#[ignore = "reproduces KBUG-01"]`. Not fixed — reported only, per
lane rules. Suggested direction: cap the number of combining characters
attached to one cell (a grapheme-cluster budget), dropping the excess.

## Behavioral observations (not robustness bugs; for the compat lane)

Both verified by execution in a scratch probe (since removed):

- **DL/SU at row 0 feed scrollback.** On a 3-row primary with a full-screen
  region, cursor home, `ESC[M` pushed the top row into scrollback, and a
  following `ESC[S` pushed the next one. Deleted lines thus enter history,
  and DL/IL are asymmetric (IL cannot push). Whether this matches xterm is a
  compatibility question, not a crash.
- **Leaving the alternate screen clears a pending wrap on the primary.**
  `leave_alternate_screen` runs `restore_cursor`, which unconditionally sets
  `wrap_pending = false`, so `print-to-right-edge → 1049h → 1049l → print`
  overwrites the last column instead of wrapping. If xterm preserves
  `do_wrap` across mode 1049 this is a rendering divergence; it is pinned as
  current behavior in
  `alternate_screen_round_trip_restores_content_but_clears_pending_wrap`.

## Attacks that did NOT break it (all novel vs. GLM's sweep)

**Scrollback under feature combinations — GLM never fed scrollback at all**
- 50 rounds of 1049 enter → scrolling output + DECSC/DECRC + region-local
  scroll → leave: scrollback and visible primary are **byte-identical** to
  the pre-thrash snapshot.
  (`scrollback_is_byte_identical_after_alt_screen_decsc_decrc_thrash`)
- `MAX_SCROLLBACK_LINES + 100` lines of wide characters evicted through a
  1-row grid: count clamps exactly at the cap and the newest retained row
  keeps full lead/continuation fidelity.
  (`scrollback_stays_capped_with_wide_character_lines_and_keeps_pairs`)
- Resize storm (3×8 → 2×3 → 4×11 → 1×2 → 2×8) between scroll batches:
  mixed retained widths render without panic, stay append-ordered, and keep
  their historical widths (no reflow).
  (`scrollback_of_mixed_widths_survives_a_resize_storm_between_batches`)

**`EscapeIntermediate` (GLM's own freshest code), attacked exhaustively**
- All 16 intermediates (0x20..=0x2f) × all 79 possible finals (0x30..=0x7e) —
  1264 sequences — every final is consumed; only a trailing `X` prints, at
  column 1. (`every_escape_intermediate_final_byte_is_consumed_without_leaking`)
- Stacked intermediates (`ESC ( ) # SP`), ESC-restarts mid-intermediate, and
  DEL/NUL bytes inside the state never leak a final byte.
  (`stacked_intermediates_and_esc_restarts_never_leak`)
- `ESC` aborting a partial CSI followed by an SCS-like sequence, then a real
  CUP, lands exactly. (`escape_aborting_csi_then_intermediate_sequence_keeps_later_csi_working`)
- OSC split by a `resize()` (GLM split only a CSI): the string state is
  independent of the screen; BEL terminates and the next byte prints on the
  resized grid. (`mid_osc_resize_does_not_corrupt_the_string_state`)

**Determinism beyond byte-at-a-time**
- A compound script (SGR + wide chars + region + index + tab-at-edge + OSC +
  1049 round trip + DECSC/DECRC + combining mark + invalid UTF-8 + aborted
  CSI) fed whole vs. in chunks of size k for **every** k in 1..=len: all
  snapshots identical. GLM checked byte-at-a-time on 13 simple sequences;
  this checks all partitions of one interacting script.
  (`compound_hostile_script_is_deterministic_across_all_chunkings`)

**Wide characters × controls (combinations, not single ops)**
- A tab stop that lands exactly on a continuation cell snaps forward off the
  pair; the next print arms the wrap with all pairs intact.
  (`tab_landing_on_a_continuation_cell_snaps_forward_off_the_pair`)
- 40 rounds × 13 ops (SU/SD/IND/RI/IL/DL/ICH/DCH/ECH/EL0/EL2/CUF/CUD) over a
  wide-char grid inside a narrow scroll region: the lead/continuation
  invariant (asserted through the public cell view, not `debug_assert`) holds
  after every single op. (`scroll_and_edit_storm_never_splits_a_wide_pair`)
- ED 0/1/2 with the cursor on **every** column boundary of wide-char rows:
  no dangling halves. (`erase_in_display_at_every_wide_boundary_leaves_no_dangling_half`)

**`wrap_pending` interactions GLM did not combine**
- EL/DCH/ECH/ICH/ED each cancel a pending wrap; the next printable lands at
  the cursor without wrapping. (`edit_and_erase_ops_cancel_a_pending_wrap_before_the_next_print`)
- Wrap armed on a region's bottom row: the next print scrolls only the
  region, rows outside are untouched, and (region top ≠ 0) nothing enters
  scrollback. (`wrap_at_a_region_bottom_scrolls_only_the_region`)
- Wrap across an alternate-screen round trip: content and cursor position
  round-trip exactly; the pending wrap is cleared (see observations above).

**Idempotence / round-trip via full snapshot equality (stronger than GLM's
size-only checks)**
- Two consecutive alt-screen round trips restore the exact snapshot, SGR pen
  and scroll region included. (`alternate_screen_round_trip_restores_the_exact_snapshot`)
- Double-enter / single-leave of mode 1049 restores the exact snapshot (the
  second enter is a true no-op). (`double_enter_single_leave_restores_the_exact_snapshot`)
- Rejected resize (over-cap, zero-dimension) and rejected DECSTBM (CSI and
  public API) leave the **full snapshot** bit-identical, not just the size.
  (`rejected_operations_leave_the_full_snapshot_unchanged`)

**SGR pen across screens (pinning the documented design)**
- The pen set on the primary writes attributed cells on the alternate screen;
  `ESC[m` on the alternate persists as default after leaving.
  (`sgr_pen_is_terminal_global_across_screen_switches_and_resets_cleanly`)

**Public API surface the suites never call**
- `TerminalState::new` with zero dimensions errors; `cell()` out of bounds
  returns `None` (including `u16::MAX`); public `move_cursor` with `u16::MAX`
  counts clamps; public `save_cursor`/`restore_cursor` round-trips exactly.
  (`public_api_edge_inputs_are_rejected_or_clamped`)

## Note on three initially-failing assertions

First run had three failures; two were wrong expectations in my own tests
(off-by-one in which row a regional scroll evicts; lexicographic ordering
broken by truncation of retained labels after a shrink — both test bugs, the
state machine was right). The third was the wrap-pending-across-1049
deviation, reclassified as a compat observation and pinned as current
behavior (above). No state-machine defect was found in any of the three.
