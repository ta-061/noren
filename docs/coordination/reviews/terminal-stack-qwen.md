# Terminal stack review — Qwen (application/UI layer)

Reviewed at: `0daa7d6` (HEAD of this worktree, `agent/terminal-sgr-attributes`)
Reviewed code: `fd1ea69` / `agent/application-modes` (cumulative stack tip; read from the
sibling `application-modes` worktree because this worktree must not switch branches).
Scope: `crates/noren-app` (`input.rs`, `lib.rs`, `main.rs`, `renderer.rs`) and the
mode-query coupling into `noren-terminal`. The terminal state machine itself was reviewed
only where the app consumes it.

## Verdict
ACCEPT_WITH_FOLLOWUP

The stack's new functionality — DECCKM (`ESC [ ?1 h/l`) and DECKPAM/DECKPNM (`ESC =` /
`ESC >`) wired into the key encoder — is byte-correct against xterm/VT semantics and is
covered by tests. Keystrokes are neither duplicated (releases dropped, repeat emitted once,
exclusive physical-keypad branch) nor mis-routed. No BLOCKERs found. Two MAJOR defects,
both pre-existing before this stack but in this lane and reachable in normal use, should
land as follow-ups before the "first-class input contract" and Zellij-parity claims hold.

## Findings

### MAJOR Renderer silently truncates the grid to 160x60 while the PTY/terminal grid grows unbounded
- Location: `crates/noren-app/src/renderer.rs:17-18` (`MAX_RENDER_ROWS = 60`,
  `MAX_RENDER_COLS = 160`) and `renderer.rs:284-289` (clamp), while
  `crates/noren-app/src/lib.rs:139-158` (`GridGeometry::update`) computes the PTY/terminal
  grid from the same physical pixel size with only a `u16::MAX` cap.
- Reproduction: on any Retina display (scale factor 2), drag the window past roughly
  800x600 logical points. Example: `WindowEvent::Resized` of 2000x1600 physical pixels →
  PTY and terminal become 200 cols x 80 rows (`main.rs:171-201`), but the renderer clamps
  the drawn grid to 160x60.
- Expected vs actual: expected — the rendered grid and the grid handed to the PTY are the
  same geometry. Actual — columns >= 160 and rows >= 60 are never drawn. Full-screen TUIs
  (vim, htop, zellij) position UI beyond the visible area and become unusable; the shell
  wraps lines at a column the user cannot see. No error or status is surfaced.
- Suggested fix: cap the grid at one place, e.g. compute the PTY grid as
  `min(cell-grid, (MAX_RENDER_COLS, MAX_RENDER_ROWS))` in `GridGeometry` (or expose the
  render cap and clamp `GridGeometry::update` to it), so PTY, terminal state, and renderer
  always agree.

### MAJOR Silent keystroke drops vs real-terminal bytes (Delete, Alt+char, Ctrl+named keys, Shift combos)
- Location: `crates/noren-app/src/lib.rs:345-368` (`KeyEncoder::encode_with`) and
  `crates/noren-app/src/main.rs:413-415` (`translate_logical_key` fallback arm).
- Reproduction / expected vs actual (xterm as reference):
  - Delete / Home / End / PageUp / PageDown / Insert / F1-F12 →
    `KeyDropReason::UnsupportedKey` (silently nothing). xterm sends `CSI 3 ~`, `CSI H`,
    `CSI F`, `CSI 5 ~`, `CSI 6 ~`, `CSI 2 ~`, `CSI 11 ~`.. — these are everyday keys in
    vim/less/zellij.
  - Alt+<letter> → `UnsupportedModifier` (`lib.rs:345-347`). xterm sends an `ESC`-prefixed
    sequence (`ESC f` for Alt+f, used by zsh word motion), so the keystroke is lost.
  - Ctrl+Enter / Ctrl+Backspace / Ctrl+Tab / Ctrl+Escape → `UnsupportedControl`
    (`lib.rs:348-355` — the Ctrl arm only accepts `Key::Character`). xterm sends the base
    bytes (`\r`, `0x7f`, `\t`, `0x1b`), so habitual Ctrl+Backspace word-delete emits nothing.
  - Shift is tracked (`main.rs:111-126`) but never encoded: Shift+Arrow emits the bare
    arrow sequence (xterm: `CSI 1;2 A` etc., even under DECCKM) and Shift+Tab emits `0x09`
    (xterm: `CSI Z`), so the two are indistinguishable.
- Context: these are declared baseline bounds (doc comments at `lib.rs:163-165` and
  `lib.rs:250-252`) and pre-date this stack (encoder shipped in the merged PoC), so they do
  not block it. They are nevertheless real, silent drops against the project's
  input-preservation contract and must not be considered closed.
