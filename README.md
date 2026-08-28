# Noren

> A Zellij-friendly terminal for local, remote, and agent workflows.

> **Current state: a first workspace slice on a terminal foundation.** What
> exists on `main` is a macOS window over a local `zsh` PTY with a tested
> terminal state core, plus a drawn workspace sidebar, a `Super+p` command
> palette over the session registry, mouse reporting that
> reaches the program inside, and sidebar state that survives a restart,
> project rows that launch directory-rooted sessions, and agent launching
> from configuration (`local_project_worktree_and_agent_kinds_are_launchable`
> in `crates/noren-app/src/session.rs`).
>
> Per-cell SGR foreground and explicit background colours now reach drawing:
> ANSI and 256-colour values resolve through a fixed palette, while RGB passes
> through as direct truecolor, with the resolved colour carried per vertex. The
> theme is user-selectable among three built-in palettes — `dark` (the
> default), `light`, and `high-contrast` — via `[theme] name` in `config.toml`
> (`theme.rs`, `docs/configuration.md` §`[theme]`; the selection reaching the
> renderer is pinned by `configured_theme_reaches_the_app_renderer_input`);
> text selections are visible by default through each palette's theme-owned
> inverse pair, over the exact copied cell range (including wrapped rows and
> both columns of a wide character);
> the built-in palettes themselves are fixed tables — no custom-palette or
> colour-vision-friendly configuration. The renderer still uses a
> hand-built 5x7 bitmap font with bounded coverage — distinct ASCII case plus
> the Latin-1 Supplement and Box Drawing blocks, a fixed replacement glyph for
> every other code point, so CJK text and emoji do not render — with no
> visible cursor, no IME, and no accessibility surface. The palette's session
> commands spawn real local sessions that switch, park, and close through the
> live view. A bounded, explicitly partial list of positive literal aliases
> from the user's OpenSSH config is displayed in the sidebar (at most
> `MAX_SSH_SIDEBAR_HOSTS` rows — 64, in `crates/noren-app/src/main.rs`, pinned
> by `many_ssh_hosts_are_bounded_and_report_the_omitted_count` in
> `crates/noren-app/src/main/tests.rs`),
> and selecting one launches the system `/usr/bin/ssh` client for that alias in
> the terminal's PTY (argv is exactly `ssh -- <alias>`; no credential is ever
> passed on the command line), replacing the current session, with launch,
> connect, and disconnect failures shown as visible states. Wildcard-only and dynamic OpenSSH destinations are not
> a complete browseable list. Parsed `HostName` and `User` values retain
> unresolved tokens such as `%h`, `%p`, and `%r` and are discovery metadata
> only; a future connection path must resolve or reject them first. `Include`
> handling is intentionally stricter than OpenSSH: Noren follows only files
> whose canonical targets remain under the top-level config directory;
> git worktrees of the launch repository are discovered and launchable (a
> sidebar click starts a shell whose working directory is that worktree), and
> so are `[[projects]]` rows (a selection starts a directory-rooted PTY
> session, `selecting_a_project_row_starts_a_session_in_that_directory`) and
> `[[agents]]` rows (a selection runs the configured command as a shell-free
> argv PTY, `selecting_an_agent_row_launches_the_configured_command_in_a_pty`)
> — both in `crates/noren-app/src/main/tests.rs`. Palette
> keybindings — the opener and the four command chords — are configurable
> through `[keys]` in `config.toml`; the remaining shortcuts are compiled in.
> There are no published binaries, and a local build carries only
> macOS's automatic ad-hoc signature — no signing identity, no notarization. Read
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
lifecycle, persistence, Zellij pass-through, worktree sessions — has landed,
as have configurable keybindings (palette surface), SSH connection launching
through the fixed system client, project rows that launch directory-rooted
sessions, and agent launching from configuration
(`local_project_worktree_and_agent_kinds_are_launchable` in
`crates/noren-app/src/session.rs`); the
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
