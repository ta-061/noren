# Handoff — Mouse input encoder lane (GLM, `glm-mouse`)

> **Note on the handoff template:** `docs/coordination/handoffs/TEMPLATE.md`
> was **not** present on `origin/main` when this lane ran. This file follows
> the lane prompt's required fields and the shape of [`glm-a.md`](./glm-a.md).

## Identity

- **Lane:** `glm-mouse` (mouse input encoding), engine GLM 5.2 via opencode.
- **Branch:** `agent/mouse-input-encoder`, branched from `origin/main` @
  `91a0536` (424 workspace tests passing at branch point, 1 pre-existing
  ignored).
- **Base SHA:** `91a053698d6bda0de657531ef25ae1882e592841`.
- **File lease (exactly two source files + this handoff):**
  `crates/noren-app/src/mouse.rs`, `crates/noren-app/tests/mouse_encoding.rs`.
  **No edits to `lib.rs`, `main.rs`, `actions.rs`, `passthrough.rs`,
  `Cargo.toml`, or `Cargo.lock`** — export wiring is a separate serial commit
  (verified: their combined diff vs main is empty).

## The premise (read this before treating any `CSI M` as a parser bug)

Issue #46 was closed as **not-a-bug**. `TerminalState::feed_bytes` parses PTY
**output**, where parameterless `CSI M` is the valid Delete-Line command. xterm
mouse reports share that prefix but travel the **opposite direction**: the
terminal *generates* them and writes them to PTY **input**. They never reach
the output parser. So mouse work is an **input encoder** here, not output-side
disambiguation. This module never touches `feed_bytes`. See the Deferred
section of
[terminal-core-foundation.md](../../architecture/terminal-core-foundation.md).

## What was implemented

A pure, stateless `MouseEncoder` that turns pointer events into xterm mouse
report bytes, gated on the modes the application enabled.

- **Mode tracking** (`MouseModes`): the application's DECSET/DECRST state.
  - *Tracking* (decide **whether** to report): 1000 normal, 1002 button-event,
    1003 any-event.
  - *Encoding* (decide **how** to format): 1006 SGR, 1015 urxvt, 1005 UTF-8.
  - With **no tracking mode enabled, nothing is emitted at all.**
  - `set(mode: u16, on: bool)` drives state from the raw mode number for the
    future DECSET/DECRST wiring; named `with_*` builders serve tests.

- **Encoding precedence** (fixed, so the caller cannot pick a broken combo):
  1. **SGR (1006)** when enabled — `CSI < Cb ; Cx ; Cy M` for press/wheel/
     motion, lowercase `m` for release. Preferred because it alone
     distinguishes the released button (it keeps the button code; legacy forms
     collapse every release to `Cb = 3`).
  2. else **urxvt (1015)** — `CSI Cb ; Cx ; Cy M`, decimal, no angle bracket.
  3. else the **X10 byte form** — `CSI M` plus `(Cb+32)(Cx+32)(Cy+32)`.

- **Event reporting**:
  - Press, release, wheel report under any tracking mode (1000/1002/1003).
  - Motion reports only under button-event (1002) or any-event (1003), never
    under plain 1000. Under 1002 a button must be held (a hover is dropped);
    under 1003 all motion reports. **This is the 1002-vs-1003 distinction.**
  - `Cb` modifier bits: Shift=4, Alt=8, Ctrl=16. Motion=32. Wheel base=64
    (up=64, down=65). Super/Command is not modeled (the window layer drops it,
    matching the key encoder).

- **Coordinate rules**: coordinates are **1-based and clamped to the grid**.
  The encoder takes 0-based cell indices, clamps to `[0, cols-1]`/`[0, rows-1]`,
  emits `col+1`/`row+1`. An out-of-range coordinate is never emitted.

- **1005 (UTF-8) is tracked but its byte extension is not implemented** in this
  slice; with neither 1006 nor 1015 active it falls through to the X10 byte
  form. This is recorded so DECSET/DECRST state stays correct.

## The X10 223-column rule (chosen, documented)

The X10 byte form offsets each coordinate by 32, so a 1-based coordinate above
223 would overflow a byte. **Rule chosen: when the X10 byte form is active and
either 1-based coordinate exceeds 223, the report is dropped (`None`) rather
than emitting a saturated, wrong position.** SGR and urxvt are decimal and have
no such limit, so the preferred SGR encoding never hits this rule on a wide
grid. `X10_MAX_COORD = 223` is the named bound. `Cb` always fits a byte (the
largest value is wheel-down plus every modifier, `65 + 28 == 93`).

## How the unwired module is tested

The lease forbids editing `lib.rs`, so the module is compiled **standalone** in
the integration test, exactly the pattern used by the `glm-a` session lane:

```rust
#[path = "../src/mouse.rs"]
mod mouse;
```

