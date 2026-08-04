# Terminal core foundation

- Status: PR [#19](https://github.com/ta-061/noren/pull/19) merged as
  `c695920`; the scroll-region slice is in progress as Draft PR
  [#21](https://github.com/ta-061/noren/pull/21); the alternate-screen slice
  (core commit `84da736`) is stacked on top as Draft PR
  [#23](https://github.com/ta-061/noren/pull/23) for Issue
  [#22](https://github.com/ta-061/noren/issues/22)
- Supersedes nothing: the accepted [local-PTY PoC
  architecture](minimal-local-pty-poc.md) still applies

This foundation moves PTY output bytes through a renderer-independent terminal
state core in `crates/noren-terminal/`. It is a foundation slice, not a
VT100/xterm compatibility claim and not vim/tmux/zellij compatibility.

## Data flow

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

## Deferred

SGR/erase/insert/delete, cell attributes, tabs, origin mode, and
query/reply sequences remain deferred, along with other DEC private modes
(such as 1047/1048, 25, and 2004), application cursor/keypad modes, OSC
titles, alternate-screen scrollback, and saved-cursor state beyond
position. Unicode and IME remain later.
