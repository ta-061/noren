# Minimum architecture for the macOS local-PTY PoC

- Status: Accepted for the first PoC only
- Date: 2026-08-03
- Requirements: [v0.1 requirements](../requirements/v0.1.md)
- Decisions: [ADR 0001](../adr/0001-rust-toolchain-and-msrv.md),
  [ADR 0002](../adr/0002-local-pty-poc-architecture.md)

This architecture is deliberately smaller than the v0.1 product. It proves one
local shell loop while keeping the PTY, terminal state, and platform UI behind
Noren-owned boundaries. It contains no SSH, agent integration, tabs, panes,
themes, persistence, remote daemon, or production compatibility layer.

## Workspace and crates

```text
Cargo.toml                    workspace, resolver 3, shared lint policy
rust-toolchain.toml           Rust 1.88.0, rustfmt, clippy, macOS arm64 target
crates/
  noren-pty/                  process, PTY, bytes, resize, child lifetime
  noren-terminal/             terminal state, cell width, bounded snapshots
  noren-app/                  macOS entry point, winit adapter, renderer, wiring
```

Dependency direction is `noren-app -> {noren-terminal, noren-pty}`.
`noren-terminal` and `noren-pty` do not depend on one another or on window/GPU
types. Test helpers are targets inside the owning package, not a fourth product
crate.

## Ownership and data flow

1. The macOS main thread owns `winit::EventLoop`, the window, renderer, and the
   only mutable terminal state. `noren-app` converts `winit` callbacks into
   app-owned events; no `winit` type crosses a crate boundary.
2. A PTY supervisor thread owns the `portable-pty` master, writer, child handle,
   and lifecycle state. It receives bounded `Input`, `Resize`, and `Close`
   commands. It polls child status between commands and is the only code allowed
   to kill or reap the child.
3. A PTY reader thread owns a cloned blocking reader. It sends bounded
   `Output(Vec<u8>)`, `Eof`, or typed error events to the main loop. It observes
   EOF when the last slave descriptor closes; receiver disconnect terminates a
   blocked channel send, but reaping alone is not assumed to cancel `read`.
   Each read/output chunk is at most 16 KiB and the output channel holds at most
   64 chunks (1 MiB queued payload).
4. The main loop drains a bounded amount of output per callback, feeds it to a
   `TerminalEngine`, applies bounded side effects, and requests redraw. Channel
   capacity and per-turn byte budget are constants covered by load tests: at
   most 64 KiB is parsed per turn; the ordered input/resize/reply command channel
   holds at most 256 messages.
5. On redraw, `RenderBackend` consumes an immutable cell snapshot. Parser state
   never receives a surface or GPU handle, and renderer failure leaves PTY
   teardown possible.
6. At `about_to_wait`, the app coalesces pending physical-size events, computes
   the latest non-zero `(rows, columns)`, updates terminal state/surface, and
   sends one PTY resize when the grid changed. A zero-sized window retains the
   last valid grid and never sends zero dimensions.
7. Shutdown is an idempotent state machine: stop accepting input, send `Close`,
   close the writer, terminate the child only if still running, reap it, and drop
   the supervisor's master handle so the PTY hangup reaches remaining slave
   holders. The app waits for reader EOF and joins both workers until the 2 s
   deadline. If a descendant still retains the slave, it emits
   `ReaderJoinTimeout`, drops the reader `JoinHandle` (detaching it for process
   exit), and exits rather than block forever. Normal close must join; the
   detach path is a visible failed acceptance case, not silent success.

## Narrow contracts

- `PtyBackend`: structured executable/argv/cwd/environment policy; spawn, byte
  I/O, resize, status, and teardown. The PoC executable is fixed to `/bin/zsh`;
  no caller-supplied command string or `-c` path exists. Cwd is the inherited
  `HOME` only after it is validated as an absolute existing directory; missing
  or invalid `HOME` is a typed spawn failure with no ambient fallback. The child
  inherits the launch environment because this terminal is not a sandbox, then
  Noren sets `TERM=xterm-256color` and `TERM_PROGRAM=Noren-PoC` and removes
  `COLUMNS`/`LINES` so PTY dimensions are authoritative. No other variable is
  scrubbed or logged in this PoC, and no runtime configuration may add one.
