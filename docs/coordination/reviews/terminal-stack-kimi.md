# Terminal stack adversarial review — Kimi

Reviewed at: `fd1ea69584acbfdf2d0c08debbd148989f3f9f6b` (`pr30`, cumulative tip of the 7-PR terminal stack)

Test command and result:

```
$ cargo test -p noren-terminal
test result: ok. 78 passed; 0 failed; 3 ignored (all 8 test binaries + doc-tests)
  of which tests/adversarial.rs: 19 passed; 0 failed; 3 ignored
$ cargo test -p noren-terminal --test adversarial -- --ignored
test result: FAILED. 0 passed; 3 failed  <- the 3 ignored tests are the bug repros below
```

All 22 new tests live in `crates/noren-terminal/tests/adversarial.rs` on branch
`agent/kimi-adversarial`. The 3 `#[ignore = "reproduces BUG-xx"]` tests fail
when run with `--ignored` (verified), demonstrating the bugs; the other 19
pass against correct behavior. **No panic, hang, or unbounded allocation was
found.** The state machine's arithmetic hardening (saturating CSI params,
`MAX_CSI_PARAMS` cap, `ScrollRegion::checked`, `MAX_SCREEN_CELLS`) held up
everywhere.

## Bugs found

### BUG-01 DCS/SOS/PM/APC string payloads are rendered as screen text (WRONG)
- Reproducing test: `tests/adversarial.rs::dcs_sos_pm_apc_payloads_must_not_be_rendered_as_text`
- Input: `\x1bP1;2|SPOOFED\x1b\\` (also `ESC X` SOS, `ESC ^` PM, `ESC _` APC variants)
- Observed: `Parser::advance_escape` has no DCS/SOS/PM/APC states; `ESC P`
  falls through to `_ => Ground`, so the payload prints:
  `snapshot().lines() == ["1;2|SPOOFED"]`. An ESC inside the payload is also
  re-parsed normally, so control sequences smuggled inside a string sequence
  execute. This contradicts the documented contract on `feed_bytes`
  ("unsupported control sequences are ignored … never … rendered as raw
  escape-sequence payload") and corrupts the screen for any app emitting
  sixel/kitty/tmux-passthrough strings; it also enables screen-content
  spoofing by a malicious program.
- Severity: MAJOR

### BUG-02 Two-byte ESC sequences (charset selects, DECALN) print their payload byte (WRONG)
- Reproducing test: `tests/adversarial.rs::esc_intermediate_sequences_must_not_print_their_final_byte`
- Input: `\x1b(0`, `\x1b(B`, `\x1b)0`, `\x1b%G`, `\x1b#8`
- Observed: the intermediate byte (`(`, `)`, `%`, `#`) is consumed silently
  but the final byte lands in Ground and prints — `ESC ( 0` renders a stray
  `0` (`[27, 40, 48] printed ["0"]`). Zellij, tmux, and vim emit
  `ESC ( 0` / `ESC ( B` for line drawing on essentially every frame, so real
  full-screen apps will be littered with stray `0`/`B` glyphs.
- Severity: MAJOR

### BUG-03 CSI private parameter markers `<` and `=` are silently swallowed; mangled sequences execute as destructive plain CSI (WRONG)
- Reproducing test: `tests/adversarial.rs::csi_lt_and_eq_private_markers_must_poison_the_sequence`
- Input: `\x1b[<2M` (deletes 2 lines), `\x1b[<2J` (erases the display), `\x1b[=2;4r` (rewrites the scroll region)
- Observed: `Csi::advance` recognizes `?`/`>` as private markers and `:` /
  `0x20..=0x2f` as sequence-poisoning bytes, but `<` (0x3c) and `=` (0x3d) —
  which ECMA-48 places in the same 0x30–0x3f private parameter range — fall
  through to `_ => pending()` with no side effect. The sequence then executes
  as if the marker never existed: `ESC [ < 2 M` (SGR-mouse-report-shaped)
  became DL(2) and deleted screen lines (`["AAA","BBB","CCC"]` →
  `["AAA","BBB"]`). A conformant parser must treat these as unknown private
  sequences and ignore them.
- Severity: MAJOR

All three bugs share one root gap: the parser only models CSI and OSC. It
needs string states for DCS/SOS/PM/APC (swallow until ST, mirroring the OSC
handling), must treat unconsumed ESC intermediates (`( ) * + # %` …) as
sequence-poisoning, and must poison CSI sequences carrying `<`/`=` private
parameter bytes. Per the lane rules, no fixes were attempted here.

## Attacks that did NOT break it

Honest negative results — all verified by executed tests:

- **Enormous / negative-looking CSI parameters**: `ESC[999999999;999999999H`,
  `ESC[18446744073709551615;…H`, `ESC[65536;65536H` saturate at `u16::MAX` and
  clamp onto the grid; huge `S`/`T`/`L`/`M`/`P`/`X`/`@` counts stay bounded.
  No wrap-around, no panic. Negative-looking params (`ESC[-5H`) and
  intermediate bytes (`ESC[1 1H`) poison the sequence safely.
- **Parameter-count overflow / empty params**: 512 `;`-separated params trip
  the `MAX_CSI_PARAMS` overflow flag and the whole sequence is discarded;
  empty-param ladders (`ESC[;;;;;;;;H`, `ESC[;;2;;H`) resolve to defaults.
- **SGR extremes**: truncated `38`/`38;2`, exactly-8-param extended colors,
  over-cap sequences dropped; `38;2;1;4;7` channel bytes never leak into
  bold/underline/reverse; reset keeps working.
- **Chunk-boundary splits**: a mixed corpus (DECSTBM, 1049, OSC, SGR 256-color,
  ED, DECSC/DECRC) split at *every* two-part boundary plus full byte-by-byte
  drip produces snapshots identical to one-shot feeding. Parser state is
  byte-driven and split-safe.
- **Inverted/degenerate scroll regions**: `ESC[4;2r`, `ESC[3;3r`, `ESC[5;2r`,
  `ESC[99;1r` are rejected without disturbing the active region; a 20-round
  storm of `999S`/`999T`/`999L`/`999M`/LF/RI with the cursor outside the
  region stays contained; rows outside the region survive.
- **1x1 / 1-row / 1-col grids**: wrap-scroll on height-1 regions and per-byte
  wrapping on width-1 grids work with no underflow.
- **Alternate-screen thrash**: 50 rounds of double-enter/double-leave 1049
  interleaved with DECSC/DECRC and resizes (2x2..6x8) preserve invariants and
  primary content; saved cursors clamp after shrinking resizes, including
  through the alternate screen.
- **Invalid UTF-8**: lone continuation bytes, truncated 2/3/4-byte sequences,
  never-valid lead bytes, and valid non-ASCII UTF-8 are all ignored byte by
  byte; high bytes inside CSI/OSC never desynchronize the parser.
- **Resize extremes**: 0xN/Nx0/0x0 → `InvalidSize` with state untouched;
  beyond-`MAX_SCREEN_CELLS` (incl. 65535x65535) → `ScreenTooLarge` with state
  untouched; exactly 1024x1024 (the bound) works.
- **Unterminated OSC/CSI (memory)**: repeated 8 MiB unterminated-OSC +
  unterminated-CSI-digit feeds leave RSS flat on repeat rounds (min-round
  delta < 4 MiB; the parser is a fixed-size `Copy` struct and allocates
  nothing per byte), and the parser recovers to execute later sequences.
- **Hang-shaped streams**: 2000 rounds of BEL/ST-terminated OSCs, 1049 toggles,
  and ESC-aborted unterminated CSIs complete in linear time.
- **Deterministic hostile fuzz**: 200 terminals × 40 rounds of
  alphabet-biased random bytes (ESC `[` `]` `;` digits finals C0 0x80–0xff)
  fed in random chunk sizes with random (sometimes invalid) resizes and
  direct API pokes (`move_cursor`, `set_scroll_region`, save/restore with
  `u16` extremes) preserve all structural invariants after every round.

## Session notes

Second-pass independent verification (same session branch, 2026-08-05): an
independent re-attack was conducted from scratch against the same commit
before this existing suite was rediscovered on the branch. That pass wrote 24
new hostile-input tests (byte-at-a-time vs one-shot equivalence, random
chunk-boundary equivalence, sustained 16 MiB hostile load with RSS-delta
assertion, 10,000-round alternate-screen entry accumulation, 1024x1024
max-size screen operations, full-grid overwrite storms) — all passed — plus an
8,000-iteration randomized stress fuzz mixing feeds, extreme resizes
(0/1/65535/1024), scroll-region pokes, DECSC/DECRC, and 1049 thrash under
debug assertions and overflow checks: zero panics, zero hangs, zero invariant
violations. The three BUG reproducers above were then re-executed with
`cargo test -p noren-terminal --test adversarial -- --ignored` and confirmed
to fail exactly as reported (0 passed; 3 failed), and the full crate suite
re-confirmed green (78 passed; 3 ignored). No additional bugs were found
beyond BUG-01..03; the findings and negative results above are corroborated.

This review ran in a shared checkout where a concurrent duplicate agent
instance was active on the same task; it published an overlapping suite as
commit `8f7b200` on this branch. All results above were independently
produced and executed in an isolated worktree before being committed on top;
BUG-02 and BUG-03 were re-derived and re-verified from that collision before
inclusion (their reproducers here are this lane's own).
