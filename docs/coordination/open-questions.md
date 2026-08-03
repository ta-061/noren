# Open questions

## Human confirmation required

- Should `main` require pull requests and successful CI, block force pushes and
  deletion, and prefer squash merging? This changes repository access behavior
  and will not be applied without explicit confirmation.
- Which signing/notarization identity, if any, may be used for macOS Preview
  artifacts? No credential is assumed.
- Which public support/security contact should be published before Preview?

## Design required before later implementation

- Permanent terminal parser/state adoption after the PoC corpus.
- Permanent window, GPU renderer, font shaping, IME, and accessibility stack;
  Discovery found no second like-for-like window candidate.
- Linux PTY/window ownership and parity after the macOS PoC.
- OpenSSH subprocess/config integration versus an SSH library, including
  ProxyCommand/ProxyJump and host-key behavior.
- Whether a remote daemon is justified for Preview, and whether it needs a
  separate repository/release boundary.
- Workspace persistence format, crash consistency, and migrations.
- Key protocol negotiation and the exact semantics of pass-through escape.
- Trust boundary and permissions for agent hooks, OSC notifications, IPC,
  webhooks, and future plugins.

## Resolved only for the first macOS local-PTY PoC

- [ADR 0001](../adr/0001-rust-toolchain-and-msrv.md) pins Rust/MSRV 1.88.0,
  edition 2024, resolver 3, and the `aarch64-apple-darwin` target. The first
  implementation must still record installed versions/targets and compile.
- [ADR 0002](../adr/0002-local-pty-poc-architecture.md) defines three crates,
  PTY supervisor/reader ownership, bounded channels, and reversible candidate
  versions. It is not permanent library adoption.
- The PoC key contract covers printable UTF-8, Enter, Backspace, Tab, Escape,
  arrows, and Ctrl bytes. IME/dead keys, Cmd/Option policy, key negotiation, and
  pass-through escape remain closed to implementation.

## Later gates must verify

- Locked dependency features/licenses and unsafe inventory for any candidate
  used in implementation.
- Executable terminal/PTy/window/renderer behavior and the drop gates in the
  risk register; Discovery evidence alone is not adoption.
- Current upstream behavior again when a later Issue begins SSH, Zellij,
  agent-integration, packaging, or release work.
