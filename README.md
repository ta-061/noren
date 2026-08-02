# Noren

> A Zellij-friendly terminal for local, remote, and agent workflows.

> **Project status: Discovery.** Noren does not yet contain a terminal
> application, installable binaries, or a Preview release. Everything described
> below is a product goal until the implementation and release evidence says
> otherwise.

Noren (ノレン) is being designed as a Rust workspace terminal for macOS and
Linux. Its name comes from the Japanese *noren*: a curtain that divides a space
without preventing passage. Noren aims to create useful boundaries between
local shells, SSH hosts, terminal multiplexers, and CLI coding agents without
taking over their input.

**Split your workspace, not your keybindings.**

## Product goals

- Preserve input expected by Zellij, tmux, Vim, Neovim, Emacs, shells, and
  terminal applications.
- Offer native workspaces, tabs, and panes to users who do not run a terminal
  multiplexer.
- Treat OpenSSH hosts and reconnectable remote sessions as workspace concepts.
- Make existing CLI agents such as Codex, Claude Code, and OpenCode easier to
  launch and monitor without embedding a separate LLM chat product.
- Provide readable light, dark, high-contrast, and color-vision-friendly themes.
- Keep themes, keybindings, layouts, notifications, and future extensions under
  user control.

Noren is not intended to replace Zellij. When Zellij is running, correct input
pass-through takes priority over Noren shortcuts.

## Current phase

The project is completing Discovery and requirements work before production
implementation. The required sequence is:

1. verify tools, upstream behavior, licenses, and project constraints;
2. calibrate the available AI contributors on the same bounded task;
3. collect independent proposals and cross-reviews;
4. integrate testable requirements, architecture, threat model, RFCs, and ADRs;
5. implement through Issues, isolated branches/worktrees, PR review, CI, and
   independent QA;
6. publish only after every Preview gate has current evidence.

See the [roadmap](ROADMAP.md), [project principles](docs/project-principles.md),
and [coordination status](docs/coordination/status.md) for the live state. The
status ledger—not this aspirational overview—is authoritative about completion.

## Contributing

The contributor workflow is still being established. Start with
[CONTRIBUTING.md](CONTRIBUTING.md) and do not begin production implementation
until the relevant requirement, RFC/ADR, and Issue exist.

## Security

Please do not report suspected vulnerabilities in public Issues. Follow
[SECURITY.md](SECURITY.md).

## License

Noren is intended to be dual-licensed under either
[Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