- `TerminalEngine`: bytes and dimensions in; bounded snapshot, damage, replies,
  and non-authoritative title metadata out. Replies return through a distinct
  PTY command only as opaque bytes: at most 4 KiB per main-loop turn and 64 KiB
  per second. Excess replies produce a typed error; `noren-app` never interprets
  them. Clipboard, filesystem, IPC, and process side effects are disabled.
- `CellWidth`: grapheme plus explicit Unicode/ambiguous-width policy in; a
  bounded cell count out.
- `WindowInputAdapter`: platform callbacks in; timestamped app-owned lifecycle,
  resize, redraw, and key events out.
- `KeyEncoder`: pressed app-owned key events in; zero or more terminal bytes
  out. Printable UTF-8, Enter `0x0D`, Backspace `0x7F`, Tab `0x09`, Escape
  `0x1B`, arrows `ESC [ A/B/C/D`, and Ctrl `0x00..0x1F` are the PoC contract.
  Key releases and unsupported Cmd/Option/IME/dead-key combinations emit zero
  bytes and a payload-free typed drop event; they never degrade to the base key.
- `RenderBackend`: immutable grid/glyph batches plus an opaque surface handle
  in; recoverable render status out.

## Reversible PoC candidates

Exact versions come from the merged Discovery comparison. Selection here means
"measure in the first PoC," not permanent adoption.

| Boundary | First candidate | Replacement/drop trigger |
| --- | --- | --- |
| PTY | `portable-pty` 0.9.0 | Descriptor leak, unresolvable lifecycle race, or unmaintainable fork: compare `nix` 0.31.3 behind the same contract. |
| Terminal state | `avt` 0.18.0 | Missing bounded snapshot/damage/reflow/reply behavior or corpus failure: compare state-only use of `alacritty_terminal` 0.26.0. |
| Window/events | `winit` 0.30.13 | Nondeterministic AppKit lifecycle/input or boundary leakage: stop and record the supported-candidate gap before choosing another toolkit. |
| Renderer | `wgpu` 30.0.0 | Unrecoverable Metal/surface behavior or measured budget failure: compare `glow` 0.18.0 + `glutin` 0.32.3 behind `RenderBackend`. |
| Glyphs | `swash` 0.2.10 | Malformed-font, cache-bound, metrics, or raster-quality failure: compare `harfrust` 0.12.0 plus a separate rasterizer. |
| Cell width | `unicode-width` 0.2.2 | Unicode/terminal differential corpus failure: replace behind `CellWidth` with documented tailoring. |
| Concurrency | Rust threads + bounded `std::sync::mpsc::sync_channel` | If measurement proves the event bridge cannot meet correctness/latency, record a new ADR before adding an async runtime or channel dependency. |

No SSH or async runtime dependency belongs in this PoC. Direct-dependency
features stay minimal and the implementation PR records the resolved lockfile
and license metadata.

## Error model and observability

Component errors are typed events with component, operation, and safe status;
terminal contents, environment values, and input bytes are not logged. Natural
exit freezes the last grid and shows a non-sensitive exit status until the user
closes the window. Spawn failure is visible without starting the event loop's
PTY state. Panics are bugs and must not be used for expected EOF, resize,
surface loss, reader-join timeout, or child-exit paths.

## Deferred design decisions

IME/dead keys, Cmd/Option shortcut policy, key-protocol negotiation, complete
VT/xterm compatibility, font fallback/shaping, accessibility, Linux backends,
scrollback/selection, persistence, SSH, and remote/session architecture stay
closed to implementation. Their existing risk-register gates remain active.
