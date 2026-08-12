# Noren

> A Zellij-friendly terminal for local, remote, and agent workflows.

> **Current state: a first workspace slice on a terminal foundation.** What
> exists on `main` is a macOS window over a local `zsh` PTY with a tested
> terminal state core, plus a drawn workspace sidebar, a `Super+p` command
> palette that creates/selects/closes local sessions, mouse reporting that
> reaches the program inside, and sidebar state that survives a restart.
>
> Per-cell SGR foreground and explicit background colours now reach drawing:
> ANSI and 256-colour values resolve through a fixed palette, while RGB passes
> through as direct truecolor, with the resolved colour carried per vertex. The
> default palette and theme are not user-configurable. The renderer still uses
> a case-insensitive 5x7 ASCII bitmap font, with all non-ASCII shown as `?`, no
> visible cursor, no IME, and no accessibility surface. Only local sessions can
> be launched. Configured SSH hosts are discovered, displayed in the sidebar,
> and selectable as pending targets without opening an SSH connection or PTY;
> git worktrees and agents remain modelled but unreachable. Keybindings are not
> configurable. There are no published binaries. Read
> **[docs/known-limitations.md](docs/known-limitations.md)** first — it is the
> accurate predictor of what happens when you run the build. Everything below
> the status block describes **intent**, not current capability.

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

Milestones 0–2 (discovery, requirements/design, terminal foundation) are
complete on the evidence recorded in [ROADMAP.md](ROADMAP.md). Milestone 3
(workspace) is **in progress**: the vertical slice — sidebar, palette, session
lifecycle, persistence, Zellij pass-through — has landed, while configurable
keybindings, SSH connection launching, and agent launching have not; the
reasoning is in
[Milestone 3 status](ROADMAP.md#milestone-3-status). The SSH,
agent-experience, theming/accessibility, quality, and preview milestones are
open. The required sequence remains:

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
