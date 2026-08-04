# Terminal adversarial sweep — GLM (standing in for the Kimi lane)

Reviewed at: `53497cb` (on `agent/terminal-adversarial`, off `origin/main` @ `43125d2`)
Test command and result: `cargo test --workspace` → **135 passed, 0 failed, 0 ignored, 0 measured** (suite `tests/adversarial.rs` contributes 23; pre-existing baseline was 112). `cargo clippy --workspace --all-targets -- -D warnings` clean. `cargo fmt --all` clean.

## Bugs found

None. Every attack executed to completion. No `#[ignore]` reproductions were
needed. Negative evidence is recorded below.

## Attacks that did NOT break it

**Parameter parsing (`parser.rs`)**
- `ESC[999999999;999999999H` and the same magnitude on every count-accepting
  final (`A B C D S T L M X @ P`): the `u16` accumulator saturates at 65535 and
  cursor/scroll counts clamp to the grid/region. No overflow, no panic.
  (`enormous_csi_parameters_clamp_instead_of_panicking`)
- 30+ empty parameters (`ESC[;;;;…H`) and 9-parameter SGR (`ESC[1;…;9m`):
  the fixed `[u16; 8]` store sets `overflowed` and drops the whole CSI; the SGR
  pen is left untouched. (`degenerate_parameter_lists_do_not_panic`)
- Lone/leading semicolons (`ESC[;H`, `ESC[;r`) resolve to documented defaults.

**State retention across `feed_bytes` boundaries**
- 13 representative sequences (CUP, DECSTBM+scroll, 1049 enter/leave, SGR,
  ED/EL, OSC+BEL, DECCKM/DECKPAM, `ESC(B`, `a\tb`, DECSC/DECRC) fed one byte
  per call produce byte-identical snapshots to a single whole-feed call.
  (`every_sequence_survives_byte_at_a_time_feeding`)
- OSC terminated by the ST string terminator (`ESC ] … ESC \`) survives a split.
  (`split_osc_terminated_by_string_terminator_survives`)
- A CSI split by a `resize()` in the middle (`ESC[2;` → resize → `3HZ`)
  completes correctly: the parser state is independent of the screen, so the
  resize does not corrupt the in-flight sequence. (`mid_sequence_resize_does_not_corrupt_the_parser`)

**Invalid UTF-8**
- Lone continuations (0x80/0xBF), overlong forms (2/3/4-byte), surrogate
  halves (U+D800/U+DFFF), invalid leads (0xFE/0xFF), and truncated sequences are
  all dropped in `Ground` (`_ => None`); valid ASCII renders correctly
  afterward. Bytes ≥0x80 interleaved between ASCII (`A 0xFF B 0xC0 0xAF C`) are
  individually dropped, yielding `ABC`. (`invalid_utf8_bytes_are_dropped_without_panicking`,
  `high_bytes_interleaved_with_ascii_are_individually_dropped`)

**Scroll regions**
- Inverted (`ESC[4;2r`), single-row (`ESC[3;3r`), and over-wide
  (`ESC[1;99r`) ranges are rejected by `ScrollRegion::checked` with the prior
  region preserved; the public `set_scroll_region` agrees and returns
  `InvalidScrollRegion` without mutating. (`inverted_and_degenerate_scroll_regions_are_rejected_and_preserve_state`)
- Saturating scroll counts (`ESC[65535S/T/L/M`) inside a valid region clamp to
  the region height and stay in bounds. (`hard_scroll_inside_a_valid_region_stays_bounded`)

**Alternate screen + resize + DECSC/DECRC**
- 30 iterations of enter → DECSC → resize (varying 2–4 rows / 8–11 cols) →
  DECRC → leave keep modes consistent and invariants intact each iteration; the
  primary top row survives. (`alternate_screen_thrash_with_resize_and_cursor_save_restore`)
- DECRC with no prior DECSC is a no-op, not a panic. (`decsc_with_no_prior_save_is_a_no_op_not_a_panic`)

**Resize extremes and the cell cap**
- Resize to 1×1 followed by overflow printing scrolls in place; the last cell
  holds the last char. (`resize_to_one_by_one_then_print_and_overflow`)
- 1024×1024 (== `MAX_SCREEN_CELLS`) is accepted; 1024×1025, `0×N`, and `N×0`
  are rejected with state unchanged. (`the_cell_cap_holds_at_the_boundary_and_rejects_overflow`)
- A 50-step resize storm across 1–5 rows / 1–6 cols keeps the grid consistent
  every step. (`rapid_resize_storm_keeps_state_consistent`)

**Unbounded-growth boundary**
- 50 000-byte unterminated escape (`ESC` + intermediates) and unterminated OSC
  payloads store nothing and leave the screen at `rows*cols`; after a final
  byte/BEL, subsequent output renders normally.
  (`long_unterminated_escape_does_not_accumulate_or_break_state`,
  `long_unterminated_osc_does_not_accumulate_or_break_state`)
- 50 000 naked `ESC` bytes followed by a real CSI do not desync the parser.
  (`long_run_of_naked_escapes_does_not_desync`)
- A 200 000-byte printable flood into a 2×4 grid leaves `cells().len() == 8`.
  (`high_volume_printable_input_never_grows_the_grid`)
- A 100-iteration mix of every hostile construct keeps `cells().len()` at the
  grid bound. (`hostile_output_never_grows_the_screen_beyond_the_grid`)

**Tab handling (freshest code)**
- Tab in a 1×1 grid, repeated tabs clamping at the right edge, a tab while
  `wrap_pending`, and a tab on column 65534 of a 65535-column grid (where the
  next-stop product `65536` would overflow `u16`) all execute without panic;
  the `usize`/`u16::try_from` clamp holds. (`tab_in_a_one_column_grid_clamps_without_panicking`,
  `tab_at_and_past_the_right_edge_clamps_and_keeps_wrapping_sound`,
  `tab_on_the_widest_permitted_grid_does_not_overflow`,
  `tab_then_print_then_scroll_round_trip_is_sound`)

## Note on one initially-failing assertion
The first run surfaced one failure in `mid_sequence_resize_does_not_corrupt_the_parser`.
Root cause was an off-by-one in the *test's* expected coordinates (CUP is
1-based and the printed `Z` then advances the cursor one column), **not** a
state-machine defect — the partial CSI survived the resize exactly as required.
The assertion was corrected; the parser behavior was unchanged.