- Suggested fix: staged follow-ups — (1) encode Delete/Home/End/PgUp/PgDn/F-keys; (2) emit
  base bytes for Ctrl+Enter/Backspace/Tab/Escape; (3) ESC-prefix Alt+char; (4) add
  `CSI 1;<mod> <final>` modifier encoding for arrows/Tab. Open tracked follow-up issues
  rather than silently widening this merge's scope.

### MINOR Resize propagation to terminal and PTY is not atomic; divergence can persist
- Location: `crates/noren-app/src/main.rs:184-201` (`apply_pending_resize`).
- Reproduction: resize the window so the grid exceeds `MAX_SCREEN_CELLS`
  (1024*1024, `crates/noren-terminal/src/state.rs:8,927-937`); `GridGeometry` permits up to
  65535x65535 (`lib.rs:143-150`), so e.g. a 20500x10300-physical window yields a
  2050x515 grid. `terminal.resize` fails with `ScreenTooLarge`, but the code falls through
  and still sends the resize to the PTY (`main.rs:194-199`); `geometry.current` has already
  advanced, so resizing back to the same physical size coalesces to `None`
  (`lib.rs:152-153`) and the shell (new grid) / terminal state (old grid) stay diverged
  until the user picks a different size. The inverse divergence occurs if
  `session.resize` fails (`main.rs:195-198`) after the terminal already resized.
- Expected vs actual: expected — terminal state and PTY always share one geometry, or the
  resize is retried/rolled back. Actual — independent best-effort updates with a surfaced
  status string only.
- Suggested fix: resize the PTY only after `terminal.resize` succeeds, and on either
  failure keep `pending_grid` set (or revert `geometry.current`) so the next
  `about_to_wait` retries.

### MINOR GPU adapter/device request parks the AppKit event loop with no deadline
- Location: `crates/noren-app/src/renderer.rs:75-86` (`block_on` of `request_adapter` /
  `request_device`), reached from `main.rs:98` inside `Resumed`; `block_on` parks the
  calling thread (`renderer.rs:251-273`).
- Reproduction: any GPU/driver stall during window creation (adapter request never
  completes) — the UI thread blocks indefinitely inside the `Resumed` handler; the window
  never appears and AppKit marks the app unresponsive.
- Expected vs actual: expected — the project already models a deadline for PTY shutdown
  (`SHUTDOWN_DEADLINE`, `lib.rs:41`); renderer bring-up has none. Actual — unbounded park on
  the main thread. Usually milliseconds on Metal, hence MINOR.
- Suggested fix: move `Renderer::new` off the main thread with a bounded join, or give the
  `block_on` a timeout that surfaces `RendererError`.

## Areas checked and found sound
- DECCKM / DECKPAM / DECKPNM encoding tables: `input.rs:165-202` match xterm exactly
  (`ESC [ A`..`ESC [ D` vs `ESC O A`..`ESC O D`; keypad digits `ESC O p`..`ESC O y`,
  multiply `j`, add `k`, minus `m`, Enter `M`, decimal `n`, divide `o`), and the
  terminal-side parsing (`ESC =` / `ESC >`, `CSI ?1 h/l` only under
  the `?` marker) plus idempotent, independent mode setters are correct.
- Mode wiring reads live state on the main thread per keypress (`main.rs:152-169`), so
  encoding tracks `ESC [ ?1 h` from shell output within one loop turn; no stale reads.
  Modes survive alternate-screen switching and resize, and snapshots capture them
  immutably (`tests/application_modes.rs:52-92`).
- Byte contract for Enter (`\r`), Backspace (`0x7f`), Tab (`0x09`), Escape (`0x1b`),
  printable UTF-8, and Ctrl control bytes incl. Ctrl+Space/@ → NUL
  (`lib.rs:363-368,401-412`); Ctrl+digit correctly produces nothing, like xterm.
- No duplication: releases dropped once (`lib.rs:341-343`), autorepeat emitted once per
  event (`main.rs:357-363`), and the physical-keypad branch is exclusive with the logical
  branch (`main.rs:130-135`), so numpad keys are never double-encoded.
- Renderer consumes the immutable `TerminalSnapshot` by borrow only
  (`main.rs:266-279`, `renderer.rs:172-176`); zero physical size and extreme sizes are
  guarded without panic (`renderer.rs:163-170,281-289,314`; `GridGeometry` clamps to
  >=1 and `u16::MAX`, `lib.rs:143-150`).
- Resize coalescing keeps only the last changed non-zero grid; zero-dimension windows keep
  the previous grid and never send 0 to the PTY (`lib.rs:139-158`, tests at `lib.rs:492-510`).
- Lifecycle: PTY is taken and shut down on all exit paths (`main.rs:254-263,293-300,
  342-346`) with `Drop` as fallback, shutdown is deadline-bounded and idempotent
  (`crates/noren-pty/src/lib.rs:376-406`), and `send_input`/`try_recv` are non-blocking
  with the per-turn parse budget (`main.rs:203-250`), so the event loop cannot deadlock or
  leak the child on normal close.
