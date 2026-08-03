# Open questions

## Human confirmation required

- Should `main` require pull requests and successful CI, block force pushes and
  deletion, and prefer squash merging? This changes repository access behavior
  and will not be applied without explicit confirmation.
- Which signing/notarization identity, if any, may be used for macOS Preview
  artifacts? No credential is assumed.
- Which public support/security contact should be published before Preview?

## Design council must decide

- Terminal parser/state library boundary and replacement strategy.
- Window, GPU renderer, font shaping, IME, and accessibility stack.
- PTY abstraction and platform-specific ownership.
- OpenSSH subprocess/config integration versus an SSH library, including
  ProxyCommand/ProxyJump and host-key behavior.
- Whether a remote daemon is justified for Preview, and whether it needs a
  separate repository/release boundary.
- Workspace persistence format, crash consistency, and migrations.
- Key protocol negotiation and the exact semantics of pass-through escape.
- Trust boundary and permissions for agent hooks, OSC notifications, IPC,
  webhooks, and future plugins.
- Preview MSRV and the reproducible Rust toolchain installation/pinning method.

## Discovery must verify

- Current upstream behavior and licenses for every library candidate.
- Current Zellij defaults and protocol behavior by supported version.
- Current official Codex, Claude Code, and OpenCode hook/plugin/structured-output
  interfaces.
- cmux feature behavior from lawful public sources without copying code, assets,
  or marks.

## Disposition mapping for architecture-changing unknowns

Each item above under "Design council must decide" is mapped to a bounded
experiment, a named RFC/design-council question, or an explicit deferral in
the [risk register](../roadmap/risk-register.md#architecture-changing-unknowns-and-dispositions)
(Issue [#8](https://github.com/ta-061/noren/issues/8)). The mappings do not
answer the questions. `Unknown` states persist until the referenced gates
pass; for example, remote PTY persistence stays `Unknown` until `REM-02`,
and agent semantic state stays `Unknown` until the version-scoped adapter
gates pass.
