# Terminal core foundation

- Status: merged. The foundation, the parallel Terminal Core stack (PR
  [#29](https://github.com/ta-061/noren/pull/29)), and scrollback (PR
  [#50](https://github.com/ta-061/noren/pull/50)) are all on `main`.
- Supersedes nothing: the accepted [local-PTY PoC
  architecture](minimal-local-pty-poc.md) still applies

This foundation moves PTY output bytes through a renderer-independent terminal
state core in `crates/noren-terminal/`. It is a foundation slice, not a
VT100/xterm compatibility claim and not vim/tmux/zellij compatibility.

## Data flow

The pipeline is PTY -> Parser -> TerminalState -> Renderer: the PTY layer
delivers bytes, the parser turns them into bounded state changes,
`TerminalState` owns screens, modes, and cursor state, and the renderer consumes
only immutable snapshots.

```text
PTY bytes -> TerminalState::feed_bytes -> parser -> bounded screen/cursor state
TerminalState::snapshot -> TerminalSnapshot -> renderer (lines only)
```

- `TerminalState` owns the active screen, the parked primary screen while the
  alternate screen is active, and the parser state.
- Each visible grid is a fixed-size `ScreenBuffer` of `Cell` values, bounded by
  `MAX_SCREEN_CELLS` (1,048,576 cells).
- `resize` rebuilds the grids, preserves each overlapping top-left region, and
  resets scroll margins and pending autowrap.
- The renderer consumes the immutable `TerminalSnapshot` and never depends on
  PTY or parser types.

## Supported behavior

Merged in PR #19:

- Printable ASCII bytes (`0x20..=0x7e`), line feed, carriage return, and
  backspace.
- A minimal CSI cursor-movement subset: cursor up, down, forward, back,
  absolute position, and absolute column.
- Unsupported and OSC bytes are ignored without corrupting state.

Added by Draft PR #21 (scrolling regions):

- Scroll margins default to the full screen and are inclusive on both ends;
  DECSTBM sets them and rejects invalid ranges.
- LF/VT/FF/IND/NEL/RI and CSI S/T scrolling act only within the active region;
  rows outside the margins are preserved.
- CNL, CPL, and VPA.
- Delayed autowrap: wrapping is deferred until the next printable byte.
- Resize resets margins and pending autowrap.

Added by Draft PR #23 / Issue #22 (alternate screen, stacked on #21):

- `TerminalState` owns an active screen plus a parked primary screen; only
  the active screen receives bytes and appears in snapshots.
- CSI `?1049h` saves the primary cursor, parks the primary screen, and
  enters a blank alternate screen with the cursor homed, full-screen
  margins, and no pending wrap. CSI `?1049l` restores the primary screen
  contents and the cursor saved at entry. Both switches are idempotent:
  entering while already alternate, or leaving while already primary, is a
  no-op. Only `1049` is recognized; other parameter lists or `>` markers
  are ignored.
- ESC 7/8 and parameterless CSI `s`/`u` save and restore the cursor on the
  active screen only. Each screen keeps its own saved cursor (position
  only), and restoring with no saved cursor leaves the cursor unmoved and
  clears pending autowrap.
- `TerminalModes` (DEC private mode 1049) is captured in
  `TerminalSnapshot::modes` so renderers can tell which screen is visible.
- `resize` rebuilds both the active and parked buffers, preserving each
  one's overlapping top-left region, clamps cursors and saved cursors, and
  resets margins and pending autowrap; the `MAX_SCREEN_CELLS` bound is
  checked before either buffer is rebuilt.
- The renderer boundary is unchanged: renderers still consume only the
  immutable `TerminalSnapshot` and never PTY or parser types.

This slice makes `?1049` round-trips reliable; it is not full xterm, vim,
tmux, or zellij compatibility.

## Scrollback

Lines that scroll off the top of the *primary* screen are retained in a bounded
scrollback buffer instead of being discarded, and are readable through the public
renderer-independent API.

- **Bounded.** `MAX_SCROLLBACK_LINES` (10,000) sits next to `MAX_SCREEN_CELLS`
  and is the hard line-count cap. Each retained row owns `cols` cells (the column
  count when it left), so the memory ceiling is
  `MAX_SCROLLBACK_LINES * cols * bytes_per_cell`. `sizeof(Cell) == 32` (a 24-byte
  owned `String` handle, a width byte, and packed attributes) plus the owned text:
  combining marks attached to one cell are capped at
  `MAX_COMBINING_MARKS_PER_CELL` (7), so the text is at most
  `4 * (7 + 1) == 32` bytes (one base `char` plus seven marks, four bytes each in
  the worst UTF-8 case). The inflated worst case is therefore **64 bytes/cell**
  (single-character cells stay near 40). That is ~51 MiB for an 80-column
  terminal and ~164 MiB for 256 columns at the cap; typical single-character text
  is ~32 MiB and ~100 MiB respectively. The line count is the hard bound so
  hostile unbounded output — including a stream of zero-width combining marks,
  which used to grow one cell without bound and now stops at the cap — cannot
  grow history past it. The oldest retained line is evicted when the cap is
  reached.
- **Primary screen only.** Retention is gated on the primary screen being active
  and on the scrolling region starting at row 0. The alternate screen never
  contributes (so `less`/`vim` do not pollute history), and scrolling inside a
  non-screen-aligned margin never reaches scrollback. Entering/leaving the
  alternate screen leaves primary scrollback intact.
- **Capture point.** `ScreenBuffer::scroll_up` returns the rows that left the top
  of the region; `TerminalState::scroll_up_capturing` pushes them to the
  `VecDeque` scrollback only when both gates hold. All three scroll-up paths
  (explicit `CSI SU`, `LF`/`IND`/`NEL` at the bottom margin, and `CSI DL`) route
  through this capture, so wrap-driven and explicit scrolling behave identically.
- **Renderer-independent exposure.** `TerminalSnapshot::scrollback()` returns the
  retained rows as `&[Vec<Cell>]` (oldest first) and `scrollback_lines()` gives a
  trimmed text rendering parallel to `lines()`. `TerminalState::scrollback_len()`
  reports the count cheaply. The renderer never reaches past the snapshot.
- **Resize does not reflow (known limitation).** Reflowing retained history on a
  column change is deliberately out of scope for this slice. Retained rows keep
  the width they had when they scrolled off: growing the grid does not pad them,
  shrinking it does not truncate them, and each row's cell count is the original
  `cols`. This is asserted in the test suite. A future slice may reflow or
  soft-wrap retained rows; until then renderers must tolerate mixed-width
  scrollback rows (each row's `.len()` is its own width).

## Character width model

Printing honors display width, so CJK output, wide emoji, and combining marks
keep the cell grid aligned instead of drifting it one cell per wide character.

- **Decoding.** The parser's Ground state decodes UTF-8 incrementally and emits
  one `Action::Print(char)` per complete character; the decoder keeps no other
  allocation, so split feeds across `feed_bytes` calls are fine. Overlong,
  surrogate, and out-of-range sequences are rejected as a unit; a byte that
  interrupts a pending sequence is re-examined as a new lead; and a pending
  sequence never straddles an escape sequence or control-string payload.
  Invalid bytes never print.
- **Width policy.** `cell_width` delegates to `UnicodeWidthChar` from the pinned
  `unicode-width` crate; no hand-rolled width tables. Characters are zero, one,
  or two columns wide (ambiguous-width characters follow the crate's non-CJK
  default of one).
- **Wide characters.** A two-column character occupies a lead cell
  (`width == 2`, holding the text) and a continuation cell (`width == 0`, empty
  text) that renders as nothing and is never treated as an independent
  character. The grid invariant is: every continuation directly follows its
  lead, and every lead is directly followed by its continuation.
- **Wrap rule.** Delayed autowrap is preserved. A wide character that does not
  fit in the remaining columns wraps whole to the next line instead of being
  split across the right edge. A wide character written flush against the right
  edge leaves the cursor on the lead cell with autowrap pending. A character
  wider than the entire grid is dropped.
- **No dangling halves.** Erase (ED/EL/ECH), insert/delete characters (ICH/DCH),
  overwriting, and resize all enforce the invariant: clearing either half of a
  wide character clears both. `ScreenBuffer::repair_row` blanks any half that
  lost its partner, runs after every grid edit, and `apply` debug-asserts the
  invariant on both screens after every action.
- **Cursor rule.** The cursor never rests on a continuation cell. Forward
  relative motion moves past the whole wide character; absolute positioning,
  backward motion, backspace, restore-cursor, and resize clamping snap onto the
  lead cell.
- **Zero-width characters.** Combining marks are *attached* to the preceding
  cell in the row (skipping continuation cells): they extend that cell's text
  without changing its width, never advance the cursor, and do not clear
  pending autowrap. With no preceding cell (cursor at column zero with no
  pending wrap) the mark is dropped. A hostile stream of marks is bounded by
  `MAX_COMBINING_MARKS_PER_CELL` (7): once a cell carries that many, further
  marks are dropped instead of appended, so the per-cell text cannot be grown
  without bound (KBUG-01). This covers both attach paths — the normal cursor
  path and the wrap-pending path — and the cap propagates to scrollback rows
  and snapshots, which simply observe the already-capped cells. This is a
  documented simplification: the grid stores capped per-cell text, not
  grapheme clusters.

Known limitations of this slice: emoji ZWJ/variation sequences are only as wide
as their per-character widths; resize blanks a wide pair whose continuation
would be truncated rather than reflowing it; ambiguous-width and grapheme
cluster work remain later.

## Deferred

SGR/erase/insert/delete, cell attributes, tabs, origin mode, and
query/reply sequences remain deferred, along with other DEC private modes
(such as 1047/1048, 25, and 2004), application cursor/keypad modes, OSC
titles, scrollback reflow on resize, and saved-cursor state beyond
position. IME, grapheme clusters, and ambiguous-width policy remain later.

## Also merged since this document was first written

The sections above describe the foundation slices in the order they landed. These
capabilities are now on `main` as well and are no longer deferred:

- Erase/insert/delete (ED, EL, ECH, ICH, DCH, IL, DL), SGR and cell attributes,
  and application cursor/keypad modes (DECCKM, DECKPAM/DECKPNM) — PR #29.
- Escape-intermediate sequences (`ESC ( B` and siblings) consumed whole, and
  horizontal tab honored at 8-column stops — PR #29.
- String sequences (DCS/SOS/PM/APC) swallowed to ST or BEL, and CSI private
  markers `<` and `=` poisoning the sequence — PR #43.
- DECSTBM margin clamping and C0 controls executing inside a CSI — PR #45.
- A bounded VT compatibility harness (PR #32) and an adversarial hostile-input
  suite (PR #39).

## Deferred

Origin mode and query/reply sequences remain deferred, along with other DEC
private modes (1047/1048, 25, 2004), OSC titles, scrollback reflow on resize, and
saved-cursor state beyond position.

Mouse support is unimplemented, but note the channel direction before treating any
`CSI M` handling as a parser defect. `feed_bytes` parses PTY **output**, where
parameterless `CSI M` is the valid Delete Line command. X10 mouse reports share
that prefix but travel the other way — generated by the terminal and written to
PTY **input** — so they never reach this parser. Mouse work belongs in an input
encoder in `noren-app`, not in output-side disambiguation; Issue #46 and PR #52
were closed for exactly this reason.

See the [Zellij gap analysis](../compatibility/zellij-gap-analysis.md) for the
ranked list and cost estimates, with the caveat that its gap #3 describes the
output-side `CSI M` handling as a defect, which this correction supersedes.

None of the above is a VT100/xterm compatibility claim, and none is a
vim/tmux/zellij compatibility claim.
