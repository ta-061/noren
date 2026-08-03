# Terminal core foundation

- Status: Draft, in progress on PR [#19](https://github.com/ta-061/noren/pull/19)
  (branch `agent/terminal-core-foundation`); not merged
- Supersedes nothing: the accepted [local-PTY PoC
  architecture](minimal-local-pty-poc.md) still applies

This foundation moves PTY output bytes through a renderer-independent terminal
state core in `crates/noren-terminal/`. It is a foundation slice, not a
VT100/xterm compatibility claim.

## Data flow

```text
PTY bytes -> TerminalState::feed_bytes -> parser -> bounded screen/cursor state
TerminalState::snapshot -> TerminalSnapshot -> renderer (lines only)
```

- `TerminalState` owns the screen, cursor, dimensions, and parser state.
- The visible grid is a fixed-size `ScreenBuffer` of `Cell` values, bounded by
  `MAX_SCREEN_CELLS` (1,048,576 cells).
- `resize` rebuilds the grid and preserves the overlapping top-left region.
- The renderer consumes the immutable `TerminalSnapshot` and never depends on
  PTY or parser types.

## Supported behavior

- Printable ASCII bytes (`0x20..=0x7e`), line feed, carriage return, and
  backspace.
- A minimal CSI cursor-movement subset: cursor up, down, forward, back,
  absolute position, and absolute column.
- Unsupported and OSC bytes are ignored without corrupting state.

## Deferred order

Scroll regions, alternate screen, SGR/erase plus cell attributes, and mode
state, in that order. Unicode and IME remain later.
