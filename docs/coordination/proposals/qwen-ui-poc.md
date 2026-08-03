# Qwen minimum UI PoC proposal — macOS window/display for PTY output

Status: Round 1 complementary proposal for Issue #6 (Qwen,
`qwencloud/qwen3.8-max-preview`), not a decision. It complements
[glm-core-poc.md](glm-core-poc.md) and re-decides nothing; all numbers are
proposed until measured
([project-principles.md](../../project-principles.md) rule 4). Candidates are
the reversible pins from
[library-comparison.md](../../research/library-comparison.md), gates unpassed.

## 1. Minimum visible behavior

One resizable macOS window with a Metal-backed surface renders the live grid:
the zsh prompt appears after spawn, typed characters echo within one frame of
the PTY drain, and continuous output scrolls visibly. No fullscreen, no
selection model.

## 2. winit lifecycle and the event adapter

`winit` 0.30.13 runs through `ApplicationHandler`/`EventLoop::run_app` on the
macOS main thread; the app struct owns the window and renderer; state changes
only inside handler callbacks. A narrow app-owned adapter converts
`WindowEvent`/`KeyEvent` into app-defined `UiEvent`/`KeyInput` enums at the
boundary, so no winit type reaches the terminal or PTY crates; the renderer
receives an opaque surface handle per the `WindowInputBackend` boundary in
library-comparison.md §3B.

## 3. Minimal render path and cell metrics

One grayscale atlas rasterized by `swash` 0.2.10 from a pinned open-license
monospace face (proposed 14 pt), drawn as one instanced quad pass per
`RedrawRequested` from an immutable snapshot; no shaping, no GPU damage
optimization. Cell width is the ceil of the maximum ASCII advance, height the
ceil of ascent + descent + line gap, in integer physical pixels; metrics
recompute only on scale-factor change.

## 4. Event-to-byte rules for ordinary zsh

Pass-through is the default; no key protocol negotiation (KKP/CSI-u/
modifyOtherKeys) in the PoC.

| Event | Bytes sent |
| --- | --- |
| Printable text | raw UTF-8 |
| Enter | 0x0D |
| Backspace | 0x7F |
| Tab | 0x09 |
| Escape | 0x1B |
| Up/Down/Right/Left | ESC [ A / B / C / D |
| Ctrl+@..Ctrl+_ | 0x00–0x1F (e.g. C→0x03) |

IME, dead keys, fn keys, and Cmd/system shortcut interception are explicitly
deferred policy; Cmd combinations are dropped, not forwarded, until that
policy exists.

## 5. Size to rows/columns

cols = floor(width_px / cell_w), rows = floor(height_px / cell_h), each
clamped to at least 1 (proposed). During live resize only the latest size is
recorded, applied once per event-loop iteration; a resize is issued only when
(cols, rows) changes, after a proposed 50 ms settle past the last `Resized`,
as the single final state + PTY resize + surface reconfigure. Sub-one-cell
sizes are skipped: keep the last grid, never pass zero rows/columns to
`pty.resize()`.

## 6. Error and exit states

Renderer or surface failure is shown as an in-window text overlay where
possible, otherwise in the window title; the loop and PTY drain keep running.
On child exit or EOF the grid freezes and a status line shows the exit state
(code or signal); the window closes on the next user close action or a
proposed 10 s timeout. Window close sends `Close` through the core shutdown
path with a proposed 2 s grace, then exits the loop; a repeated close is a
no-op.

## 7. Acceptance checks and test seams

Proposed bars, each with a named method:

- Key adapter fixture: recorded event table → exact byte oracle, no window.
- Fake `RenderBackend` seam records glyph batches; deterministic numbered-grid
  fixture asserts per-cell coverage, plus a surface readback check on Apple
  Silicon.
- Resize trace: one PTY resize per settled (cols, rows) change, none
  otherwise; resize-to-render at or below 100 ms.
- Zero-size trace: no resize call, grid unchanged.
- Static-grid frame p95 at or below 16 ms; idle wakeups recorded.
- Metrics fixture: pinned face at scale 1.0/2.0 matches recorded pixel values.

## 8. Drop triggers

No gate is claimed passed; triggers mirror library-comparison.md §3A–§5.

| Candidate | Drop trigger |
| --- | --- |
| `winit` 0.30.13 | Nondeterministic AppKit lifecycle/input on the main thread, or winit types leak into terminal/PTY crates → second candidate or recorded gap |
| `wgpu` 30.0.0 | Unrecoverable Metal surface loss during resize storms or scale-factor changes → `glow` 0.18.0 + `glutin` 0.32.3 |
| `swash` 0.2.10 | Metrics/raster quality or malformed-font gate fails → `harfrust` 0.12.0 plus separate raster |
| `unicode-width` 0.2.2 | UAX #11/#17 differential fixtures fail → owned tailoring |
| Cell metrics | Persistent cursor desync versus a pinned reference terminal → revisit metrics policy |
