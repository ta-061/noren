# Independent design proposal task

Status: **Draft — do not execute until the Discovery input commit below is
filled in and all listed reports pass review.**

## Provenance

- Issue: #6
- Shared-input commit: `TBD`
- Prompt revision: 1
- Execution date: `TBD` (Asia/Tokyo)

Every proposer receives this file and the same immutable input snapshot. Model
responses are captured verbatim in separate files; command, tool version, model
identifier, outcome, duration, and word count are recorded outside the response.
Do not read another proposal or any cross-critique.

## Shared evidence

Read only these project inputs before answering:

- `README.md`
- `ROADMAP.md`
- `ARCHITECTURE.md`
- `docs/project-principles.md`
- `docs/research/terminal-landscape.md`
- `docs/research/library-comparison.md`
- `docs/compatibility/cmux-parity.md`
- `docs/compatibility/zellij.md`
- `docs/research/ssh-and-remote.md`
- `docs/research/agent-integrations.md`

Treat the reports as evidence, not as predetermined architecture. Do not browse,
modify files, run project commands, or inspect proposal/review outputs. If an
answer is absent from the shared evidence, label it `Unknown` or propose a
bounded experiment; do not invent an upstream API, behavior, benchmark, license,
or compatibility claim.

## Product brief

Noren is a new open-source Rust terminal for macOS and Linux. Its intended value
is a coherent workspace for local shells, SSH hosts, and existing CLI coding
agents while preserving input expected by Zellij, tmux, editors, shells, and
full-screen terminal applications. It may offer native tabs, panes, workspaces,
recovery, themes, notifications, and agent-aware navigation, but it is not a
Zellij replacement or an embedded LLM chat product. Input integrity, honest
feature status, recoverability, security, accessibility, and measurable release
evidence take priority over feature count.

The first public target is `0.1.0-preview`. Production implementation cannot
begin until requirements, architecture, threat model, test strategy, and
material ADR/RFC decisions pass independent review. Disposable experiments are
allowed only to resolve stated unknowns.

The requested product surface includes a local PTY; bash, zsh, and fish; modern
ANSI/VT behavior; Unicode, CJK, emoji, IME, and HiDPI; bounded scrollback,
selection, copy, paste, search, URLs, fonts, and zoom; multiple workspaces, tabs,
horizontal and vertical panes, layout save/restore, crash recovery, a command
palette, sidebar, and notification history; OpenSSH configuration and host-key
behavior; reconnectable SSH workspaces; existing CLI-agent launchers and only
evidence-backed agent state; separate UI themes and terminal palettes; and user
control over keybindings, layouts, notifications, and future extensions.

The requested Preview release floor includes successful macOS Apple Silicon and
Linux x86_64 builds; usable bash/zsh/fish sessions; stable local PTY and text I/O;
copy, paste, and search; tabs and panes; workspace save/restore; tested Zellij
input preservation and pass-through; SSH connection, disconnect detection, and
reconnect; light and dark themes; basic Codex, Claude Code, and OpenCode use;
jump from a notification to its pane; passing CI; no known critical crash or
security issue; license and secret scans; installation and known-limitations
documentation; an independent Claude security review; and a codex-lab release
verdict. A proposal may challenge feasibility or sequence, but it must identify
any requested floor it would defer and explain the release consequence.

## Fixed constraints and provisional goals

- When a terminal pane is focused, preservation of Ctrl, Alt, Ctrl+Alt, function
  keys, terminal protocols, and application input takes precedence over Noren
  convenience. Prefer Command-based defaults on macOS and avoid a large fixed
  Ctrl map on Linux. Every Noren shortcut must be rebindable or disableable.
- Zellij pass-through must minimize captured keys and allow a configurable
  leader, command-palette exit, and GUI fallback. Invalid configuration must be
  diagnosed while input continues to the child.
- SSH must respect existing OpenSSH config, agents, and `known_hosts`; Noren must
  not store private keys or passphrases. Ordinary OpenSSH remains available if a
  remote-session enhancement fails or is deferred. A daemon, if justified, uses
  SSH stdio or a tunnel by default rather than a public listening port.
