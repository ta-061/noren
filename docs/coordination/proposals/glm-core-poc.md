# GLM core PoC proposal — first macOS local-PTY loop

Status: Round 1 proposal for Issue #6 (GLM, `zai-coding-plan/glm-5.2`), not a
decision. Versions are reversible candidates from
[library-comparison.md](../../research/library-comparison.md); that report's
gates are not claimed passed. Non-goals: SSH, AI-agent state, tabs, panes,
themes, remote daemon, and production-ready IME, accessibility, or
cross-platform parity.

## 1. Seven functional steps

1. **Boot & window** — `noren-app` creates the `winit` 0.30.13 window/event
   loop, renderer (wgpu Metal surface), and terminal state on the macOS main
   thread.
2. **Spawn** — `noren-pty` spawns the user's shell (zsh) as direct argv (no
   `/bin -c`, no concatenation) via `portable-pty` 0.9.0 and starts one PTY
   worker thread owning the child handle.
3. **Read loop** — The worker blocks on the PTY master, chunks bytes, and sends
   each chunk over a bounded stdout channel to main; a full channel
   backpressures the child rather than growing memory.
4. **State update** — Main drains stdout, feeds bytes to the `avt` 0.18.0 engine
   behind the `TerminalEngine` seam, and records grid damage plus replies.
5. **Render** — On `RedrawRequested` the renderer draws an immutable grid
   snapshot plus damage through the `RenderBackend` seam via wgpu.
6. **Input** — Keyboard events are encoded and sent over a bounded stdin channel
   to the worker, which writes them to the PTY master; pass-through is default,
   no interception in the PoC.
7. **Resize** — Main computes new rows/cols, calls `terminal.resize()`, sends a
   resize command to the worker (`pty.resize()`), and lets the renderer
   reconfigure its surface.

## 2. Minimum nonfunctional targets

All are proposed budgets, not measured facts, with a method and pass bar
([project-principles.md](../../project-principles.md) rule 4).

- Input-to-render p95 at or below 16 ms (one 60 Hz frame) for a static grid.
- Sustain at least 10 MB/s child output with no UI freeze and bounded queue.
- No memory growth across 60 s idle after a 100 MB burst; scrollback bounded.
- Resize-to-render at or below 100 ms; child reaped within 2 s, no orphan or
  zombie.
- Cold start to first byte rendered at or below 1 s on a recorded Apple
  Silicon Mac.

## 3. Ownership and data-flow architecture

- **winit main thread** owns window, event loop, renderer, and terminal state
  (single-writer, no state locks).
- **PTY worker thread** owns the child handle and master reader; it alone
  touches PTY FFI.
- **Channels** are bounded sync (`std::sync::mpsc` or crossbeam sync): stdout
  worker→main, stdin and resize commands main→worker; a full channel blocks the
  producer for backpressure.
- **Terminal state** lives on main, updated only from stdout events, exposing
  immutable snapshots plus damage.
- **Renderer** runs on main under `RedrawRequested`; it consumes snapshots and
  never mutates state or touches the PTY.
- **Resize** serializes through main — state resize, PTY resize command, then
  surface reconfigure — so child and grid stay consistent.
- **Idempotent shutdown/reap** — main sends `Close`; the worker stops reading,
  closes the writer, waits the child with a bounded timeout, reaps via
  `waitpid` (no zombie), reports exit; main then drops channels and the
  renderer. Steps are reentrant (second `Close` is a no-op); PTY or renderer
  failure surfaces an error but cannot block or crash the loop.

## 4. Initial crates

- `noren-pty`: `PtyBackend` trait plus the `portable-pty` adapter; spawn, read,
  write, resize, reap.
- `noren-terminal`: `TerminalEngine` and `CellWidth` seams; wraps `avt` state
  and `unicode-width`.
- `noren-app`: `winit` main loop, `RenderBackend` plus the wgpu renderer,
  channels, and wiring.

One repo per [D-0002](../decisions.md); no circular deps; direction
`app → terminal, pty`.

## 5. Reversible candidate stack

No SSH and no async runtime: std threads plus bounded sync channels.

| Role | Candidate | Drop/replace trigger |
| --- | --- | --- |
| Window/event | `winit` 0.30.13 | Cannot deliver deterministic AppKit lifecycle/IME → second candidate or recorded gap |
| PTY | `portable-pty` 0.9.0 | Descriptor leak, unmanageable race, or sustained fork → `nix` 0.31.3 |
| Terminal state | `avt` 0.18.0 | Cannot expose bounded snapshot, damage, reflow → `alacritty_terminal` 0.26.0 (state modules only) |
| Renderer | `wgpu` 30.0.0 | Unrecoverable Metal surface loss or budget miss → `glow` 0.18.0 + `glutin` 0.32.3 |
| Cell width | `unicode-width` 0.2.2 | UAX #11/#17 differential fixtures fail → owned tailoring |
| Glyph raster | `swash` 0.2.10 | Malformed-font or memory gate fails → `harfrust` 0.12.0 plus separate raster |

All are disposable `experiments/` spikes, not production choices
([design-process.md](../design-process.md)).

## 6. Toolchain / MSRV

Proposed pin: Rust **1.88.0** stable, MSRV 1.88.0, installed target
**aarch64-apple-darwin** (Apple Silicon PoC). Rust is currently absent
([risk-register.md](../../roadmap/risk-register.md) R-PORT-01), so installation,
`rust-toolchain.toml`, the lockfile, and the first CI compile belong to the
first implementation Issue — not this proposal. 1.88.0 matches the highest
observed candidate MSRV (`ssh2-config`, `self_update`); a proposal, not an
installed fact.

## 7. Core threat boundaries

- PTY child output is **data, never authority**; OSC payloads bounded; no
  OSC-driven clipboard, IPC, or execution in the PoC.
- No shell concatenation; structured argv only.
- Renderer, PTY, and child failure are isolated; one failure cannot block or
  crash the loop.
- No secrets or private data in logs or dumps; PoC logging is redacted.
- Font and width tables are untrusted structured input, size-bounded.

## 8. Test seams

- `TerminalEngine`: feed an ECMA-48/xterm corpus plus fuzz seeds; assert grid,
  damage, replies, no panic, bounded memory.
- `PtyBackend`: a fake PTY drives resize storms, EOF, exit/signal races, and
  descriptor enumeration without a real child.
- `RenderBackend`: a headless deterministic numbered grid plus damage trace;
  assert draw calls and frame timing.
- `CellWidth`: UAX #11/#29, emoji ZWJ, combining marks, ambiguous-width
  fixtures.
- Channels are injectable to replay synthetic stdout timing in tests.

## 9. ADRs needed to start

- **ADR-0001 Toolchain and MSRV** — pin Rust 1.88.0 and target
  aarch64-apple-darwin; unblocks the R-PORT-01 prerequisite gate.
- **ADR-0002 Workspace crates** — `noren-pty`, `noren-terminal`, `noren-app`;
  single repository.
- **ADR-0003 Reversible PoC seams** — the trait boundaries and candidate pins
  above, all reversible.

## Open questions deferred

PTY abstraction ownership, the state/engine boundary, and the
window/renderer/font/IME stack stay open
([open-questions.md](../open-questions.md)); the PoC feeds them evidence, not
closures.
