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
  `MAX_SCROLLBACK_LINES * cols * sizeof(Cell)`. At ~40 bytes/cell that is ~32 MiB
  for an 80-column terminal and ~100 MiB for 256 columns; the line count is the
  hard bound so hostile unbounded output cannot grow history past it. The oldest
  retained line is evicted when the cap is reached.
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

Unicode/CJK character width remains the largest outstanding gap: the parser still
prints only `0x20..=0x7e`, so wide characters and combining marks are not yet
modeled even though `Cell` carries a `width` field.

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
