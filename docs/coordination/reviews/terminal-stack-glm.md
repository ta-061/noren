# Terminal stack review — GLM (Rust core)

Reviewed at: `fd1ea69` (cumulative tip: application input modes over alternate
screen / SGR / erase / scroll regions / parser).

Scope: `crates/noren-terminal/src/{state,parser,attributes}.rs` plus the
`noren-app` input encoder (`src/input.rs`) that consumes DECCKM/DECKPAM.

## Verdict
**BLOCK** — one parser soundness defect leaks printable bytes to the screen on
input that virtually every real program emits. The panic/overflow/growth
boundaries are sound; the merge blocker is a byte leak in the escape state
machine, not a memory-safety issue.

## Findings

### BLOCKER — `ESC <intermediate> <final>` leaks the final byte as printable text
- Location: `crates/noren-terminal/src/parser.rs:157` (the `_ =>` arm of
  `advance_escape`), interacting with `advance_ground` at `parser.rs:119`.
- Input that triggers it: `b"\x1b(B"` (i.e. `ESC ( B`, the SCS sequence that
  selects the USASCII G0 charset). Also `b"\x1b#8"` (DECALN), `b"\x1b)0"`,
  `b"\x1b F"`, and in general any `ESC` + intermediate (`0x20..=0x2f`) +
  final (`0x30..=0x7e`).
- Why it is wrong:
  - Trace of `b"\x1b(B"` on a fresh `TerminalState`:
    1. `ESC` → `Ground`→`Escape` (`parser.rs:112`).
    2. `(` (`0x28`, an intermediate) is fed to `advance_escape`. It matches no
       arm, falls to `_ => self.state = ParserState::Ground` (`parser.rs:157`),
       returns `None`. The `(` is consumed.
    3. `B` (`0x42`) is now processed in `Ground`, where `0x20..=0x7e =>
       Some(Action::Print(byte))` (`parser.rs:119`) emits `Print('B')`.
  - Expected (xterm / ECMA-48 state machine): `ESC` enters escape entry,
    intermediate bytes (`0x20..=0x2f`) are collected, then a single final byte
    (`0x30..=0x7e`) terminates the sequence. The whole `ESC ( B` is consumed
    and produces no printed output (the SCS designation is honored; here it is
    an unsupported no-op that must still be swallowed whole).
  - Actual: a spurious `B` is written at the cursor and the cursor advances.
  - Impact: `ESC ( B` (and its G1/DEC-graphics siblings) are emitted by
    bash/zsh init, ncurses, vim, and essentially every terminfo-driven
    program. Every such emission prints one garbage character, corrupting the
    visible grid in any real session. No existing test exercises `ESC` +
    intermediate (verified: no `\x1b(`/`\x1b#`/`\x1b ` appears in `tests/`),
so the green CI does not cover this path.
- Suggested fix: handle intermediate bytes in the escape state instead of
  returning to `Ground` immediately. Minimal change — add an
  `EscapeIntermediate` state:
  ```rust
  // in advance_escape:
  0x20..=0x2f => { self.state = ParserState::EscapeIntermediate; }
  _ => self.state = ParserState::Ground,   // unknown single-byte C1/Fp/Ft finals
  // new state consumes further intermediates and one final byte (0x30..=0x7e)
  // without emitting anything, then returns to Ground.
  ```
  Note the `Csi` path already does this correctly (`parser.rs:221` sets
  `ignored = true` and stays pending through intermediates), so only the
  bare-`ESC` path needs the fix.

### MAJOR — Horizontal Tab (HT, `0x09`) is silently dropped
- Location: `crates/noren-terminal/src/parser.rs:110-122` (`advance_ground`
  has an arm for `0x0a/0x0b/0x0c`, `0x0d`, `0x08`, but none for `0x09`; it
  falls through to `_ => None`).
- Input that triggers it: `b"a\tb"` on any terminal.
- Why it is wrong:
  - Expected (xterm): HT advances the cursor to the next tab stop (default
    every 8 columns), so `a\tb` renders `a` at column 0 and `b` at column 8.
  - Actual: the `0x09` byte yields `None`; the cursor does not move. `b` is
    then printed at column 1, producing `ab`.
  - HT is in the same C0 family as the LF/VT/FF controls the crate *does*
    honor (`parser.rs:116`), and it is not listed among the explicitly
    deferred features in `lib.rs`. Common output (`ls`, `git log`, `printf
    '\t'`, dialog, tab-aligned menus) is misrendered.
- Suggested fix: add a `Tab` action in `advance_ground` (`0x09 => Some(Action::Tab)`)
  and move the cursor to the next multiple of 8 (clamped to `cols - 1`),
  clearing `wrap_pending`. If HT is intended to be deferred, document it
  alongside the other deferred controls so callers know output is non-conformant.

### MINOR — DECSTBM rejects an out-of-range bottom instead of clamping it
- Location: `crates/noren-terminal/src/state.rs:128-133` (`ScrollRegion::checked`),
  reached via `apply_scroll_region` at `state.rs:792-799`.
