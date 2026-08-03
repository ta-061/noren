# ADR 0002: Start with a replaceable local-PTY vertical slice

- Status: Accepted for PoC; candidate adoption remains provisional
- Date: 2026-08-03
- Decision owners: Codex integration for Issue #6; human owner approves by merge
- Related Issue/RFC: [#6](https://github.com/ta-061/noren/issues/6)

## Context

Noren needs executable evidence before finalizing every v0.1 subsystem. The
owner selected a first vertical slice: workspace/CI, one macOS window, direct
local zsh PTY, transparent supported input, visible output, resize propagation,
and orderly exit. SSH, agents, tabs, panes, themes, and a remote daemon are
explicitly excluded.

GLM proposed narrow PTY/terminal/render contracts and three initial crates.
Qwen proposed an app-owned winit event adapter, minimal cell rendering, exact
key mappings, and coalesced non-zero resize. Both treat libraries and numeric
budgets as unmeasured candidates.

## Decision drivers

- Produce a runnable end-to-end loop with the fewest product responsibilities.
- Keep process/PTY, terminal state, and platform UI independently testable and
  replaceable.
- Keep the main-thread rule explicit for AppKit/winit.
- Make input, resize, backpressure, and shutdown observable before convenience
  features.
- Avoid an async runtime or remote/security surface not needed by the slice.

## Options considered

1. One application crate: fastest initially, but couples native, parser, process,
   and renderer ownership and defeats independent PTY tests.
2. Three crates with Noren-owned seams: small extra wiring, clear ownership and
   replacement tests.
3. Design and implement the whole v0.1 architecture first: delays the evidence
   needed to make those later decisions and contradicts the owner cut line.

## Decision

Use one repository and three workspace crates:

- `noren-pty`: structured fixed-zsh spawn, master I/O, resize, status, teardown;
- `noren-terminal`: terminal bytes/state, bounded snapshots, cell-width policy;
- `noren-app`: winit main loop, app-owned input/lifecycle adapter, renderer, and
  bounded channel wiring.

The main thread owns window, renderer, and terminal state. A PTY supervisor owns
the master/writer/child and handles bounded commands; a cloned blocking reader
has one reader thread. The supervisor is the sole child reaper. Output travels
through a bounded channel; the main loop drains a bounded amount per callback.
Latest non-zero size is coalesced once per event-loop turn and sent only when
rows/columns change. Shutdown follows one idempotent state machine and joins both
workers before event-loop exit.

Trial these exact direct candidates behind the documented contracts:

- `portable-pty` 0.9.0;
- `avt` 0.18.0;
- `winit` 0.30.13;
- `wgpu` 30.0.0;
- `swash` 0.2.10;
- `unicode-width` 0.2.2;
- bounded Rust standard-library threads/channels; no async runtime.

Supported key encoding is deliberately finite: printable UTF-8 text, Enter,
Backspace, Tab, Escape, arrows, and Ctrl control bytes. Key releases emit
nothing. Cmd combinations, Option policy, IME/dead keys, key-protocol
negotiation, clipboard, and authority-bearing terminal side effects are not
implemented.

## Consequences

The slice can produce real measurements and separate PTY tests without waiting
for SSH/workspace/agent design. There is more lifecycle wiring than a monolith,
and blocking PTY I/O requires explicit close/unblock tests. The renderer/font
path is intentionally basic and may be replaced. Passing the slice authorizes
no Preview compatibility claim.

## Security and reliability impact

Structured spawn prevents command concatenation; PTY output remains
non-authoritative; bounded queues/snapshots constrain exhaustion; the dedicated
supervisor centralizes ownership and reaping. Native/dependency unsafe,
environment inheritance, parser/font hostility, renderer loss, and blocked
reader teardown still require the threat-model tests and Claude review.

## Validation evidence

The merged Discovery reports support only candidate status. The GLM and Qwen
proposals define complementary seams. Final adoption requires the implementation
tests in [the strategy](../testing/strategy.md), macOS measurements, codex-lab
PTY/resize review, and Claude process/input review.

## Reversal or replacement plan

Each third-party candidate can be replaced behind its Noren-owned contract using
the same corpus. If the two-thread PTY design cannot terminate deterministically,
stop the PoC and amend this ADR before adding polling/async dependencies or
lower-level `nix`. Reverting the implementation leaves documentation and no
persisted user format to migrate.

## Dissent and unresolved questions

Qwen proposed a 50 ms resize settle and 10 s automatic exit window. Integration
instead coalesces once per event-loop turn and leaves the final grid visible
until user close, avoiding hidden latency and time-based UI policy. Full
terminal semantics, second window candidate, IME/accessibility, Linux, font
fallback, persistence, SSH, agent state, and remote architecture remain open.