`cargo test --workspace` and `cargo clippy --workspace --all-targets` both see
`mouse.rs` through the test target. The module is deliberately self-contained
(no `crate::` items) so the `#[path]` include compiles. **When the serial
wiring commit adds `pub mod mouse;` to `lib.rs`, that `#[path]` line must
become `use noren_app::mouse;`** or the module compiles twice as two unrelated
types.

## Required-test coverage (all present, all byte-exact)

- SGR press/release/drag/wheel against the xterm forms, plus modifier bits and
  middle/right button codes.
- X10 fallback when only 1000 is on (press/release/wheel/shift/middle/drag).
- **Nothing emitted** when no tracking mode is enabled (every event kind),
  including SGR+urxvt enabled but no tracking.
- 1002 vs 1003 differ on drag-without-button: 1002 drops the hover, 1003
  reports `Cb = 35` (3 + motion 32).
- Coordinates clamp at the right/bottom edges and floor at 1x1; a 1x1 grid
  clamps everything to cell `(1,1)`; zero-dimension grids are rejected.
- Coordinates beyond X10's 223 limit: column-overflow and row-overflow both
  drop; the cx=223 boundary reports (byte 255) and cx=224 drops; SGR reports
  the same overflow coordinate with no limit.
- Encoding precedence (SGR wins over urxvt when both on) and full urxvt
  (1015) byte forms.
- Mode-number `set()` wiring, off-toggles, and unknown-mode-no-op.

## Commands actually run (gate), with real results

On `agent/mouse-input-encoder`, macOS arm64, rustc 1.88.0.

```
$ cargo fmt --all && cargo fmt --all --check          → exit 0 (clean)
$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile; exit 0, 0 warnings
$ cargo test --workspace                               → exit 0
    PASSED=424 FAILED=0 IGNORED=1
```

`mouse_encoding` contributes **36 passing tests**. Baseline at branch point
was 388 workspace tests; the +36 brings the total to 424. The 1 ignored test
is pre-existing and unrelated.

### Two test-correction iterations (implementation unchanged)

The first `cargo test` run surfaced three **test-side** errors, none in the
encoder:

1. Two `sgr_drag_*` tests used a normal-only mode for a motion event; motion
   requires button-event (1002) or any-event (1003). Fixed by giving those
   tests a button-event mode.
2. The X10 boundary byte arithmetic was off by one: `223 + 32 == 255` (0xff),
   not 254. The encoder produced the correct 255; the expected literal was
   corrected to `\xff`.

After those corrections the run was clean on the first re-pass.

## Standing design decisions (so a reviewer can challenge them)

1. **X10 overflow = drop**, not saturate. xterm saturates; this lane drops to
   avoid silently misreporting the position. SGR is preferred and unaffected.
2. **1005 is tracked, not byte-extended.** Its UTF-8 coordinate extension is
   notoriously broken (the reason xterm added 1006/1015) and is out of scope;
   1005-only falls through to the X10 byte form.
3. **SGR release keeps the button code**; legacy X10 and urxvt collapse every
   release to `Cb = 3` (they cannot say which button was released).
4. **The encoder is stateless.** Callers own `MouseModes` (from DECSET/DECRST
   observation) and `MouseGrid` (from resize) and pass them per event.
5. **Coordinates are 0-based on input** (cell indices from the window layer);
   pixel→cell conversion is the window layer's job, not the encoder's.
6. **Super/Command is not modeled** on pointer modifiers, matching the key
   encoder's policy.
7. No motion history/debouncing; every event is encoded independently.

## Historical limits and current disposition

- **The module is not compiled as part of `noren-app`'s library yet**
  (`mod mouse;` absent from `lib.rs` by lease). It compiles only via the
  `#[path]` test. `noren_app::mouse::*` cannot resolve until the serial wiring
  commit.
- Pixel-to-cell conversion, HiDPI, pane-frame offsets, and the `Shift bypass`
  selection gesture are all window/selection-layer concerns and are out of
  scope for this input-encoding lane.

## Authorship / conflict of interest

I (GLM `glm-mouse`) authored all the code (`mouse.rs`, `mouse_encoding.rs`)
and this handoff. Per the [development model](../development-model.md), an
independent reviewer must cover the current head before merge.

## Resume instructions

1. `git checkout agent/mouse-input-encoder`; confirm the head.
2. Re-run the gate: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`,
   `cargo test --workspace` (expect 424 passed / 1 ignored).
3. To wire into the crate (serial integration commit, **not** this branch): add
   `pub mod mouse;` to `crates/noren-app/src/lib.rs` (and re-export the
   surface the PTY writer needs), then change the first non-comment line of
   `tests/mouse_encoding.rs` from `#[path = "../src/mouse.rs"] mod mouse;` to
   `use noren_app::mouse;`. Drive `MouseModes` from the terminal-state
   DECSET/DECRST handler via `MouseModes::set(mode, on)`.