- Agent state signal priority is official hook, official plugin API, structured
  output, explicit shell integration, OSC notification, then process metadata.
  Unsupported state is `Unknown`; a process name alone is insufficient.
- UI themes and terminal ANSI palettes are separate. Include light, dark, high-
  contrast, and color-vision-friendly intent; do not silently rewrite a user's
  chosen terminal colors.
- Provisional performance goals are input-to-render p95 within one frame, warm
  start within 500 ms on a recorded reference machine, responsive 100 MB output,
  usable eight-pane operation, bounded scrollback memory, no continuing memory
  growth in an eight-hour soak, and no whole-UI block during SSH failure. These
  are hypotheses to validate and may be revised by measured ADR.
- Required targets are macOS Apple Silicon and Linux x86_64 on Wayland and X11,
  with bash, zsh, and fish. macOS Intel, Linux ARM64, and Windows are Later unless
  evidence supports a smaller safe inclusion.
- Persisted configuration/workspace data uses versioning and atomic replacement;
  failed reload or migration preserves the last valid state. Bound OSC, config,
  IPC, diagnostic, and scrollback inputs. Keep `unsafe` minimal and explain each
  use with a `SAFETY` invariant.
- Do not select a terminal parser, PTY, SSH/crypto, renderer, font shaper, or
  plugin runtime without official-source/license/maintenance evidence and a PoC
  when the risk cannot be resolved on paper. Do not copy cmux code, assets, or
  marks.
- Begin with one repository. A remote daemon, conformance suite, site, or
  extension repository may split only when an independent release cycle or
  security boundary is demonstrated.

## Assignment

Write an independent, decision-oriented proposal for the smallest credible
Preview. Use exactly the 27 numbered headings below, in this order; they mirror
the project owner's required Round 1 submissions. Prefer concrete contracts,
failure semantics, measurable gates, and explicit deferral. Keep the response at
or below 4,000 words.

1. **Noren's central value**
2. **Target users**
3. **Problems Noren solves**
4. **Problems Noren does not solve**
5. **v0.1 Preview scope**
6. **Architecture proposal**
7. **Crate structure**
8. **Repository structure**
9. **Candidate libraries**
10. **Rendering approach**
11. **PTY approach**
12. **SSH approach**
13. **Remote-daemon approach**
14. **Zellij compatibility design**
15. **Keybinding design**
16. **CLI-agent integration**
17. **Theme approach**
18. **Plugin approach**
19. **Security risks**
20. **Performance risks**
21. **Portability risks**
22. **Ten greatest technical risks**
23. **Test strategy**
24. **Implementation order**
25. **Preview release conditions**
26. **Design decisions that should remain easy to discard**
27. **Overengineering to avoid**

## Required decision quality

- Distinguish `Preview`, `Later`, `Experiment`, and `Rejected` scope.
- Name the user-visible behavior and failure behavior of every Preview subsystem.
- Cover workspace interaction, persistence/recovery, rendering/fonts/IME/
  accessibility, Noren-owned API/data boundaries, packaging, rollback, support,
  and repository/crate dependency direction under the relevant required heading.
- Prefer Noren-owned interfaces around replaceable or security-sensitive
  dependencies; avoid speculative abstraction elsewhere.
- State which work can proceed in parallel and which dependencies serialize it.
- Give each performance target a measurement method and target environment; mark
  an ungrounded number as a proposed budget, not a measured fact.
- Give each compatibility claim a fixture or conformance-test path.
- Treat agent state as `Unknown` unless a trusted, documented signal proves it.
- Never store SSH private keys, passphrases, agent secrets, or API credentials.
- Preserve valid state when reload, migration, child process, remote transport,
  or plugin/adapter operations fail.
- Section 22 must contain exactly ten ranked risks. Each risk needs impact,
  trigger, mitigation, evidence needed, and release consequence.
- End section 27 with unresolved questions that could change the architecture.

This is a proposal, not an approval. The integration round will retain dissent
and may reject any recommendation that lacks evidence or a practical test.
