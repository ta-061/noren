# Candidate library comparison

Status: Milestone 0 evidence and PoC backlog, not an adoption decision

Retrieved: 2026-08-03 (Asia/Tokyo)

Scope: GitHub Issue [#3](https://github.com/ta-061/noren/issues/3)

## Method and limits

This comparison uses versioned upstream documentation and source, official
package metadata, release records, standards, and upstream security policies.
Each crates.io metadata link records the observed version, publication time,
SPDX expression, declared repository, and dependency metadata. A publication
date or default-branch commit is maintenance evidence, not a maintenance
judgment. Download counts, stars, and model recollection are not evidence here.

The labels such as `PtyBackend` and `CellWidth` below are replaceability seams
for PoCs. They are not approved Noren modules or architecture. “Candidate” means
worth measuring, not selected or recommended for production.

The unsafe and dependency notes are deliberately conservative. This pass did
not produce a locked dependency graph, recursive unsafe inventory, provenance
attestation, or complete advisory history for any Rust candidate. Those are
explicit PoC gates. No row treats an absent search result as proof that a
project has no vulnerabilities.

## Common validation rules

Every PoC below must pin the crate/release and source commit, preserve a lockfile,
record Rust and native toolchain versions, run license and advisory scans, list
enabled features, collect `cargo tree`, inventory direct and transitive `unsafe`,
and test both macOS and Linux unless the row says otherwise. Inputs derived from
terminal output, SSH peers, config, IPC, fonts, plugins, or update services must
be bounded and sanitized. Logs must not contain credentials, private keys,
passphrases, cookies, terminal clipboard contents, or protected input.

This research does not propose a repository-wide fork. The explicit
per-candidate ledger below records whether the current PoC needs a fork or
patch and gives a screening-level replacement-cost estimate. A later PoC must
replace those estimates with measured patch size, generated/native code,
release automation, and upstream contribution latency; popularity is not a
forkability or maintenance proxy. The replaceable seams below keep a
dependency's types out of Noren's public contract.

## 1. VT byte parsing

### `vte` 0.15.0

- **Function and evidence:** A state-machine escape parser that drives a
  consumer-supplied `Perform` implementation; it does not supply Noren's full
  terminal grid or policy. See the versioned
  [API](https://docs.rs/vte/0.15.0/vte/) and
  [source](https://github.com/alacritty/vte/tree/v0.15.0).
- **Release, maintenance, and license:** Version 0.15.0 was published
  2025-02-02 with SPDX `Apache-2.0 OR MIT` in official
  [package metadata](https://crates.io/api/v1/crates/vte/0.15.0). Default-branch
  commit [`abeae765dd54`](https://github.com/alacritty/vte/commit/abeae765dd54)
  is dated 2026-02-28. The 0.x API has no separate stability promise in the
  reviewed sources.
- **Targets:** Parser logic is OS-independent; docs.rs records `no_std` support.
  macOS/Linux viability still depends on the surrounding state and I/O layers.
- **Unsafe, dependencies, and security:** A recursive inventory is pending.
  Security-critical concerns are unbounded OSC/DCS strings, malformed or
  truncated sequences, terminal replies, and policy implemented by `Perform`,
  not parser recognition alone.
- **Replaceable boundary:** `VtParser` emits bounded typed actions into a
  separately tested `TerminalState` seam.
- **Validating PoC:** Feed ECMA-48/xterm fixtures, malformed UTF-8, byte-by-byte
  splits, oversized OSC/DCS, and fuzz-generated streams. Assert deterministic
  events, hard payload caps, no panic, bounded memory, and throughput/byte.

### `vtparse` 0.7.0

- **Function and evidence:** Implements the DEC ANSI parser state machine,
  modified for UTF-8, and emits actions through `VTActor`; it does not provide a
  terminal grid. See its versioned
  [API](https://docs.rs/vtparse/0.7.0/vtparse/) and the
  [packaged source](https://docs.rs/crate/vtparse/0.7.0/source/).
- **Release, maintenance, and license:** Version 0.7.0 was published 2025-04-19
  with SPDX `MIT` in official
  [package metadata](https://crates.io/api/v1/crates/vtparse/0.7.0). It is a
  0.x API with no stronger compatibility promise in the reviewed sources.
- **Targets:** Parser logic is OS-independent. Default features enable `std`;
  the package also exposes `alloc` and `no_std` feature paths.
- **Unsafe, dependencies, and security:** Its direct surface is much smaller
  than `termwiz`, but recursive unsafe and advisory review is still pending.
  The same payload bounds, partial-sequence, reply-policy, and malformed-input
  risks apply as for `vte`.
- **Replaceable boundary:** The same `VtParser` action contract as above; an
  adapter must normalize differences without discarding source bytes needed for
  diagnostics.
- **Validating PoC:** Run the identical parser corpus and fuzz seeds used for
  `vte`. Compare normalized actions, unsupported-sequence diagnostics, payload
  bounds, allocations, throughput, and behavior at every possible input split.

**Screening result:** Two parser-only candidates are available for a bounded
PoC. Neither candidate supplies terminal grid/state semantics, and no parser
choice is made.

## 1A. Terminal state engine

### `avt` 0.18.0

- **Function and evidence:** An embeddable terminal emulator with parser and
  screen state rather than a parser-only callback surface; see its
  [versioned API](https://docs.rs/avt/0.18.0/avt/) and
  [upstream source](https://github.com/asciinema/avt/tree/v0.18.0).
- **Release, maintenance, and license:** Version 0.18.0 was published
  2026-05-05 with SPDX `Apache-2.0` in
  [package metadata](https://crates.io/api/v1/crates/avt/0.18.0). It is a 0.x
  API; no additional compatibility guarantee was found.
- **Targets:** OS-independent Rust state engine; upstream also publishes a Web
  assembly use case. PTY, renderer, IME, and accessibility are outside this
  function.
- **Unsafe, dependencies, and security:** Recursive dependency and unsafe review
  is pending. A larger supplied state model reduces Noren code but increases the
  amount of untrusted terminal behavior accepted from one dependency.
- **Replaceable boundary:** `TerminalEngine` accepts bytes/resizes and returns an
  immutable snapshot, damage, replies, title changes, and bounded side effects.
- **Validating PoC:** Drive `avt` through the parser/state corpus and compare its
  grids, modes, replies, reflow, selection coordinates, memory, throughput, and
  fuzz stability against standards-derived expected fixtures. Parser-only
  libraries are not treated as state-engine oracles.

**Supported-candidate gap:** `avt` is the one independently released
parser-plus-screen-state candidate that passed this desk screen. The published
[`termwiz::Surface`](https://docs.rs/termwiz/0.23.3/termwiz/surface/struct.Surface.html)
models cells and display changes, but the reviewed API does not itself apply the
complete VT action/state policy under test. WezTerm's fuller
[`wezterm-term` workspace crate](https://github.com/wezterm/wezterm/blob/fa0a1da0f93f/term/Cargo.toml)
is versioned `0.1.0` in source, while an official
[registry API query](https://crates.io/api/v1/crates?page=1&per_page=10&q=wezterm-term)
retrieved 2026-08-03 returned no exact `wezterm-term` package. Consuming a moving
monorepo commit or vendoring it would not be a like-for-like supported release
candidate, so it is not counted as candidate two.

**PoC/drop gate:** Exercise resize/reflow, primary/alternate screen, margins,
wide/combining cells, modes, replies, title/clipboard side effects, scrollback,
damage, serialization boundaries, and hostile streams. Drop `avt` if the
adapter cannot expose bounded snapshots, damage and replies; if required
behavior needs a sustained fork; or if corpus, fuzz, memory, or throughput gates
fail. Before any adoption ADR, either identify and pin a second supportable
state engine or explicitly justify proceeding with a single-candidate market
gap and a Noren-owned replacement corpus. No state-engine choice is made here.

## 2. PTY and child-process control

### `portable-pty` 0.9.0

- **Function and evidence:** Provides a cross-platform PTY abstraction,
  `CommandBuilder`, master/slave handles, resize, and child control in its
  [versioned API](https://docs.rs/portable-pty/0.9.0/portable_pty/). Source lives
  in the active [WezTerm repository](https://github.com/wezterm/wezterm/tree/fa0a1da0f93f/pty).
- **Release, maintenance, and license:** Version 0.9.0 was published
  2025-02-11 with SPDX `MIT` in
  [package metadata](https://crates.io/api/v1/crates/portable-pty/0.9.0).
  WezTerm default-branch commit `fa0a1da0f93f` is dated 2026-08-02, but that
  branch activity does not promise a new `portable-pty` release or API support.
- **Targets:** Upstream documents Unix and Windows backends; macOS and Linux are
  in scope for the PoC.
- **Unsafe, dependencies, and security:** Platform PTYs and process creation
  necessarily cross OS/FFI boundaries; the recursive unsafe inventory is
  pending. Environment, working directory, file descriptors, signal handling,
  child lifetime, and command arguments are security-sensitive. External input
  must never become a concatenated shell command.
- **Replaceable boundary:** `PtyBackend` owns structured spawn arguments,
  byte-stream I/O, resize, exit status, and teardown; no UI object crosses it.
- **Validating PoC:** Spawn a fixed helper without a shell; test UTF-8 and binary
  I/O, resize storms, EOF, signal/exit races, descriptor leakage, child cleanup,
  backpressure, and UI survival after child failure on macOS and Linux.

### `nix` 0.31.3 PTY APIs

- **Function and evidence:** Lower-level Unix `openpty`/`forkpty` bindings in
  [`nix::pty`](https://docs.rs/nix/0.31.3/nix/pty/), leaving child lifecycle and
  async I/O policy to Noren.
- **Release, maintenance, and license:** Version 0.31.3 was published
  2026-05-11 with SPDX `MIT` and Rust 1.69 minimum in
  [package metadata](https://crates.io/api/v1/crates/nix/0.31.3).
- **Targets:** Unix only; macOS and Linux are covered, Windows is not relevant to
  the current product scope.
- **Unsafe, dependencies, and security:** The documented `forkpty` API is unsafe
  because only async-signal-safe operations are valid before `exec` in a
  multithreaded child. A lower-level API makes ownership and error paths Noren's
  responsibility. Feature selection, libc calls, and recursive unsafe need an
  audit.
- **Replaceable boundary:** The same `PtyBackend` contract as above, with all
  `fork`-sensitive code confined to the backend.
- **Validating PoC:** Reuse the portable-pty suite, then add a multithreaded
  fork/exec stress test, injected failures at each setup step, file-descriptor
  enumeration, controlling-terminal checks, and sanitizer runs.

**Screening result:** Both are viable for bounded experiments. The higher-level
and lower-level scopes create different implementation and audit burdens; no PTY
choice is made.

## 3A. GPU rendering

### `wgpu` 30.0.0

- **Function and evidence:** `wgpu` supplies a WebGPU-style graphics API and
  surface abstraction. See its versioned
  [`wgpu`](https://docs.rs/wgpu/30.0.0/wgpu/) API. Window creation and event
  delivery are explicitly outside this candidate's function.
- **Release, maintenance, and license:** `wgpu` 30.0.0 was published 2026-07-01
  with SPDX `MIT OR Apache-2.0`
  ([metadata](https://crates.io/api/v1/crates/wgpu/30.0.0)); a parallel 29.0.4
  patch was published later on 2026-07-02, so “latest by date” conflicts with
  “highest stable major.”
- **Targets:** Official backend documentation covers Metal on macOS and Vulkan
  or GL-family paths on Linux. Exact adapter/backend viability remains measured
  PoC evidence rather than a window-system claim.
- **Unsafe, dependencies, and security:** The GPU dependency tree is
  substantial. Driver isolation, surface lifetime, shader validation, device
  loss, raw-window handles, and transitive unsafe require audit. Renderer
  failure must not kill the PTY/session.
- **Replaceable boundary:** `RenderBackend` consumes immutable
  grid/damage/glyph batches and reports recoverable device errors; it receives
  an opaque surface handle rather than owning the event loop.
- **Validating PoC:** Render a deterministic multilingual grid and damage trace
  on Apple Silicon/macOS and two Linux paths. Measure p50/p95 frame time, upload
  bytes, idle CPU, memory, resize latency, scale-factor changes, device loss,
  and surface recreation. The controlled renderer comparison uses `winit`
  0.30.13 for both candidates, so its results cannot count as an independent
  window/event comparison. Before measurement, run both pinned `wgpu` 30.0.0
  and 29.0.4 lines under the identical trace, or record and justify a single
  selected measurement line; never combine results across lines.

### `glow` 0.18.0 with `glutin` 0.32.3 context/surface support

- **Function and evidence:** `glow` is a low-level OpenGL binding abstraction;
  `glutin` selects configurations and creates OpenGL displays, contexts, and
  surfaces. Neither is a window/event-loop library. See the versioned
  [`glow`](https://docs.rs/glow/0.18.0/glow/) and
  [`glutin`](https://docs.rs/glutin/0.32.3/glutin/) APIs.
- **Release, maintenance, and license:** `glow` 0.18.0 was published 2026-07-09
  with SPDX `MIT OR Apache-2.0 OR Zlib`
  ([metadata](https://crates.io/api/v1/crates/glow/0.18.0)). `glutin` 0.32.3 was
  published 2025-04-30 with SPDX `Apache-2.0`
  ([metadata](https://crates.io/api/v1/crates/glutin/0.32.3)); default-branch
  commit [`06dc57bffcb1`](https://github.com/rust-windowing/glutin/commit/06dc57bffcb1)
  is dated 2026-07-21.
- **Targets:** Native OpenGL paths exist for macOS and Linux, with Linux backend
  selection documented by glutin. Exact supported driver/profile combinations
  remain a PoC result.
- **Unsafe, dependencies, and security:** OpenGL calls and context-currentness
  expose more low-level invariants to the caller. Function loading, object
  lifetime, context loss, shader inputs, platform libraries, and transitive
  unsafe need audit.
- **Replaceable boundary:** The same `RenderBackend` seam, without exposing GL
  object IDs or context-currentness outside the renderer.
- **Validating PoC:** Use the identical trace and hardware matrix as the wgpu
  stack; add context recreation, missing-extension, GL error, and software
  renderer cases. This OpenGL PoC also uses `winit` 0.30.13 for window/events
  and `glutin-winit` 0.5.0 only as the bridge into `glutin`; compare renderer
  behavior and measured cost, not feature-list claims.

**Screening result:** Two renderer candidates are viable for PoCs. They share
the same window/event host in the controlled experiment; this section supplies
no second windowing candidate. No renderer choice is made.

## 3B. Window and event integration

### `winit` 0.30.13

- **Function and evidence:** Cross-platform window creation and application,
  window, device, lifecycle, scale-factor, keyboard, pointer, and IME events in
  its versioned [`winit`](https://docs.rs/winit/0.30.13/winit/) API. It does not
  render terminal cells.
- **Release, maintenance, and license:** Version 0.30.13 was published
  2026-03-02 with SPDX `Apache-2.0` in official
  [package metadata](https://crates.io/api/v1/crates/winit/0.30.13). The 0.31
  line was beta-only at retrieval and is not counted as the stable comparison
  line; no stronger 0.x API compatibility promise was found.
- **Targets:** Upstream documents AppKit on macOS and Wayland/X11 on Linux.
  Thread ownership, compositor/desktop differences, and headless behavior need
  local measurement.
- **Unsafe, dependencies, and security:** Platform backends, raw-window handles,
  input provenance, event ordering, lifecycle re-entry, clipboard/drag-and-drop,
  and transitive native code need audit. Synthetic input must not be mistaken
  for trusted user activation.
- **Replaceable boundary:** `WindowInputBackend` owns windows and the event loop,
  normalizes events with timestamps/provenance, and lends opaque surface handles
  to a renderer without exposing `winit` types elsewhere.
- **Validating PoC:** Replay a timestamped lifecycle/input trace on AppKit,
  Wayland, and X11: create/destroy, suspend/resume, focus, multiple windows,
  scale changes, resize storms, redraw scheduling, keyboard/pointer/IME events,
  monitor removal, and orderly shutdown. Capture event ordering, idle wakeups,
  latency, and failures.

**Supported-candidate gap:** `glutin` is not candidate two: its documented scope
is GL display/config/context/surface management. The convenience
[`glutin-winit` 0.5.0 dependency list](https://crates.io/api/v1/crates/glutin-winit/0.5.0/dependencies)
contains a required `winit ^0.30.0` dependency, so it is an adapter to the same
window/event implementation, not an independent implementation. GTK4 is kept
as a separately measured full-toolkit path for IME and accessibility; counting
it here without a comparable ownership/event PoC would hide that coupling. No
second like-for-like, independently supported window/event candidate was
evidenced in this pass.

**PoC/drop gate:** Drop `winit` if the adapter cannot produce deterministic
lifecycle/input semantics on all three target backends, if event-thread or raw
handle constraints prevent renderer recovery, if IME/accessibility composition
cannot be made correct, or if a sustained fork is required. Before an adoption
ADR, either pin and run the same trace against a second supportable candidate or
explicitly justify the single-candidate market gap and replacement plan. No
window/event choice is made.

## 4. Font parsing and shaping

### `harfrust` 0.12.0

- **Function and evidence:** A Rust text-shaping implementation under the
  HarfBuzz organization; see its
  [versioned API](https://docs.rs/harfrust/0.12.0/harfrust/) and
  [packaged source](https://docs.rs/crate/harfrust/0.12.0/source/).
- **Release, maintenance, and license:** Version 0.12.0 was published
  2026-07-03 with SPDX `MIT` and Rust 1.85 minimum in
  [package metadata](https://crates.io/api/v1/crates/harfrust/0.12.0).
- **Targets:** OS-independent shaping suitable for macOS and Linux; font
  discovery, fallback, rasterization, and GPU atlas management are separate.
- **Unsafe, dependencies, and security:** Font files are untrusted structured
  input. Recursive unsafe/dependency and malformed-font audit is pending; a Rust
  implementation is not automatically memory-safe if dependencies or caller
  invariants use unsafe.
- **Replaceable boundary:** `TextShaper` accepts validated font bytes, face
  index, script/language/direction/features, and text; returns bounded glyph IDs,
  clusters, advances, and offsets.
- **Validating PoC:** Shape pinned open-license fonts and a corpus covering Latin
  ligatures, Arabic, Devanagari, combining marks, variation selectors, emoji,
  fallback boundaries, malformed fonts, and huge tables. Differentially compare
  against the pinned C
  [HarfBuzz 14.3.0](https://github.com/harfbuzz/harfbuzz/releases/tag/14.3.0)
  reference and measure allocations/latency; record it as a native toolchain
  input rather than inferring a version correspondence from `harfrust`.

### `swash` 0.2.10

- **Function and evidence:** Font introspection, shaping, and scaler APIs are
  documented in the versioned
  [`swash`](https://docs.rs/swash/0.2.10/swash/) crate and
  [packaged source](https://docs.rs/crate/swash/0.2.10/source/).
- **Release, maintenance, and license:** Version 0.2.10 was published
  2026-07-17 with SPDX `Apache-2.0 OR MIT` in
  [package metadata](https://crates.io/api/v1/crates/swash/0.2.10). It is a 0.x
  API and no separate compatibility promise was found.
- **Targets:** OS-independent core suitable for macOS/Linux; system font
  discovery and platform fallback policy remain external.
- **Unsafe, dependencies, and security:** It combines more font functions than a
  shaping-only candidate, increasing evaluation scope. Malformed-font behavior,
  table-size bounds, cache growth, recursive dependencies, and unsafe remain
  audit gates.
- **Replaceable boundary:** The same `TextShaper` seam, with optional rasterizer
  functionality kept behind a distinct `GlyphRasterizer` PoC seam.
- **Validating PoC:** Run the identical shaping/font fuzz corpus, compare clusters
  and advances to the same pinned HarfBuzz 14.3.0 reference, then measure raster
  quality, atlas churn, memory, fallback cost, and behavior under invalid fonts.

**Screening result:** Two current Rust candidates exist. Neither removes the
need to define font discovery, fallback, rasterization, cell alignment, and
license handling for user and bundled fonts.

## 5. Unicode grapheme and cell width

### `unicode-width` 0.2.2

- **Function and evidence:** Computes `char`/`str` widths using documented UAX
  #11 and additional rules, exposes a Unicode version constant, and supports a
  CJK mode; see the
  [versioned API and rule list](https://docs.rs/unicode-width/0.2.2/unicode_width/).
- **Release, maintenance, and license:** Version 0.2.2 was published
  2025-10-06 with SPDX `MIT OR Apache-2.0` and Rust 1.66 minimum in
  [package metadata](https://crates.io/api/v1/crates/unicode-width/0.2.2).
- **Targets:** `no_std`, OS-independent, and usable on macOS/Linux.
- **Unsafe, dependencies, and security:** The docs show only optional Rust
  workspace-core dependencies, but a source audit is still required. Width is a
  terminal-compatibility and visual-integrity boundary: cursor desynchronization
  can mislead users even without memory corruption.
- **Replaceable boundary:** `CellWidth` accepts a grapheme plus explicit Unicode
  and ambiguous-width policy and returns a bounded cell count; parser state does
  not call the crate directly.
- **Validating PoC:** Assert the crate's Unicode version, run Unicode 17 UAX #29
  grapheme data, UAX #11 data, emoji ZWJ/variation fixtures, ambiguous-width
  modes, combining marks, and differential fixtures against pinned terminals.

### `unicode-display-width` 0.3.0 (provisional)

- **Function and evidence:** Computes grapheme-oriented notional display width.
  Its [versioned docs](https://docs.rs/unicode-display-width/0.3.0/unicode_display_width/)
  explicitly target Unicode 15.1.0 and list terminal/font limitations.
- **Release, maintenance, and license:** Version 0.3.0 was published
  2023-11-15 with SPDX `MIT` in
  [package metadata](https://crates.io/api/v1/crates/unicode-display-width/0.3.0).
  Default-branch commit
  [`1f853b13d0d2`](https://github.com/jameslanska/unicode-display-width/commit/1f853b13d0d2)
  is dated 2026-01-19, but no newer crate release was observed.
- **Targets:** OS-independent Rust; its documented Unicode 15.1 data is behind
  the Unicode 17.0 reference baseline used by this report.
- **Unsafe, dependencies, and security:** Official metadata lists
  `unicode-segmentation`; recursive unsafe and generated-table provenance need
  review. The documented `{1,2}`-per-grapheme model and listed Indic limitations
  may conflict with terminal/application behavior.
- **Replaceable boundary:** The same `CellWidth` seam, with algorithm/version
  recorded in snapshots.
- **Validating PoC:** Run the identical Unicode 17 and terminal differential
  corpus. Count disagreements by sequence class and reject silent Unicode-version
  drift; do not “fix” differences from model memory.

**Screening result:** `unicode-width` is a current direct candidate. The second
candidate is provisional rather than comparably current because its published
data targets Unicode 15.1.0. UAX #11 itself warns that its property needs
terminal-specific tailoring. This is an explicit evidence gap, not a forced
two-equal-candidate conclusion.

## 6. Input method editors

### `winit` 0.30.13 IME events

- **Function and evidence:** The versioned window API exposes IME enablement,
  purpose, and cursor-area methods, and the event API exposes preedit/commit
  events; see
  [`Window::set_ime_allowed`](https://docs.rs/winit/0.30.13/winit/window/struct.Window.html#method.set_ime_allowed)
  and [`Ime`](https://docs.rs/winit/0.30.13/winit/event/enum.Ime.html).
- **Release, maintenance, and license:** Version 0.30.13 was published
  2026-03-02 with SPDX `Apache-2.0` in
  [package metadata](https://crates.io/api/v1/crates/winit/0.30.13). A 0.31 beta
  exists; this comparison does not infer stable/beta API parity.
- **Targets:** macOS AppKit, Linux Wayland, and Linux X11 backends. Identical API
  shapes do not prove identical platform behavior.
- **Unsafe, dependencies, and security:** Large platform dependency surface and
  raw-window integration require audit. Protected/password contexts, preedit
  lifetime, candidate-window coordinates, focus, and interaction with terminal
  key protocols are security and correctness concerns.
- **Replaceable boundary:** `WindowInputBackend` emits semantic composition,
  committed text, physical/logical key, modifier, focus, and scale-aware cursor
  events before shortcut policy or PTY encoding.
- **Validating PoC:** Record event traces for Japanese and Chinese IMEs, dead
  keys, emoji, compose, Option/Alt, AltGr, repeat, focus loss, pane movement,
  scale changes, and cancellation on macOS, Wayland, and X11.

### GTK4 `GtkIMContext` through `gtk4` 0.11.4

- **Function and evidence:** GTK's official
  [`GtkIMContext`](https://docs.gtk.org/gtk4/class.IMContext.html) defines the
  input-method context; Rust bindings are in the versioned
  [`gtk4`](https://docs.rs/gtk4/0.11.4/gtk4/) crate.
- **Release, maintenance, and license:** Rust binding 0.11.4 was published
  2026-06-29 with SPDX `MIT` and Rust 1.92 minimum in
  [package metadata](https://crates.io/api/v1/crates/gtk4/0.11.4). The native GTK
  library is separately licensed under
  [`LGPL-2.1-or-later`](https://gitlab.gnome.org/GNOME/gtk/-/blob/main/meson.build#L12);
  exact packaged native versions and terms need inventory.
- **Targets:** GTK supports Linux and macOS builds, but its primary Linux desktop
  integration and macOS packaging behavior need direct measurement.
- **Unsafe, dependencies, and security:** GObject/C FFI and a large native
  toolkit dependency tree expand the audit and packaging surface. Embedding only
  an IM context outside a GTK-owned window is not assumed to be supported.
- **Replaceable boundary:** A toolkit-specific `WindowInputBackend` produces the
  same semantic events as the winit PoC.
- **Validating PoC:** Run the identical IME trace suite in a GTK-owned test
  window; measure preedit rendering, candidate placement, focus, accessibility
  interaction, binary size, startup, and packaging on both target OSes.

**Screening result:** Two integration paths are documented, but their window
ownership differs materially. The GTK4 path entails GTK window and main-loop
ownership as described in section 3B, so these paths are not independently
selectable and the IME decision remains coupled to the window/event decision.
No claim is made that GTK IME can be cleanly mixed with a non-GTK window.

## 7. SSH transport

### `russh` 0.62.5

- **Function and evidence:** Async Rust SSH client/server implementation with
  client handlers and channels in the versioned
  [`russh`](https://docs.rs/russh/0.62.5/russh/) API and
  [packaged source](https://docs.rs/crate/russh/0.62.5/source/).
- **Release, maintenance, and license:** Version 0.62.5 was published
  2026-07-31 with SPDX `Apache-2.0` and Rust 1.85 minimum in
  [package metadata](https://crates.io/api/v1/crates/russh/0.62.5).
- **Targets:** Rust networking supports macOS/Linux; crypto backend, agent, and
  platform feature behavior must be pinned in the PoC.
- **Unsafe, dependencies, and security:** SSH is a high-risk protocol and crypto
  dependency surface. Host-key verification, algorithm policy, rekey, agent use,
  keyboard-interactive prompts, channel/window flow control, timeouts, and
  secret-zeroization require source and advisory review. Upstream July 2026
  advisories record pre/post-auth remote panics fixed in 0.62.4
  ([example](https://github.com/warp-tech/russh/security/advisories/GHSA-5xvq-cp9x-6p6r))
  and a channel-callback flaw fixed in the compared 0.62.5
  ([GHSA-m65r-rprj-r5rg](https://github.com/warp-tech/russh/security/advisories/GHSA-m65r-rprj-r5rg)).
  Pinning 0.62.5 is evidence of those fixes, not a complete audit conclusion.
- **Replaceable boundary:** `SshTransport` receives resolved connection options
  and a host-key decision callback, then exposes typed session/channel events;
  config parsing and UI prompts stay outside it.
- **Validating PoC:** Use ephemeral local OpenSSH 10.4 fixtures for correct and
  changed host keys, known-host hashing, agent and password auth,
  keyboard-interactive, rekey, resize, slow/abrupt peers, channel backpressure,
  reconnect boundaries, and algorithm negotiation. Capture packets only with
  test credentials.

### `ssh2` 0.9.6 over libssh2

- **Function and evidence:** Rust bindings that document a client-only safe
  interface over libssh2 in the versioned
  [`ssh2`](https://docs.rs/ssh2/0.9.6/ssh2/) API. The native implementation is
  [libssh2](https://libssh2.org/).
- **Release, maintenance, and license:** Rust crate 0.9.6 was published
  2026-06-30 with SPDX `MIT OR Apache-2.0` in
  [package metadata](https://crates.io/api/v1/crates/ssh2/0.9.6). Bundled/system
  libssh2 has its own
  [BSD-style license](https://github.com/libssh2/libssh2/blob/master/COPYING);
  exact native version depends on features and build environment and must be
  recorded.
- **Targets:** libssh2 and the wrapper support macOS/Linux; native TLS/crypto and
  build dependencies vary with selected features.
- **Unsafe, dependencies, and security:** C FFI, native library provenance,
  OpenSSL or alternative crypto, callback lifetime, blocking/nonblocking mode,
  host-key policy, agent behavior, and advisories require joint review. The safe
  Rust surface does not remove native memory-safety risk. libssh2 has published
  native memory-safety advisories such as
  [CVE-2019-3855](https://libssh2.org/CVE-2019-3855.html), fixed in libssh2
  1.8.1; that history makes capture of the actual bundled/system version a gate,
  not evidence that a current build is affected.
- **Replaceable boundary:** The same `SshTransport` contract, with FFI/native
  handles confined to the adapter.
- **Validating PoC:** Run the identical OpenSSH fixture suite, add sanitizer and
  native-library version capture, then compare supported algorithms,
  authentication methods, flow control, error fidelity, latency, and teardown.

**Screening result:** Two client candidates are viable for local tests. Neither
is assumed to implement OpenSSH config semantics; that is a separate category.

## 8. OpenSSH configuration resolution

### Delegate resolution to OpenSSH 10.4p1 (`ssh -G` / `ssh`)

- **Function and evidence:** `ssh -G` prints configuration after evaluating
  `Host` and `Match` blocks according to the official
  [`ssh(1)` manual](https://man.openbsd.org/ssh.1), while
  [`ssh_config(5)`](https://man.openbsd.org/ssh_config.5) defines ordering,
  includes, tokens, canonicalization, proxies, and executable directives.
- **Release, maintenance, and license:** Official
  [release notes](https://www.openssh.com/releasenotes.html) record OpenSSH
  10.4/10.4p1 on 2026-07-06 and security fixes in that release. Portable source
  uses the upstream
  [OpenSSH license collection](https://github.com/openssh/openssh-portable/blob/V_10_4_P1/LICENCE)
  (BSD/ISC-style components; no single SPDX expression asserted here).
- **Targets:** System OpenSSH is available on macOS and mainstream Linux, but
  vendor versions, paths, patches, and sandbox behavior differ and must be
  captured rather than hard-coded.
- **Unsafe, dependencies, and security:** Invoking a structured executable with
  arguments avoids Noren's own shell interpolation but creates a subprocess
  trust boundary. It does **not** make an arbitrary destination token safe. The
  official
  [`Match exec` documentation](https://man.openbsd.org/ssh_config.5#exec) says
  its command is executed under the user's shell, and the official
  [token rules](https://man.openbsd.org/ssh_config.5#TOKENS) say token expansion
  performs no quoting or escaping of shell characters. Because `%h` is the
  resolved remote hostname and `%n` is the original command-line hostname, a
  destination containing shell metacharacters can be expanded directly into
  `Match exec` shell text even when Noren passed `ssh`, `-G`, and the destination
  as separate argv elements. `ProxyCommand`, `LocalCommand`, environment,
  config permissions, timeouts, and output bounds are also security-critical.
  Noren must not concatenate an additional shell command or assume `ssh -G` is a
  side-effect-free host-discovery API.
- **Replaceable boundary:** `SshConfigResolver` returns a provenance-tagged,
  typed, redacted result for a host; transport execution remains separate.
- **Validating PoC:** With disposable isolated fixture configs, compare
  `ssh -G host` output across OpenSSH 9.x and 10.4, `Host`/`Match` precedence,
  `Include`, tokens, canonicalization, ProxyJump, malformed files, huge includes,
  timeout, locale, and redaction. Add host inputs containing isolated harmless
  fixtures for semicolons, whitespace, quotes, dollar-command substitution, and
  backticks; have `Match exec` write only a nonce sentinel into a disposable
  directory, then assert whether `%h`/`%n` changed shell parsing. The inert host
  listing experiment parses aliases and keeps executable predicates opaque; it
  must never invoke `ssh -G`. A separate, explicit user-requested resolution
  experiment may invoke pinned OpenSSH after surfacing that config evaluation
  can execute user-configured shell text. Record these two modes separately and
  never silently upgrade passive listing into evaluation.

### `ssh2-config` 0.7.2

- **Function and evidence:** Pure-Rust parser/resolver intended for `ssh2`, with
  its exposed fields, resolution rules, and missing features documented in the
  [versioned crate docs](https://docs.rs/ssh2-config/0.7.2/ssh2_config/).
- **Release, maintenance, and license:** Version 0.7.2 was published
  2026-08-01 with SPDX `MIT` and Rust 1.88 minimum in
  [package metadata](https://crates.io/api/v1/crates/ssh2-config/0.7.2).
- **Targets:** OS-independent parser; default paths, tilde expansion, filesystem
  permissions, and system config paths are platform policy outside parsing.
- **Unsafe, dependencies, and security:** Recursive audit is pending. Includes,
  globbing, cycles, file sizes, unknown/unsupported fields, executable
  directives, token expansion, and divergence from the locally installed
  OpenSSH version are primary risks. Documented fields are not assumed to have
  identical OpenSSH semantics.
- **Replaceable boundary:** The same `SshConfigResolver` result and provenance
  model, with unsupported directives preserved as diagnostics rather than
  silently invented.
- **Validating PoC:** Differentially resolve a generated and curated config
  corpus against pinned `ssh -G` versions. Classify every difference by
  directive/version; fuzz parser limits, include cycles, permissions, and
  malformed input; never run `ProxyCommand` or `Match exec` in the parser PoC.

**Screening result:** Both approaches are viable experiments with different
trust and fidelity tradeoffs. OpenSSH behavior is versioned, and the Rust parser
must not be described as equivalent until the differential suite proves a
defined subset.

## 9. Local IPC

### `interprocess` 2.4.3

- **Function and evidence:** Cross-platform local sockets and named-pipe
  abstractions with sync and Tokio integrations in the versioned
  [`interprocess`](https://docs.rs/interprocess/2.4.3/interprocess/) API.
- **Release, maintenance, and license:** Version 2.4.3 was published
  2026-08-01 with SPDX `0BSD OR Apache-2.0` and Rust 1.75 minimum in
  [package metadata](https://crates.io/api/v1/crates/interprocess/2.4.3).
- **Targets:** Unix-domain/local sockets cover macOS/Linux; cross-platform names
  and namespace behavior still vary by OS.
- **Unsafe, dependencies, and security:** Peer identity, socket directory
  permissions, symlink/race resistance, stale endpoints, descriptor passing,
  message framing, backpressure, and size/time limits are application policy.
  No uniform peer-credential API is inferred without documentation and tests.
- **Replaceable boundary:** `LocalIpc` transports length-bounded, versioned
  messages after an explicit peer-authorization decision; commands are typed.
- **Validating PoC:** Test private-runtime-directory creation, competing server,
  symlink/stale socket, wrong UID, permissions, partial/oversized frames,
  cancellation, backpressure, restart, and terminal-stream injection attempts.

### Tokio 1.53.1 Unix sockets

- **Function and evidence:** Lower-level async
  [`UnixListener`](https://docs.rs/tokio/1.53.1/tokio/net/struct.UnixListener.html)
  and [`UnixStream`](https://docs.rs/tokio/1.53.1/tokio/net/struct.UnixStream.html)
  leave naming, framing, authorization, and protocol policy to Noren.
- **Release, maintenance, and license:** Tokio 1.53.1 was published 2026-07-20
  with SPDX `MIT` and Rust 1.71 minimum in
  [package metadata](https://crates.io/api/v1/crates/tokio/1.53.1).
- **Targets:** Unix API covers macOS/Linux; feature-gated runtime and I/O
  dependencies must be pinned.
- **Unsafe, dependencies, and security:** Smaller IPC abstraction does not mean a
  smaller total runtime. Runtime features, OS socket calls, peer credentials,
  permissions, framing, cancellation, and resource exhaustion need audit.
- **Replaceable boundary:** The same `LocalIpc` contract; Tokio types stay inside
  the adapter.
- **Validating PoC:** Run the identical adversarial IPC suite and compare binary
  size, dependency graph, latency, allocations, idle wakeups, cancellation, and
  error fidelity.

**Screening result:** Two viable transport layers exist. Neither supplies an
authorization or application protocol; those must remain Noren-owned and tested.

## 10. TOML parsing and editing

### `toml` 1.1.4+spec-1.1.0

- **Function and evidence:** Serde-oriented TOML parsing/serialization in the
  versioned [`toml`](https://docs.rs/toml/1.1.4+spec-1.1.0/toml/) API, aligned by
  version label with the official [TOML 1.1.0 specification](https://toml.io/en/v1.1.0).
- **Release, maintenance, and license:** Version 1.1.4+spec-1.1.0 was published
  2026-07-28 with SPDX `MIT OR Apache-2.0` and Rust 1.85 minimum in
  [package metadata](https://crates.io/api/v1/crates/toml/1.1.4%2Bspec-1.1.0).
- **Targets:** OS-independent Rust for macOS/Linux.
- **Unsafe, dependencies, and security:** Config size/depth, duplicate/unknown
  keys, error locations, Unicode keys, dates, allocation, and serde defaults
  require bounded tests. File watching, atomic writes, includes, migrations, and
  failed-reload rollback are outside the parser.
- **Replaceable boundary:** `ConfigCodec` converts bytes to a versioned raw
  configuration plus diagnostics; validation and transactional activation are
  Noren-owned.
- **Validating PoC:** Run TOML spec fixtures plus Noren schemas with invalid,
  duplicate, unknown, huge, and deeply nested data; measure error spans,
  allocations, parse time, round trip, failed-reload preservation, and fuzzing.

### `taplo` 0.14.0

- **Function and evidence:** TOML syntax/parser DOM and tooling APIs are exposed
  in versioned [`taplo`](https://docs.rs/taplo/0.14.0/taplo/) documentation and
  [upstream source](https://github.com/tamasfe/taplo).
- **Release, maintenance, and license:** Version 0.14.0 was published
  2025-05-22 with SPDX `MIT` and a publisher-declared Rust 1.74 minimum in
  [package metadata](https://crates.io/api/v1/crates/taplo/0.14.0).
  Default-branch commit
  [`08f343be02ce`](https://github.com/tamasfe/taplo/commit/08f343be02ce) is dated
  2026-07-28. Branch activity does not establish a newer crate API.
- **Targets:** OS-independent parser/tooling core; optional tooling dependencies
  and features need a locked inventory.
- **Unsafe, dependencies, and security:** A syntax DOM can preserve richer source
  information but expands dependency and error-recovery behavior to audit.
  Bounds, semantic validation, atomic reload, and migration remain Noren-owned.
- **Replaceable boundary:** The same `ConfigCodec`, optionally returning a
  source-preserving edit representation behind a separate capability.
- **Validating PoC:** Run the identical spec/schema/fuzz suite; additionally test
  comment/format preservation, edits, diagnostics after recovery, dependency
  size, and deterministic serialization.

**Screening result:** Two viable parser paths exist. Selection depends on
measured configuration-editing requirements that are not approved in this
report.

## 11. Logging, panic reports, and native crash diagnostics

### Structured logging: `tracing` 0.1.44 + `tracing-subscriber` 0.3.23

- **Function and evidence:** Structured spans/events and subscriber filtering
  through versioned [`tracing`](https://docs.rs/tracing/0.1.44/tracing/) and
  [`tracing-subscriber`](https://docs.rs/tracing-subscriber/0.3.23/tracing_subscriber/)
  APIs.
- **Release, maintenance, and license:** `tracing` 0.1.44 was published
  2025-12-18 and `tracing-subscriber` 0.3.23 on 2026-03-13; both declare SPDX
  `MIT` in official [core metadata](https://crates.io/api/v1/crates/tracing/0.1.44)
  and [subscriber metadata](https://crates.io/api/v1/crates/tracing-subscriber/0.3.23).
- **Targets:** OS-independent core suitable for macOS/Linux.
- **Unsafe, dependencies, and security:** Enabled subscriber/format features
  determine dependencies. Field values can leak SSH destinations, commands,
  paths, clipboard data, protected input, and secrets; recursion, log injection,
  file permissions, rotation, and disk exhaustion require policy.
- **Replaceable boundary:** `DiagnosticsSink` accepts a closed redacted event
  schema; library-specific spans do not cross core interfaces.
- **Validating PoC:** Generate high-rate PTY/render/SSH events with synthetic
  secrets; prove redaction, deterministic fields, rotation/retention caps,
  backpressure, disabled-cost budget, crash-flush behavior, and no UI stall.

### Conventional logging: `log` 0.4.33 + `env_logger` 0.11.11

- **Function and evidence:** A facade plus environment-configurable logger in
  versioned [`log`](https://docs.rs/log/0.4.33/log/) and
  [`env_logger`](https://docs.rs/env_logger/0.11.11/env_logger/) APIs.
- **Release, maintenance, and license:** Both were published in June 2026 and
  declare SPDX `MIT OR Apache-2.0` in official
  [`log` metadata](https://crates.io/api/v1/crates/log/0.4.33) and
  [`env_logger` metadata](https://crates.io/api/v1/crates/env_logger/0.11.11).
- **Targets:** OS-independent core suitable for macOS/Linux.
- **Unsafe, dependencies, and security:** A simpler facade still needs the same
  redaction, rotation, injection, retention, permission, and exhaustion policy.
  Environment-controlled filters are untrusted configuration.
- **Replaceable boundary:** The same closed `DiagnosticsSink` schema, flattened
  to facade records inside the adapter.
- **Validating PoC:** Run the identical load/redaction suite; compare structured
  context loss, allocation, disabled-call overhead, filtering, rotation support,
  and failure isolation.

### Panic reports: `color-eyre` 0.6.5 and `human-panic` 2.0.8

- **Function and evidence:** `color-eyre` installs formatted error/panic hooks;
  `human-panic` generates user-facing panic reports. See versioned
  [`color-eyre`](https://docs.rs/color-eyre/0.6.5/color_eyre/) and
  [`human-panic`](https://docs.rs/human-panic/2.0.8/human_panic/) APIs.
- **Release, maintenance, and license:** `color-eyre` 0.6.5 was published
  2025-05-30 and `human-panic` 2.0.8 on 2026-04-02; both declare SPDX
  `MIT OR Apache-2.0` in official
  [color-eyre metadata](https://crates.io/api/v1/crates/color-eyre/0.6.5) and
  [human-panic metadata](https://crates.io/api/v1/crates/human-panic/2.0.8).
- **Targets:** Both are Rust-level and usable on macOS/Linux.
- **Unsafe, dependencies, and security:** Panic hooks are not native crash
  handlers. Backtraces, environment, paths, command lines, and report files can
  expose private data. Hook recursion, unwinding/abort profiles, permissions,
  and GUI presentation need testing.
- **Replaceable boundary:** `PanicReporter` receives an already-redacted incident
  envelope and may write only to a private bounded location.
- **Validating PoC:** Trigger panics on UI/background threads under unwind and
  abort builds; inspect redaction, permissions, recursion, report size, offline
  behavior, and whether child/session cleanup still occurs. Compare both hooks.

### Native crashes: Rust minidump stack vs Crashpad

- **Function and evidence:** Rust option combines
  [`crash-handler` 0.8.0](https://docs.rs/crash-handler/0.8.0/crash_handler/) and
  [`minidump-writer` 0.13.0](https://docs.rs/minidump-writer/0.13.0/minidump_writer/).
  Alternative [Crashpad](https://chromium.googlesource.com/crashpad/crashpad/)
  is Chromium's out-of-process native crash-reporting system.
- **Release, maintenance, and license:** Both Rust crates were published
  2026-07-20; `crash-handler` declares `MIT OR Apache-2.0` and
  `minidump-writer` declares `MIT` in
  [handler metadata](https://crates.io/api/v1/crates/crash-handler/0.8.0) and
  [writer metadata](https://crates.io/api/v1/crates/minidump-writer/0.13.0).
  Crashpad main commit
  [`ad1827ddbc03`](https://chromium.googlesource.com/crashpad/crashpad/+/ad1827ddbc03)
  is dated 2026-07-28 and its source license is
  [Apache-2.0](https://chromium.googlesource.com/crashpad/crashpad/+/refs/heads/main/LICENSE).
- **Targets:** Both paths cover native macOS/Linux scenarios, but exact signal,
  sandbox, architecture, and dump support must be measured per target.
- **Unsafe, dependencies, and security:** Signal/exception handlers operate
  under severe async-safety constraints. Dumps can contain secrets and terminal
  contents. The Rust path uses native/unsafe mechanisms; Crashpad adds C++,
  subprocess, upload, and build-system surface. Neither upload nor consent is
  implied by capture.
- **Replaceable boundary:** `CrashCapture` writes a bounded local artifact with
  explicit consent/retention policy; symbolication and any upload are separate.
- **Validating PoC:** Induce controlled crashes in disposable builds on each OS;
  verify handler reliability, dump readability, redaction feasibility, private
  permissions, disk caps, recursion, startup cost, helper cleanup, offline mode,
  symbolization, and no automatic upload.

**Screening result:** There are two logging paths, two Rust panic-report hooks,
and two native-crash approaches. Panic hooks are not substitutes for native
crash capture. No diagnostics stack is selected.

## 12. Packaging

### Packaging: `cargo-dist` 0.32.0

- **Function and evidence:** Release automation and artifact/installer generation
  documented in the versioned
  [`cargo-dist`](https://axodotdev.github.io/cargo-dist/) manual and
  [upstream source](https://github.com/axodotdev/cargo-dist/tree/v0.32.0).
- **Release, maintenance, and license:** Version 0.32.0 was published
  2026-05-22 with SPDX `MIT OR Apache-2.0` and Rust 1.74 minimum in
  [package metadata](https://crates.io/api/v1/crates/cargo-dist/0.32.0).
- **Targets:** Documentation covers macOS, Linux, and Windows release artifacts;
  exact Noren formats, signing, notarization, repositories, and architectures
  are undecided.
- **Unsafe, dependencies, and security:** This is build/release tooling with a
  broad supply-chain surface. CI permissions, action pinning, provenance,
  checksums, signing keys, notarization, reproducibility, and generated script
  review are gates.
- **Replaceable boundary:** `ReleasePackager` consumes already-built pinned
  binaries and emits a manifest/artifact set; runtime code does not depend on it.
- **Validating PoC:** Produce unsigned test artifacts in a locked CI sandbox;
  inspect generated workflows/installers, SBOM/licenses, checksums, provenance,
  reproducibility, architecture coverage, uninstall, and package-manager rules.

### Packaging: `cargo-packager` 0.11.8

- **Function and evidence:** Application bundle/package generation documented in
  the versioned
  [`cargo-packager`](https://docs.rs/cargo-packager/0.11.8/cargo_packager/) API
  and [upstream source](https://github.com/crabnebula-dev/cargo-packager).
- **Release, maintenance, and license:** Version 0.11.8 was published
  2025-11-27 with SPDX `Apache-2.0 OR MIT` in
  [package metadata](https://crates.io/api/v1/crates/cargo-packager/0.11.8).
  Default-branch commit
  [`37a538e76608`](https://github.com/crabnebula-dev/cargo-packager/commit/37a538e76608)
  is dated 2026-03-21.
- **Targets:** Documents macOS and Linux package formats; exact support must be
  tested for Noren's eventual binary layout.
- **Unsafe, dependencies, and security:** Same build-chain, signing, provenance,
  installer-script, and credential risks as above. Native packaging tools add
  host dependencies.
- **Replaceable boundary:** The same build-time `ReleasePackager` input/output
  manifest contract.
- **Validating PoC:** Run the identical packaging matrix and compare artifact
  contents, signing/notarization integration, deterministic output, SBOM/license
  capture, host dependencies, install/uninstall, and CI privilege needs.

**Screening result:** Two packaging candidates are available for bounded
artifact-generation PoCs. Packaging is not runtime update discovery or
installation, and no packager is selected.

## 12A. Update discovery and notification

### `update-informer` 1.3.0

- **Function and evidence:** Checks a configured registry/repository for a newer
  version and returns metadata for application-owned notification; it does not
  replace the executable. See the versioned
  [API](https://docs.rs/update-informer/1.3.0/update_informer/).
- **Release, maintenance, and license:** Version 1.3.0 was published 2025-07-11
  with SPDX `MIT` in official
  [package metadata](https://crates.io/api/v1/crates/update-informer/1.3.0);
  default-branch commit
  [`4a4d526ea583`](https://github.com/mgrachev/update-informer/commit/4a4d526ea583)
  is dated 2025-12-01.
- **Targets:** Pure Rust client behavior is macOS/Linux-capable, subject to the
  enabled HTTP/TLS backend, platform trust store, proxy, and sandbox policy.
- **Unsafe, dependencies, and security:** Metadata origin and authenticity, TLS
  roots, redirects, cache freshness, replay/downgrade, timeouts, proxies,
  rate limits, privacy leakage, and untrusted release text are in scope. No
  download, signature, replacement, or rollback guarantee is inferred.
- **Replaceable boundary:** `UpdateDiscovery` receives current version/channel
  and returns bounded provenance-tagged metadata plus cache status; UI
  notification is a separate policy.
- **Validating PoC:** Against a local fixture service, test trusted and tampered
  metadata, stale-cache and replayed responses, downgrade/channel confusion,
  redirects, TLS failure, timeout, offline startup, rate limits, proxy behavior,
  response caps, escaped release text, and whether requests reveal unnecessary
  OS, architecture, installation, or user identifiers. Do not run executable
  replacement or rollback tests against this informer.

### `self_update` 1.0.0-rc.6 metadata-query path

- **Function and evidence:** Its release backends can query release metadata
  before any download/install operation; this row evaluates only that query
  path from the versioned
  [API](https://docs.rs/self_update/1.0.0-rc.6/self_update/).
- **Release, maintenance, and license:** Release candidate 1.0.0-rc.6 was
  published 2026-07-16 with SPDX `MIT` and Rust 1.88 minimum in official
  [package metadata](https://crates.io/api/v1/crates/self_update/1.0.0-rc.6).
  It is not a final 1.0 release.
- **Targets:** Query behavior is macOS/Linux-capable subject to selected
  provider and HTTP/TLS features; installation-specific OS behavior is excluded
  from this row.
- **Unsafe, dependencies, and security:** The same metadata trust, caching,
  replay, timeout, redirect, proxy, privacy, and untrusted-text risks apply.
  The adapter must make it impossible for a discovery call to trigger download
  or replacement as a side effect.
- **Replaceable boundary:** The same query-only `UpdateDiscovery` contract as
  above; installer types and methods cannot cross it.
- **Validating PoC:** Run the identical metadata fixture matrix as
  `update-informer`, with network/download instrumentation proving that the
  adapter performs query-only behavior. Compare cache controls, provider
  coupling, request disclosure, and error fidelity.

**Screening result:** Two query-capable paths are available for a discovery
PoC, although one comes from a broader installer crate. This comparison makes
no notification or update-source choice.

## 12B. Automatic update installation

### `self_update` 1.0.0-rc.6 install path

- **Function and evidence:** Downloads an identified release and uses its
  replacement facilities to install it; see the same pinned
  [versioned API](https://docs.rs/self_update/1.0.0-rc.6/self_update/). Query
  capability does not itself establish safe installation.
- **Release, maintenance, and license:** The pinned release/license evidence is
  the same as above: 1.0.0-rc.6, published 2026-07-16, SPDX `MIT`, Rust 1.88
  minimum, and still a release candidate in
  [official metadata](https://crates.io/api/v1/crates/self_update/1.0.0-rc.6).
- **Targets:** macOS and Linux are in scope, but executable replacement differs
  across application bundles, read-only/package-managed locations, permissions,
  code signing/notarization, filesystem boundaries, and running-image behavior.
- **Unsafe, dependencies, and security:** Artifact authenticity, signature and
  checksum policy, downgrade, archive traversal, decompression limits, partial
  writes, symlinks, permissions, atomicity, crash consistency, rollback,
  package ownership, consent, and secret-safe logging are high-risk. The API is
  not treated as evidence of a cryptographic update framework.
- **Replaceable boundary:** `UpdateInstaller` accepts already verified pinned
  artifact metadata and a disposable install target, returns a journaled result,
  and is unavailable for package-manager-owned installations.
- **Validating PoC:** Use a local fixture server and disposable install roots for
  valid, tampered, truncated, redirected, replayed, downgraded, wrong-platform,
  path-traversal, symlink, disk-full, permission, kill-at-every-write, and
  concurrent-launch cases. Verify signature/checksum policy, atomic replacement,
  rollback/crash recovery, package ownership, explicit consent, channel pinning,
  and no credential logging on both OSes.

**Supported-candidate gap:** `self_update` is the only automatic installer that
passed this desk screen. `update-informer` explicitly stops at discovery and
notification, so replacement and rollback tests are inapplicable and it is not
counted as installer candidate two. OS package managers are alternative
delivery ownership models, not embeddable automatic-update libraries. Before an
adoption ADR, either add a second supportable installer/update-framework PoC or
explicitly justify a single-candidate gap and why package-manager-only delivery
is insufficient.

**PoC/drop gate:** Drop the install path if authenticity cannot be enforced
before replacement, atomic rollback cannot be demonstrated under injected
failure, ownership/signing rules cannot be preserved, or a sustained security
fork is required. No installer or update architecture is selected.

## 13. Plugin sandbox / WebAssembly runtime

### `wasmtime` 47.0.3

- **Function and evidence:** Embeddable WebAssembly engine with resource controls,
  WASI, component-model support, and documented stability/platform tiers in the
  versioned [API](https://docs.rs/wasmtime/47.0.3/wasmtime/) and
  [manual](https://docs.wasmtime.dev/).
- **Release, maintenance, and license:** Version 47.0.3 was published
  2026-07-31 with SPDX `Apache-2.0 WITH LLVM-exception` and Rust 1.94 minimum in
  [package metadata](https://crates.io/api/v1/crates/wasmtime/47.0.3).
- **Targets:** Official platform tiers include macOS/Linux architectures; exact
  JIT/interpreter availability and deployment requirements must be pinned.
- **Unsafe, dependencies, and security:** The official
  [security model](https://docs.wasmtime.dev/security.html) describes sandboxing
  and defense in depth while acknowledging implementation bugs and platform
  tradeoffs. Host imports define authority. Fuel/epoch interruption, memory,
  filesystem, network, terminal output, JIT, cache, and transitive native code
  require strict configuration and advisory response. Two advisories published
  2026-07-31 identify 47.0.3 as the patched release for type-index confusion
  ([GHSA-hgjw-h833-99q9](https://github.com/bytecodealliance/wasmtime/security/advisories/GHSA-hgjw-h833-99q9))
  and preemption/trap state corruption
  ([GHSA-2hw9-mc66-jc2q](https://github.com/bytecodealliance/wasmtime/security/advisories/GHSA-2hw9-mc66-jc2q)).
  This makes an exact patch pin material; it does not prove the absence of other
  vulnerabilities.
- **Replaceable boundary:** `PluginRuntime` loads a versioned Noren interface and
  grants explicit capabilities; no ambient filesystem, network, environment,
  PTY, clipboard, SSH agent, or raw terminal-output authority.
- **Validating PoC:** Run benign and adversarial guests for infinite loops,
  memory growth, trap storms, oversized output, invalid modules, forbidden WASI,
  capability confusion, cache tampering, startup/steady-state cost, cancellation,
  and terminal escape filtering.

### `wasmer` 7.2.1

- **Function and evidence:** Embeddable WebAssembly runtime with engine/compiler
  choices and WASI APIs in the versioned
  [`wasmer`](https://docs.rs/wasmer/7.2.1/wasmer/) documentation and
  [upstream source](https://github.com/wasmerio/wasmer/tree/v7.2.1).
- **Release, maintenance, and license:** Version 7.2.1 was published
  2026-07-27 with SPDX `MIT` and Rust 1.93 minimum in
  [package metadata](https://crates.io/api/v1/crates/wasmer/7.2.1).
- **Targets:** Upstream supports macOS/Linux; exact architecture/compiler tiers
  and deployment footprint need a pinned test.
- **Unsafe, dependencies, and security:** Same host-capability, resource,
  terminal-output, JIT/native-code, cache, WASI, and advisory risks apply. A
  2024 upstream advisory records a filesystem sandbox symlink bypass for Wasmer
  versions through 4.3.1
  ([GHSA-55f3-3qvg-8pv5](https://github.com/wasmerio/wasmer/security/advisories/GHSA-55f3-3qvg-8pv5));
  compared version 7.2.1 is outside that stated range, which is not a complete
  security-history conclusion. A full security/stability review remains a gate.
- **Replaceable boundary:** The same versioned, capability-denying
  `PluginRuntime` seam, with runtime-specific handles confined to the adapter.
- **Validating PoC:** Run the identical malicious/benign guest suite and compare
  isolation, interruption reliability, memory/CPU caps, cold start, steady-state
  latency, binary size, dependency/unsafe inventory, and platform coverage.

**Screening result:** Two current runtimes are viable PoCs. This report does not
approve a plugin system, WASI access, an ABI, or third-party code execution.

## 14. Accessibility

### AccessKit 0.24.1 + `accesskit_winit` 0.33.2

- **Function and evidence:** Cross-platform accessibility tree/actions and a
  winit adapter. The official
  [architecture README](https://github.com/AccessKit/accesskit/blob/main/README.md)
  documents macOS NSAccessibility and Unix AT-SPI adapters, incremental tree
  updates, and current rich-text/hypertext limitations; versioned APIs are
  [`accesskit`](https://docs.rs/accesskit/0.24.1/accesskit/) and
  [`accesskit_winit`](https://docs.rs/accesskit_winit/0.33.2/accesskit_winit/).
- **Release, maintenance, and license:** `accesskit` 0.24.1 was published
  2026-06-12 with SPDX `MIT OR Apache-2.0`; `accesskit_winit` 0.33.2 was
  published 2026-07-14 with SPDX `Apache-2.0`, in official
  [core metadata](https://crates.io/api/v1/crates/accesskit/0.24.1) and
  [adapter metadata](https://crates.io/api/v1/crates/accesskit_winit/0.33.2).
  The README also identifies BSD-licensed Chromium-derived portions.
- **Targets:** Official adapters cover macOS and Unix AT-SPI, including the
  current Noren targets.
- **Unsafe, dependencies, and security:** Platform adapters add Objective-C and
  D-Bus/platform dependencies. A terminal accessibility tree can leak protected
  input or consume unbounded memory if it mirrors all scrollback. Stable node
  identity, update ordering, actions, focus, and redaction are Noren policy.
- **Replaceable boundary:** `AccessibilityBridge` receives a bounded semantic
  snapshot/update stream independent of renderer pixels and returns typed user
  actions.
- **Validating PoC:** Expose panes, title, focused cell, selection, visible grid,
  and a bounded scrollback window. Test VoiceOver and a Linux AT-SPI screen
  reader for reading order, navigation, selection, focus, rapid updates, resize,
  IME preedit, protected input, memory, and latency.

### GTK4 accessibility through `gtk4` 0.11.4

- **Function and evidence:** GTK widgets implement the official
  [`GtkAccessible`](https://docs.gtk.org/gtk4/iface.Accessible.html) interface
  and accessibility properties; Rust access is through
  [`gtk4`](https://docs.rs/gtk4/0.11.4/gtk4/).
- **Release, maintenance, and license:** Rust binding 0.11.4 was published
  2026-06-29 with SPDX `MIT`
  ([metadata](https://crates.io/api/v1/crates/gtk4/0.11.4)); native GTK uses
  [`LGPL-2.1-or-later`](https://gitlab.gnome.org/GNOME/gtk/-/blob/main/meson.build#L12).
- **Targets:** Linux accessibility is a primary GTK path; GTK can be built on
  macOS, but parity with native AppKit accessibility and packaging is not
  assumed.
- **Unsafe, dependencies, and security:** GObject/C FFI, native toolkit and
  accessibility-service dependencies, widget ownership, main-thread rules, tree
  size, and protected content need audit. A custom GPU-rendered terminal is not
  assumed to obtain correct text semantics automatically.
- **Replaceable boundary:** A GTK-specific `AccessibilityBridge` must consume the
  same bounded semantic snapshot and action model as AccessKit.
- **Validating PoC:** Run the identical VoiceOver/AT-SPI behavior suite in a GTK
  window; compare semantic coverage, custom-render integration, update cost,
  focus/actions, native quality on macOS, binary/dependency size, and packaging.

**Screening result:** Two framework paths are documented, but neither is proven
for terminal-scale dynamic text. Accessibility is a functional requirement and
release gate, not an optional polish layer.

## Per-candidate evidence-dimension ledger

This ledger makes Issue #3's evidence dimensions explicit for every unique
candidate or candidate stack above. A package reused in more than one category
has one entry with category-specific replacement costs. “No policy found” means
the versioned API/readme/repository material cited here was searched on
2026-08-03; it is not proof that no statement exists elsewhere. “Production
use” requires a project-owned source or an explicit evidence gap; stars and
download counts do not qualify.

Replacement-cost labels are screening estimates, not architecture decisions:
low means a narrow adapter plus corpus, medium includes material platform or
workflow integration, and high includes persistent state, native lifecycle, a
security protocol, or a user-visible compatibility contract. “No current
patch” means the published API appears sufficient to start the stated PoC, not
that implementation has already proved patch-free.

For Cargo packages, “direct dependency records” comes from the exact-version
crates.io `/dependencies` endpoint and counts target-specific records, which can
repeat a package across targets. Default features come from exact-version
package metadata. These fields deliberately do not substitute for the required
feature-resolved lockfile, recursive license/advisory tree, build-script review,
or unsafe inventory.

### `vte` 0.15.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc and the 0.15.0
  source are cited above; no compatibility/support policy stronger than the 0.x
  release line was found.
- **Primary-source production usage:** Alacritty's own
  [`alacritty_terminal` manifest at `v0.17.0`](https://github.com/alacritty/alacritty/blob/v0.17.0/alacritty_terminal/Cargo.toml)
  pins `vte` 0.15.0, providing immutable first-party integration evidence.
- **Fork/patch and replacement cost:** No current patch is required for the
  parser PoC. Estimated replacement cost is low because normalized actions and
  the shared byte corpus stay behind `VtParser`; behavior-difference triage is
  still required.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/alacritty/vte/security)
  was searched; no complete vulnerability-history claim is made.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/vte/0.15.0/dependencies)
  has 6 normal direct records, 4 optional; default features are `std`.

### `vtparse` 0.7.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  compatibility/support policy stronger than its 0.x line was found.
- **Primary-source production usage:** WezTerm's pinned
  [`wezterm-escape-parser` manifest](https://github.com/wezterm/wezterm/blob/fa0a1da0f93f/wezterm-escape-parser/Cargo.toml)
  depends on `vtparse`, providing first-party integration evidence for the
  parser in WezTerm's source tree.
- **Fork/patch and replacement cost:** No current patch is required for the
  parser PoC. Estimated replacement cost is low behind `VtParser`, subject to
  normalizing action vocabularies without erasing unsupported-sequence evidence.
- **Security-policy/advisory search (2026-08-03):** WezTerm's upstream
  [Security overview and advisory surface](https://github.com/wezterm/wezterm/security)
  was searched; monorepo advisories are not assumed to identify every
  crate-specific issue.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/vtparse/0.7.0/dependencies)
  has 2 normal direct records, 1 optional; default features are `std`.

### `avt` 0.18.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc and the tagged
  source are cited above; no support window or stronger 0.x compatibility policy
  was found.
- **Primary-source production usage:** The tagged upstream
  [README](https://github.com/asciinema/avt/blob/v0.18.0/README.md) states that
  asciinema CLI, player, server, and GIF generator use `avt`. This is an
  upstream production-use statement; independent consumer pin evidence was not
  collected.
- **Fork/patch and replacement cost:** No current patch is required to begin the
  state PoC. Estimated replacement cost is high because grid, modes, reflow,
  replies, damage, and snapshot semantics form persistent compatibility state.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/asciinema/avt/security)
  was searched; absence of a listed advisory is not an audit result.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/avt/0.18.0/dependencies)
  has 2 non-optional normal direct records and no declared default feature set.

### `portable-pty` 0.9.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  independent support window or stronger 0.x compatibility policy was found for
  the crate inside the WezTerm monorepo.
- **Primary-source production usage:** WezTerm's pinned
  [`wezterm-gui` manifest](https://github.com/wezterm/wezterm/blob/fa0a1da0f93f/wezterm-gui/Cargo.toml)
  depends on `portable-pty`; this is first-party product integration evidence.
- **Fork/patch and replacement cost:** No current patch is required to start the
  PoC. Estimated replacement cost is medium because spawn, descriptor, resize,
  child-lifetime, and platform error semantics must be revalidated.
- **Security-policy/advisory search (2026-08-03):** WezTerm's
  [Security overview and advisory surface](https://github.com/wezterm/wezterm/security)
  was searched; a crate-specific security policy was not identified.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/portable-pty/0.9.0/dependencies)
  has 15 normal direct records, 2 optional, and no declared default feature set;
  Unix/native dependencies include `libc`, `nix`, and `filedescriptor`.

### `nix` 0.31.3 PTY evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc documents the
  unsafe `forkpty` contract; no stronger compatibility/support promise was found
  for the 0.x line.
- **Primary-source production usage:** No consumer-owned production source for
  the PTY APIs was collected; this remains an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for the PoC.
  Estimated replacement cost is medium-high because Noren would own fork/exec,
  descriptor cleanup, signals, and platform error mapping directly.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/nix-rust/nix/security)
  was searched; a complete historical advisory mapping was not produced.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/nix/0.31.3/dependencies)
  has 5 normal direct records, 2 optional, and no defaults; the PoC must enable
  only the documented `term` and `process` feature set needed by its calls.

### `wgpu` 30.0.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc and upstream backend
  documentation are cited above; no cross-major API compatibility promise was
  found. The observed parallel major lines require an explicit pin.
- **Primary-source production usage:** The upstream
  [README](https://github.com/gfx-rs/wgpu/blob/trunk/README.md) states that
  `wgpu` is the core of WebGPU integrations in Firefox, Servo, and Deno. No
  Noren-like terminal consumer pin was collected.
- **Fork/patch and replacement cost:** No current patch is required to start the
  renderer PoC. Estimated replacement cost is high because shader/resource
  models, recovery, surface negotiation, and performance tuning differ across
  graphics APIs even behind `RenderBackend`.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/gfx-rs/wgpu/security)
  was searched; GPU-driver and transitive backend advisories remain separate
  lockfile/platform work.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/wgpu/30.0.0/dependencies)
  has 28 normal direct records, 10 optional. Defaults enable `std`,
  `parking_lot`, DX12, Metal, GLES, Vulkan, WGSL, and WebGPU; a Noren PoC must
  record and minimize the actually enabled target features.

### `glow` 0.18.0 + `glutin` 0.32.3 evidence dimensions

- **Documented API/compatibility policy:** Both versioned APIs are cited above;
  no stronger compatibility policy was found for either 0.x line. `glutin`'s
  API documents context/surface scope, not window/event ownership.
- **Primary-source production usage:** Alacritty's own
  [application manifest at `v0.17.0`](https://github.com/alacritty/alacritty/blob/v0.17.0/alacritty/Cargo.toml)
  pins `glutin` and its platform features. No consumer-owned production pin for
  `glow` 0.18.0 was collected, so that component has an explicit usage gap.
- **Fork/patch and replacement cost:** No current patch is required to start the
  PoC. Estimated renderer replacement cost is high; replacing only the context
  bridge is medium. GL object lifetime and context recovery remain adapter-owned.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [`glow` Security surface](https://github.com/grovesNL/glow/security) and
  [`glutin` Security surface](https://github.com/rust-windowing/glutin/security)
  were searched; native GL driver history is not represented by those pages.
- **Direct/default-feature dependency surface:** `glow` exact
  [metadata](https://crates.io/api/v1/crates/glow/0.18.0/dependencies) has 8
  normal direct records, 1 optional, and no defaults. `glutin` exact
  [metadata](https://crates.io/api/v1/crates/glutin/0.32.3/dependencies) has 18
  normal direct records, 8 optional; defaults enable EGL, GLX, X11, Wayland, and
  WGL. The controlled PoC additionally pins `winit` and `glutin-winit`.

### `winit` 0.30.13 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  support window or stronger 0.x compatibility policy was found. The beta 0.31
  line is not treated as compatible evidence.
- **Primary-source production usage:** Alacritty's own
  [application manifest at `v0.17.0`](https://github.com/alacritty/alacritty/blob/v0.17.0/alacritty/Cargo.toml)
  pins the 0.30 line and target features, providing first-party terminal
  integration evidence.
- **Fork/patch and replacement cost:** No current patch is required to start the
  window and IME PoCs. Estimated replacement cost is high for window lifecycle
  and medium-high for IME because event order, thread ownership, surface handles,
  and platform composition must all be remapped.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/rust-windowing/winit/security)
  was searched; platform-framework advisories remain separate evidence.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/winit/0.30.13/dependencies)
  has 47 target-specific normal direct records, 16 optional. Defaults enable
  raw-window-handle 0.6, X11, Wayland, dynamic Wayland loading, and the Wayland
  CSD path; the PoC must capture target-pruned resolution.

### `harfrust` 0.12.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc and tagged source
  are cited above; no compatibility/support policy stronger than the 0.x line
  was found.
- **Primary-source production usage:** No consumer-owned production dependency
  or deployment source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for the
  shaping PoC. Estimated replacement cost is medium-high because cluster maps,
  fallback, script/language features, and malformed-font behavior flow into
  selection and rendering.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/harfbuzz/harfrust/security)
  was searched; malformed-font safety still requires corpus/fuzz evidence.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/harfrust/0.12.0/dependencies)
  has 5 normal direct records, 1 optional; default features are `std`.

### `swash` 0.2.10 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  support window or stronger 0.x compatibility policy was found.
- **Primary-source production usage:** No consumer-owned production dependency
  or deployment source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required to start the
  PoC. Estimated replacement cost is medium-high because its shaping, scaling,
  rendering, cache, and cluster behavior must be separated behind the font seam.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/dfrg/swash/security)
  was searched; a complete malformed-font/advisory history was not established.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/swash/0.2.10/dependencies)
  has 4 normal direct records, 3 optional. Defaults enable `std`, `scale`, and
  `render`, so the PoC must state whether rasterization is in or out of scope.

### `unicode-width` 0.2.2 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  Unicode-data support window or compatibility promise beyond the released API
  was found.
- **Primary-source production usage:** Alacritty's
  [`alacritty_terminal` manifest at `v0.17.0`](https://github.com/alacritty/alacritty/blob/v0.17.0/alacritty_terminal/Cargo.toml)
  declares `unicode-width` 0.2, providing first-party terminal integration
  evidence, though not an exact 0.2.2 lockfile pin.
- **Fork/patch and replacement cost:** No current patch is required for the
  width PoC. Estimated replacement cost is low at `CellWidth`, but behavioral
  migration is user-visible and requires replaying saved grids and selections.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/unicode-rs/unicode-width/security)
  was searched; Unicode-version drift is a correctness/security concern even
  without a memory-safety advisory.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/unicode-width/0.2.2/dependencies)
  has 2 optional normal direct records and no non-optional direct crate; default
  features enable `cjk`, which must be an explicit policy choice in the PoC.

### `unicode-display-width` 0.3.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; the
  documented Unicode 15.1 data and provisional status provide no current-
  Unicode compatibility guarantee.
- **Primary-source production usage:** No consumer-owned production dependency
  or deployment source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for a
  provisional PoC. Estimated replacement cost is low at `CellWidth`, but the
  older data baseline is a drop risk rather than a reason to patch silently.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/jameslanska/unicode-display-width/security)
  was searched; no complete advisory claim is made.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/unicode-display-width/0.3.0/dependencies)
  has 1 non-optional normal direct record (`unicode-segmentation`) and no
  declared default feature set.

### `gtk4` 0.11.4 plus native GTK4 evidence dimensions

- **Documented API/compatibility policy:** Rust and native APIs are cited above.
  No binding-level support window or compatibility promise stronger than the
  Rust crate's 0.x line was found; native version features must be pinned.
- **Primary-source production usage:** Ghostty's official
  [architecture overview](https://ghostty.org/docs/about) documents its GTK4
  Linux frontend, providing terminal-product usage evidence for GTK4. It does
  not prove Noren's macOS parity or custom-render integration.
- **Fork/patch and replacement cost:** No current patch is required for a
  GTK-owned-window PoC. Estimated replacement cost is high because window/main-
  loop ownership, IME, accessibility, native packaging, and widget semantics
  are coupled; an IM-context-only fork is not assumed viable.
- **Security-policy/advisory search (2026-08-03):** The binding
  [Security surface](https://github.com/gtk-rs/gtk4-rs/security) and the official
  [GNOME security site](https://security.gnome.org/) were searched; native
  distro patch status remains platform evidence.
- **Direct/default-feature dependency surface:** Exact Rust
  [dependency metadata](https://crates.io/api/v1/crates/gtk4/0.11.4/dependencies)
  has 13 non-optional normal direct records and no defaults. It requires native
  GTK/GDK/GSK/Pango/GLib libraries; exact direct shared libraries and distro
  versions remain an explicit `pkg-config`/`otool`/package-manifest PoC gap.

### `russh` 0.62.5 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  support window or stronger 0.x compatibility guarantee was found.
- **Primary-source production usage:** The upstream
  [README's users section](https://github.com/warp-tech/russh#projects-using-russh)
  names client/server consumers, but no consumer-owned exact 0.62.5 production
  pin was collected; exact-version production use remains a gap.
- **Fork/patch and replacement cost:** No current patch is required to start the
  SSH fixture PoC. Estimated replacement cost is high because crypto/provider,
  host-key, authentication, rekey, flow-control, and secret-lifecycle behavior
  must remain equivalent at `SshTransport`.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [published-advisory surface](https://github.com/warp-tech/russh/security/advisories)
  and repository security overview were searched; the July 2026 advisories and
  patched versions are recorded in the candidate row above.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/russh/0.62.5/dependencies)
  has 70 normal direct records, 9 optional. Defaults enable `flate2`,
  `aws-lc-rs`, and `rsa`, making the crypto/provider and native-build selection
  an explicit lockfile/feature gate.

### `ssh2` 0.9.6 plus libssh2 evidence dimensions

- **Documented API/compatibility policy:** Versioned Rust API and native libssh2
  site are cited above; no stronger compatibility/support policy was found for
  the Rust 0.x wrapper. Native API stability does not establish wrapper safety.
- **Primary-source production usage:** No consumer-owned exact-version
  production dependency source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required to start the
  fixture PoC. Estimated replacement cost is high because transport semantics
  and native crypto/build provenance cross the FFI boundary.
- **Security-policy/advisory search (2026-08-03):** The wrapper's
  [Security surface](https://github.com/alexcrichton/ssh2-rs/security), libssh2's
  [Security surface](https://github.com/libssh2/libssh2/security), and official
  CVE pages were searched; the historical native CVE example is recorded above.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/ssh2/0.9.6/dependencies)
  has 4 non-optional normal direct records and no defaults, including
  `libssh2-sys`. The resolved native libssh2/TLS/crypto libraries are an
  additional direct platform surface that the PoC must print and attest.

### OpenSSH 10.4p1 delegation evidence dimensions

- **Documented API/compatibility policy:** Versioned release notes and current
  manuals are cited above. They document CLI/config behavior, but no promise was
  found that vendor-patched 9.x and upstream 10.4 produce identical `ssh -G`
  output or side effects.
- **Primary-source production usage:** OpenSSH is itself the shipped client
  implementation under test; no Noren-like consumer-owned delegation source was
  collected, so integration-pattern production evidence remains a gap.
- **Fork/patch and replacement cost:** No OpenSSH fork or patch is proposed.
  Estimated replacement cost is medium-high because subprocess, vendor version,
  config provenance, redaction, side-effect policy, and differential fixtures
  must be preserved when changing resolver strategy.
- **Security-policy/advisory search (2026-08-03):** Official
  [security advisories](https://www.openssh.com/security.html) and
  [release notes](https://www.openssh.com/releasenotes.html) were searched; the
  installed vendor's notices remain platform-specific evidence.
- **Direct/default-feature dependency surface:** This is a system executable,
  not a Cargo package, and has no Cargo defaults. The pinned portable
  [`INSTALL`](https://github.com/openssh/openssh-portable/blob/V_10_4_P1/INSTALL)
  requires a C toolchain and enumerates optional zlib, libcrypto providers, PAM,
  libedit, LDNS, BSM, and libfido2/configure surfaces. Exact system binary path,
  vendor patches, enabled options, linked libraries, config search paths, helper
  programs, and sandbox permissions remain a per-platform inventory gap.

### `ssh2-config` 0.7.2 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc documents its
  supported fields and omissions; no stronger 0.x compatibility/support policy
  was found.
- **Primary-source production usage:** No consumer-owned production dependency
  or deployment source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for the
  differential PoC. Estimated replacement cost is medium because parsed models
  are confined to `SshConfigResolver`, while fidelity diagnostics and provenance
  fixtures must survive a switch.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/veeso/ssh2-config/security)
  was searched; parser fidelity/security is not inferred from no listed result.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/ssh2-config/0.7.2/dependencies)
  has 6 non-optional normal direct records and no declared defaults.

### `interprocess` 2.4.3 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  compatibility/support policy stronger than the released API was found.
- **Primary-source production usage:** No consumer-owned production dependency
  or deployment source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for the IPC
  PoC. Estimated replacement cost is medium because naming, permissions,
  framing, peer identity, cancellation, and platform cleanup are adapter-owned.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/kotauskas/interprocess/security)
  was searched; OS IPC advisories and permissions require separate platform work.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/interprocess/2.4.3/dependencies)
  has 8 normal direct records, 3 optional, and no defaults. The Tokio comparison
  requires enabling the optional `tokio`/`async` path explicitly.

### Tokio 1.53.1 Unix-socket evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; the
  1.x release line and MSRV are documented upstream, but no frozen
  platform-event behavior promise was found for this PoC's socket/lifecycle use.
- **Primary-source production usage:** No consumer-owned production source for
  this exact local-socket configuration was collected; this is an explicit gap.
- **Fork/patch and replacement cost:** No current patch is required for the IPC
  PoC. Estimated replacement cost is medium because the runtime, cancellation,
  backpressure, task ownership, and shutdown model extend beyond socket calls.
- **Security-policy/advisory search (2026-08-03):** Tokio's upstream
  [Security overview and advisory surface](https://github.com/tokio-rs/tokio/security)
  was searched; runtime and transitive I/O advisories remain lockfile gates.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/tokio/1.53.1/dependencies)
  has 16 normal direct records, 15 optional, and no defaults. The PoC must name
  only its required `net`, runtime, synchronization, and time features rather
  than use the broad `full` feature.

### `toml` 1.1.4+spec-1.1.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc and the encoded TOML
  spec version are cited above; no additional support-window policy was found.
- **Primary-source production usage:** Cargo default-branch commit
  [`5727d3b9bd87`](https://github.com/rust-lang/cargo/blob/5727d3b9bd873a4e05fc3ee944da1b7d503947a3/Cargo.toml),
  dated 2026-08-02, pins the 1.1 line and enables parse/display/Serde features,
  providing
  first-party production-tool integration evidence.
- **Fork/patch and replacement cost:** No current patch is required for the
  parse/serialize PoC. Estimated replacement cost is low-medium because typed
  config can stay at Noren's seam, but formatting/comment preservation may force
  a separate editor representation.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/toml-rs/toml/security)
  was searched; parser resource limits still require hostile-input tests.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/toml/1.1.4%2Bspec-1.1.0/dependencies)
  has 10 normal direct records, 8 optional. Defaults enable `std`, `serde`,
  `parse`, and `display`.

### `taplo` 0.14.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  stronger 0.x compatibility/support policy was found.
- **Primary-source production usage:** The upstream
  [Taplo repository](https://github.com/tamasfe/taplo) contains the library,
  CLI, and language-server implementation, providing first-party product
  integration evidence; an independent consumer pin was not collected.
- **Fork/patch and replacement cost:** No current patch is required for the
  editing PoC. Estimated replacement cost is medium because syntax-tree,
  recovery, diagnostics, spans, and formatting-preservation behavior must be
  normalized.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/tamasfe/taplo/security)
  was searched; malformed-input and glob/schema behavior remain PoC gates.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/taplo/0.14.0/dependencies)
  has 14 normal direct records, 2 optional; default features enable `serde`.

### `tracing` 0.1.44 + `tracing-subscriber` 0.3.23 evidence dimensions

- **Documented API/compatibility policy:** Both versioned APIs are cited above;
  no support window or stronger compatibility promise was found for these 0.x
  crates.
- **Primary-source production usage:** Cargo default-branch commit
  [`5727d3b9bd87`](https://github.com/rust-lang/cargo/blob/5727d3b9bd873a4e05fc3ee944da1b7d503947a3/Cargo.toml),
  dated 2026-08-02, pins these exact versions, providing first-party
  production-tool integration evidence.
- **Fork/patch and replacement cost:** No current patch is required for the
  logging PoC. Estimated replacement cost is medium because span/event schema,
  field redaction, filtering, reload, and sink behavior become operational
  contracts even though macros stay behind a logging facade.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/tokio-rs/tracing/security)
  was searched; formatter/filter and transitive sink advisories remain lockfile
  work.
- **Direct/default-feature dependency surface:** `tracing` exact
  [metadata](https://crates.io/api/v1/crates/tracing/0.1.44/dependencies) has 4
  normal direct records, 2 optional; defaults are `std` and `attributes`.
  `tracing-subscriber` exact
  [metadata](https://crates.io/api/v1/crates/tracing-subscriber/0.3.23/dependencies)
  has 18 normal records, 17 optional; defaults enable formatting, ANSI,
  `tracing-log`, `std`, and `smallvec`, not JSON or environment filtering.

### `log` 0.4.33 + `env_logger` 0.11.11 evidence dimensions

- **Documented API/compatibility policy:** Versioned APIs are cited above; no
  separate support-window policy was found for this exact pair.
- **Primary-source production usage:** No consumer-owned source pinning this
  exact pair in a production tool was collected; this is an explicit evidence
  gap.
- **Fork/patch and replacement cost:** No current patch is required for the
  logging PoC. Estimated replacement cost is low-medium behind the logging
  facade, with filter syntax, redaction, and output format retained as tests.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [`log` Security surface](https://github.com/rust-lang/log/security) and
  [`env_logger` Security surface](https://github.com/rust-cli/env_logger/security)
  were searched; no complete transitive history is claimed.
- **Direct/default-feature dependency surface:** `log` exact
  [metadata](https://crates.io/api/v1/crates/log/0.4.33/dependencies) has 4
  optional normal records and no defaults. `env_logger` exact
  [metadata](https://crates.io/api/v1/crates/env_logger/0.11.11/dependencies) has
  5 normal records, 3 optional; defaults enable auto-color, human time, and
  regex filtering.

### `color-eyre` 0.6.5 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  support window or stronger 0.x compatibility policy was found.
- **Primary-source production usage:** No consumer-owned exact-version
  production dependency source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for the panic
  PoC. Estimated replacement cost is low behind `PanicReporter`, but redaction,
  hook ordering, and output permissions must be preserved.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/eyre-rs/eyre/security)
  was searched; panic-report data exposure remains an application-owned risk.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/color-eyre/0.6.5/dependencies)
  has 8 normal direct records, 3 optional. Defaults enable caller tracking and
  spantrace capture, both of which affect sensitive report content and size.

### `human-panic` 2.0.8 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  separate compatibility/support window was found.
- **Primary-source production usage:** No consumer-owned exact-version
  production dependency source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for the panic
  PoC. Estimated replacement cost is low behind `PanicReporter`, subject to
  retaining redaction, permissions, and user-facing incident behavior.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/rust-cli/human-panic/security)
  was searched; report contents and system-information collection require local
  review.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/human-panic/2.0.8/dependencies)
  has 8 normal direct records, 2 optional; default features enable color.

### `crash-handler` 0.8.0 + `minidump-writer` 0.13.0 evidence dimensions

- **Documented API/compatibility policy:** Both versioned APIs are cited above;
  no cross-release compatibility/support policy was found for the stack or its
  dump schema integration.
- **Primary-source production usage:** `minidump-writer`'s upstream
  [`0.13.0` README](https://github.com/rust-minidump/minidump-writer/blob/a7139ad447e667bc7085e3bcdf06f57e14fc17c6/README.md)
  says it is usable with production caveats, but no consumer-owned exact stack
  deployment was collected; production-use evidence remains a gap.
- **Fork/patch and replacement cost:** No current patch is required to begin the
  controlled crash PoC. Estimated replacement cost is high because signal/
  exception setup, helper-process protocol, dump format, symbolization, and
  platform permissions are tightly integrated.
- **Security-policy/advisory search (2026-08-03):** The
  [`crash-handler` Security surface](https://github.com/EmbarkStudios/crash-handling/security)
  and [`minidump-writer` Security surface](https://github.com/rust-minidump/minidump-writer/security)
  were searched; dump-content exposure and async-safety still require source
  and runtime audit.
- **Direct/default-feature dependency surface:** `crash-handler` exact
  [metadata](https://crates.io/api/v1/crates/crash-handler/0.8.0/dependencies)
  has 5 non-optional normal records and no defaults. `minidump-writer` exact
  [metadata](https://crates.io/api/v1/crates/minidump-writer/0.13.0/dependencies)
  has 21 non-optional normal records and no defaults, including target-native
  process, Mach, `procfs`, and memory-map surfaces.

### Crashpad at `ad1827ddbc03` evidence dimensions

- **Documented API/compatibility policy:** The official
  [interface docs](https://crashpad.chromium.org/doxygen/) and pinned
  [status document](https://chromium.googlesource.com/crashpad/crashpad/+/ad1827ddbc03/doc/status.md)
  were reviewed; no stable embeddable-API or support-window policy was found for
  arbitrary third-party integration at this commit.
- **Primary-source production usage:** The Chromium-hosted project describes
  Crashpad as a crash-reporting system, but no consumer-owned exact-commit
  deployment source was collected; this remains an explicit evidence gap.
- **Fork/patch and replacement cost:** No source patch is proposed for the PoC;
  build/integration glue is still required. Estimated replacement cost is high
  because the C++ build, handler process, client protocol, database, dumps,
  symbolization, and platform exception registration are coupled.
- **Security-policy/advisory search (2026-08-03):** The official
  [Crashpad commit log](https://chromium.googlesource.com/crashpad/crashpad/+log/refs/heads/main),
  [issue tracker](https://crashpad.chromium.org/bug/), and
  [Chromium security program](https://www.chromium.org/Home/chromium-security/)
  were searched; no Crashpad-specific public advisory completeness claim is made.
- **Direct/default-feature dependency surface:** Crashpad is GN/C++, not Cargo,
  and has no Cargo defaults. Its pinned
  [`DEPS`](https://chromium.googlesource.com/crashpad/crashpad/+/ad1827ddbc03/DEPS)
  enumerates buildtools, mini_chromium, zlib, Linux syscall support, tests, and
  platform-conditional packages; the target-pruned linked/runtime inventory
  remains a PoC output.

### `cargo-dist` 0.32.0 evidence dimensions

- **Documented API/compatibility policy:** The versioned manual/source are cited
  above; no guarantee was found that generated workflow/schema/install output is
  compatible across all future releases.
- **Primary-source production usage:** The upstream
  [`v0.32.0` README](https://github.com/axodotdev/cargo-dist/blob/v0.32.0/README.md)
  documents that `cargo-dist` self-hosts its releases. This is first-party
  build-pipeline use, not independent consumer evidence.
- **Fork/patch and replacement cost:** No current patch is required for the
  packaging PoC. Estimated replacement cost is medium because generated CI,
  artifact manifests, signing hooks, installer behavior, and reproducibility
  assertions must be migrated; runtime code remains unaffected.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/axodotdev/cargo-dist/security)
  was searched; generated actions and toolchain provenance remain separate
  supply-chain evidence.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/cargo-dist/0.32.0/dependencies)
  has 41 non-optional normal direct records and no declared defaults. As a build
  tool, its executed helpers/actions and host tools must also be inventoried.

### `cargo-packager` 0.11.8 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc and upstream source
  are cited above; no stronger 0.x generated-output or compatibility policy was
  found.
- **Primary-source production usage:** The upstream repository includes
  packaging examples, but no consumer-owned exact-version production pipeline
  was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required to start the
  packaging PoC. Estimated replacement cost is medium because platform metadata,
  signing hooks, package layout, installer/uninstaller, and host tools change.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/crabnebula-dev/cargo-packager/security)
  was searched; pre-packaging commands and native tools remain explicit
  supply-chain review items.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/cargo-packager/0.11.8/dependencies)
  has 42 normal direct records, 4 optional. Defaults enable the CLI and Rustls
  TLS, in addition to native packaging tools invoked at runtime.

### `update-informer` 1.3.0 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; no
  registry support window or stronger compatibility policy was found.
- **Primary-source production usage:** The upstream
  [README's users list](https://github.com/mgrachev/update-informer#users) names
  applications, but no consumer-owned exact 1.3.0 production pin was collected;
  exact-version production use remains a gap.
- **Fork/patch and replacement cost:** No current patch is required for the
  metadata-only PoC. Estimated replacement cost is low behind `UpdateDiscovery`;
  it has no installer replacement/rollback cost because it performs neither.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/mgrachev/update-informer/security)
  was searched; metadata-provider, TLS, cache, replay, and privacy behavior still
  require fixture evidence.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/update-informer/1.3.0/dependencies)
  has 6 normal direct records, 2 optional. Defaults select crates.io, `ureq`, and
  Rustls TLS.

### `self_update` 1.0.0-rc.6 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc is cited above; as a
  release candidate it supplies no final-1.0 compatibility premise, and no
  separate support window was found.
- **Primary-source production usage:** No consumer-owned exact-rc.6 production
  dependency/deployment source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required to start the
  query-only or disposable-root installer PoC. Estimated replacement cost is low
  for query-only use and high for installation because verification, atomic
  replacement, rollback journal, signing, and package ownership must migrate.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/jaemk/self_update/security)
  was searched; the PoC must separately test archive, TLS/provider, signature,
  replay, and replacement failure behavior.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/self_update/1.0.0-rc.6/dependencies)
  has 26 normal direct records, 17 optional. Defaults enable reqwest, Rustls,
  progress UI, GitHub, tar archives, and gzip; provider/archive/TLS minimization
  must be recorded per PoC.

### `wasmtime` 47.0.3 evidence dimensions

- **Documented API/compatibility policy:** The official
  [stability document](https://docs.wasmtime.dev/stability.html) defines API,
  embedding, CLI, and WebAssembly compatibility scopes; the exact versioned
  rustdoc remains the compile target.
- **Primary-source production usage:** No consumer-owned exact 47.0.3 production
  embedding source was collected; this is an explicit evidence gap despite the
  project's broader deployment claims.
- **Fork/patch and replacement cost:** No current patch is required for the
  hostile-guest PoC. Estimated replacement cost is high because component/WASI
  ABI, host imports, resource controls, cache artifacts, interruption, and
  capability policy must migrate together.
- **Security-policy/advisory search (2026-08-03):** The official
  [security model](https://docs.wasmtime.dev/security.html) and upstream
  [published advisories](https://github.com/bytecodealliance/wasmtime/security/advisories)
  were searched; the two advisories patched by 47.0.3 are recorded above.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/wasmtime/47.0.3/dependencies)
  has 50 normal direct records, 35 optional. Defaults enable a broad compiler,
  runtime, async, cache, profiling, pooling, GC, component-model, threads,
  debugging, and WIT surface; a least-feature PoC is mandatory.

### `wasmer` 7.2.1 evidence dimensions

- **Documented API/compatibility policy:** Versioned rustdoc and tagged source
  are cited above; no support window or embedding-compatibility policy equivalent
  to Wasmtime's cited stability document was found.
- **Primary-source production usage:** No consumer-owned exact 7.2.1 production
  embedding source was collected; this is an explicit evidence gap.
- **Fork/patch and replacement cost:** No current patch is required for the
  hostile-guest PoC. Estimated replacement cost is high because compiler/engine,
  WASI, ABI, host imports, stores, caches, metering, and interruption differ.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [published-advisory surface](https://github.com/wasmerio/wasmer/security/advisories)
  was searched; the historical filesystem bypass and compared version range are
  recorded above.
- **Direct/default-feature dependency surface:** Exact
  [dependency metadata](https://crates.io/api/v1/crates/wasmer/7.2.1/dependencies)
  has 39 normal direct records, 14 optional; defaults enable `sys-default`, which
  pulls the native system/compiler path and must be decomposed in the lockfile.

### AccessKit 0.24.1 + `accesskit_winit` 0.33.2 evidence dimensions

- **Documented API/compatibility policy:** Versioned APIs and architecture
  limitations are cited above; no cross-version support window or compatibility
  promise was found for the 0.x crates.
- **Primary-source production usage:** Slint default-branch commit
  [`ff0cd0a52d84`](https://github.com/slint-ui/slint/blob/ff0cd0a52d841b17ea95c1b5cd9d5dc97f0fdc25/internal/backends/winit/Cargo.toml),
  dated 2026-08-02, pins `accesskit` 0.24 and `accesskit_winit` 0.33, providing
  first-party UI-product integration evidence. It does not prove terminal
  rich-text behavior.
- **Fork/patch and replacement cost:** No current patch is required for the
  accessibility PoC. Estimated replacement cost is high because stable node
  identity, incremental tree semantics, focus/actions, platform adapters,
  protected text, and assistive-technology behavior are user-visible contracts.
- **Security-policy/advisory search (2026-08-03):** The upstream
  [Security overview and advisory surface](https://github.com/AccessKit/accesskit/security)
  was searched; platform accessibility-service advisories and privacy behavior
  remain separate gates.
- **Direct/default-feature dependency surface:** Core exact
  [metadata](https://crates.io/api/v1/crates/accesskit/0.24.1/dependencies) has 6
  normal direct records, 5 optional, and no defaults. Adapter exact
  [metadata](https://crates.io/api/v1/crates/accesskit_winit/0.33.2/dependencies)
  has 9 normal records, 4 optional; defaults enable Unix, async-io,
  raw-window-handle 0.6, and winit X11/Wayland, coupling it to the same window
  implementation rather than supplying a window alternative.

## Cross-category conflicts and unknowns

- `vte` and `vtparse` are action parsers, not terminal-state alternatives.
  `avt` is the only released parser-plus-screen-state candidate retained by
  this screen, so a state-engine decision would require either a second PoC or
  an explicit gap justification.
- `wgpu` has a higher major published before a later patch on the preceding
  major; the PoC must pin an explicit line rather than use “newest” ambiguously.
- `glutin`/`glutin-winit` supply GL context integration on top of `winit`; they
  do not make the OpenGL renderer PoC a second window/event implementation.
  Window/event integration therefore retains a one-candidate gap.
- The Unicode reference baseline is 17.0.0, while the provisional second width
  crate documents Unicode 15.1.0. No silent data-version substitution is valid.
- `winit` and GTK expose IME concepts through different window ownership models;
  no mixed-toolkit API is inferred.
- SSH transport libraries do not establish OpenSSH config fidelity. Config
  directives that can execute commands must remain inert during discovery and
  parsing unless a later approved, user-requested flow evaluates them with a
  visible side-effect policy. Structured argv protects only Noren's invocation;
  it does not shell-escape `%h`/`%n` when OpenSSH expands destination text into
  a configured `Match exec` command.
- Rust panic hooks and native crash capture solve different failure classes.
  Minidumps may contain secrets; collection, symbolication, consent, retention,
  and upload are separate decisions.
- Release packaging, update discovery/notification, and update installation are
  three separate capabilities. A package-manager install may forbid or make
  self-replacement unsafe, and an informer has no replacement/rollback behavior
  to test.
- A WebAssembly runtime is not itself a plugin permission model. Host imports,
  output handling, resource budgets, persistence, versioning, and revocation are
  Noren-owned if a plugin system is ever approved.
- Neither renderer nor window toolkit proves shaping, cell width, IME, or
  accessibility correctness. The corresponding PoCs must be composed only after
  their individual boundaries and measurements are understood.
- Minimum supported Rust version, exact target triples, native dependency
  versions, recursive licenses, `unsafe` counts, advisories, security-response
  history, and binary-size impact remain lockfile/source-audit outputs. They are
  not inferred from top-level crate metadata.

## Evidence-to-decision gate

No candidate may advance from this report directly to production. A later
comparison must attach reproducible PoC results, locked dependency and license
inventory, recursive unsafe review, advisory scan, platform evidence, failure
behavior, performance measurements, and independent source/security review.
Only then can an RFC/ADR compare tradeoffs against approved requirements. This
document deliberately leaves every adoption decision open.