- Input that triggers it: on a 5-row terminal, `b"\x1b[1;6r"`.
- Why it is wrong:
  - Expected (xterm): the bottom margin is clamped to the last screen line,
    yielding region `(0,4)`; DECSTBM only ignores the sequence when
    `top >= bottom` after clamping.
  - Actual: `checked` rejects because `bottom (5) >= rows (5)`; the entire
    DECSTBM is dropped and the previous region is retained. A program that
    computes the margin one row past the actual size (off-by-one against
    `TIOCGWINSZ`) silently fails to set the region it intended.
  - This appears deliberate (the test at `tests/scroll_regions.rs:45` asserts
    `\x1b[2;99r` keeps the old region), but it diverges from xterm's clamp
    behavior, which the review brief explicitly asks about.
- Suggested fix: clamp `bottom = bottom.min(rows - 1)` (and similarly
  `top = top.min(rows - 1)`) before the `top >= bottom` check in `checked`,
  matching xterm.

### MINOR — Embedded C0 controls inside CSI/Escape are swallowed, not executed
- Location: `crates/noren-terminal/src/parser.rs:233` (`_ => CsiAdvance::pending()`
  in `Csi::advance`) and `parser.rs:157` (same pattern in `advance_escape`).
- Input that triggers it: `b"\x1b[1\n2A"`.
- Why it is wrong:
  - Expected (DEC VT / xterm): a C0 control embedded inside a control
    sequence is executed immediately without aborting the sequence; the LF
    moves the cursor down, then `CSI 2 A` moves it up two.
  - Actual: the `0x0a` matches no arm in `Csi::advance`, returns `pending()`,
    and the LF is lost; only `MoveUp(2)` executes.
  - Real-world impact is low (well-behaved emitters do not interleave C0
    controls inside CSI), so this is informational.
- Suggested fix: in `Csi::advance`, dispatch embedded `0x00..=0x1f` bytes
  (except CAN/SUB/ESC) to the Ground action before continuing to collect
  parameters, mirroring the VT `execute` transition.

## Areas checked and found sound
- **No panics or overflows on untrusted input.** Every indexing path goes
  through the bounds-checked `ScreenBuffer::index` (`state.rs:324`) or a
  validated cursor; every subtraction (`cols - 1`, `rows - 1`, `row_end -
  start`, `end - shift`) is guarded by a prior bound/min check; all parser
  arithmetic uses `saturating_mul`/`saturating_add` (`parser.rs:202-207`).
  Fuzzing-style CSI (huge params, `>8` params → `overflowed`, many `;`) and
  oversized scroll counts (`count.min(region.height())` at `state.rs:251,261`)
  stay in range.
- **No unbounded growth.** Screen is capped at `MAX_SCREEN_CELLS`
  (`state.rs:8,927`); at most one `primary_screen` is retained
  (`enter_alternate_screen` early-returns while alternate is active,
  `state.rs:812`); parser state is fixed-size (`[u16; 8]`, `MAX_CSI_PARAMS`);
  OSC payloads are consumed, never stored (`parser.rs:91-106`).
- **Parser stays in sync across chunk boundaries.** State lives in
  `Parser::state` and persists across `feed_bytes` calls; split CSI/OSC
  sequences resume correctly (covered by
  `tests/terminal_state.rs::split_sequences_are_retained...`). `ESC` correctly
  aborts an in-progress CSI (`parser.rs:79-82`).
- **Erase ops at region edges.** ED/EL ignore margins as the spec requires;
  ICH/DCH clamp to the remainder of the line (`state.rs:299,309,319`); IL/DL
  are correctly scoped to `[cursor_row, region.bottom]` and ignored when the
  cursor is outside the region (`state.rs:708-736`). Boundary at the bottom
  margin blanks the cursor line as expected.
- **SGR parsing, including 256/truecolor and colon forms.** Extended colors
  (`38/48/58`) consume the right number of sub-parameters via
  `extended_color_parameter_count` (`state.rs:835`) and do **not** leak
  channel values (`1/4/7`) into bold/underline/reverse flags. Colon
  sub-parameter forms set `ignored` at `parser.rs:221` and drop the whole CSI
  safely rather than mis-parsing it. Parameter overflow drops the SGR.
- **DECCKM / DECKPAM key encoding.** `CSI ?1h` → application cursor keys →
  `ESC O x`; `CSI ?1l` → normal → `ESC [ x` (`input.rs:165-176`,
  `state.rs:805`). Keypad SS3 letters (`p..y`, `n`, `k/m/j/o`, `M`) match the
  VT220 application-keypad map (`input.rs:179-202`).
- **DECSC/DECRC cursor across the alternate screen.** `saved_cursor` is
  per-`ScreenState`; entering 1049 saves the primary cursor, leaving 1049
  restores it, and a DECSC performed inside the alternate does not leak back
  to the primary (`state.rs:811-832`). (SGR is global by design and is not
  part of DECSC here — a documented simplification, not a defect.)
- **Scroll-region scrolling.** IND/RI/NEL/SU/SD scroll only within the
  margins and move the cursor correctly when outside them (`state.rs:638-673`).
