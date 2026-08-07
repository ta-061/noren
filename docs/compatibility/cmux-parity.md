# cmux parity research matrix

Snapshot: 2026-08-03. This document translates the feature groups in the
Noren project goal into testable product intent for
[Issue #4](https://github.com/ta-061/noren/issues/4). It is a research and
planning artifact, not a compatibility claim.

Noren is still in Discovery and has no terminal runtime, installable binary,
or Preview release ([README](../../README.md)). Consequently, every current
Noren row below is either **Planned** or **Not planned**. The
[project principles](../project-principles.md) prohibit treating a plan,
prototype, or untested feature as working, while the
[discovery plan](../research/discovery-plan.md) requires a source, test plan,
and honest state for every row.

On 2026-08-07, [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md)
drew the Noren/Zellij responsibility boundary: Noren manages the workspace
*outside* the terminal (a sidebar of projects, worktrees, SSH connections,
agents, and terminal sessions, with exactly one session visible); tabs, pane
splits, layout, and in-terminal focus are Zellij's job. Rows that previously
assumed Noren-side tabs/panes or layout persistence were reclassified with a
recorded disposition; no history was deleted.

## State legend

The `Noren state` column uses only this vocabulary. A state describes Noren,
never cmux.

| State | Meaning |
| --- | --- |
| Not planned | A comparison candidate with no accepted Noren delivery commitment. |
| Planned | Product intent exists, but no runtime evidence exists. |
| Experimental | A bounded experiment exists; it is not a product feature. |
| Partial | Some accepted behavior exists, but the row's pass criteria are not all met. |
| Supported | The behavior is implemented within a documented support boundary. |
| Tested | The supported behavior has current release evidence on every named target environment. |

## Evidence boundary and test targets

The upstream snapshot is cmux stable release
[v0.64.20](https://github.com/manaflow-ai/cmux/releases/tag/v0.64.20), published
2026-07-19 from commit
[`14e3400`](https://github.com/manaflow-ai/cmux/tree/14e3400b95daedd652d0b6f395d0777c41e39eef).
cmux documentation is mutable, so its official pages are recorded as retrieved
2026-08-03 and may not map exactly to that tag. **Every cmux behavior in the
matrix is an official public claim that was not independently executed or
verified for this report.** The matrix neither assigns a cmux status nor uses
cmux behavior as proof about Noren.

Future test targets are aliases, not claims that a fixture exists today:

- `C-MAC`: a future Noren release candidate on macOS Apple Silicon, using the
  Preview-supported OS version pinned by the release plan.
- `C-LINUX`: the same candidate on Linux x86_64 under both Wayland and X11,
  with compositor, desktop, and package versions recorded.
- `C-DUAL`: both `C-MAC` and `C-LINUX`.
- `C-SSH`: each client target connected to an isolated Linux x86_64 OpenSSH
  VM through a recorded clean-network and `tc netem` fault profile; only
  synthetic hosts, keys, and data may be captured.
- `C-SEC`: `C-DUAL` in an isolated account/runtime directory with a second
  unprivileged account for authorization tests and sanitized logs.

`codex-lab` owns future executable evidence unless a row says otherwise.
`Claude Code` is the additional review owner for security-sensitive IPC,
configuration, transfer, hook, and approval tests. These are evidence roles,
not implementation assignments.

## Preview priorities

| Feature | Noren state | cmux public behavior (not independently verified) | Future executable Noren test, target, and evidence owner |
| --- | --- | --- | --- |
| Workspace sidebar | Planned | Official pages describe a vertical workspace sidebar and workspace metadata ([Getting Started](https://cmux.com/docs/getting-started), [changelog](https://cmux.com/docs/changelog)). | `parity_sidebar_lifecycle`: create 12 named workspaces, select/reorder/close them by keyboard and pointer, and compare the model plus light/dark accessibility snapshots with the expected fixture. Target: `C-DUAL`. Owner: `codex-lab`. |
| Tabs | Not planned | Official shortcuts and the product page describe terminal/browser tabs within workspaces ([keyboard shortcuts](https://cmux.com/docs/keyboard-shortcuts), [product page](https://cmux.com/)). | Reclassified 2026-08-07 by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): in-terminal tabs are Zellij's job. Noren shows exactly one selected session; native tabs that duplicate Zellij are out of scope. The former `parity_tab_lifecycle` target is withdrawn — Noren has no tab model to lifecycle-test. |
| Panes | Not planned | Official pages describe horizontal and vertical split panes ([product page](https://cmux.com/), [keyboard shortcuts](https://cmux.com/docs/keyboard-shortcuts)). | Reclassified 2026-08-07 by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): in-terminal panes and splits are Zellij's job. Noren does not model, hold, or persist a pane/split tree; the former `parity_pane_lifecycle` target is withdrawn because there is no Noren pane topology to assert. Pane behavior inside a session is covered by the Zellij matrix `zellij_pane_operations` row instead. |
| Command palette | Planned | Official documentation exposes a searchable command palette and its default shortcut ([keyboard shortcuts](https://cmux.com/docs/keyboard-shortcuts)). | `parity_command_palette`: enumerate actions from the frozen Noren action manifest, filter and invoke each keyboard-only, then assert the action ID and that query keystrokes never reach the focused PTY. Target: `C-DUAL`. Owner: `codex-lab`. |
| Layout save | Not planned | cmux documents saving workspace splits and commands as layouts and separately limits session restore to app-owned state ([custom commands](https://cmux.com/docs/custom-commands), [session restore](https://cmux.com/docs/session-restore)). | Reclassified 2026-08-07 by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren must not persist Zellij's layout — it holds no layout tree to save. The former `parity_layout_roundtrip` target is replaced by `parity_sidebar_state_roundtrip`: save and restore Noren's sidebar state (which projects, worktrees, SSH targets, agents, and sessions exist, plus the selected session), assert versioned sidebar equality after restart, and assert no terminal content, tab/pane layout, key, passphrase, token, or raw command is persisted. Target: `C-DUAL`. Owner: `codex-lab`. |
| Project launch configuration | Planned | cmux documents project-local `.cmux/cmux.json` actions, commands, layouts, and trust prompts ([custom commands](https://cmux.com/docs/custom-commands), [configuration](https://cmux.com/docs/configuration)). | `parity_project_launch`: load an isolated project fixture containing structured argv, CWD, and environment sentinels; assert exact argv without shell interpolation, and assert an untrusted or invalid file cannot execute. Scoped by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md) to Noren-side actions only (creating/selecting a session from a project entry); project-local layout or pane directives are not Noren's to honor. Target: `C-SEC`. Owner: `codex-lab`; security review: `Claude Code`. |
| Notification history | Planned | cmux documents a notification lifecycle and panel history ([notifications](https://cmux.com/docs/notifications)). | `parity_notification_history`: inject bounded synthetic adapter and OSC notifications with IDs, assert source attribution to a **session** (sidebar entry), ordering, deduplication, retention, and sanitized persistence after restart. Re-anchored to sessions by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren cannot see inside a Zellij pane, so attribution stops at the session, not the pane. Target: `C-DUAL`. Owner: `codex-lab`. |
| Unread display | Planned | cmux documents unread workspace badges and transitions to read when the workspace is viewed ([notifications](https://cmux.com/docs/notifications)). | `parity_unread_state`: notify from background **sessions**, select them in a scripted order, and assert unread counts and transitions from the event log plus accessibility tree. Re-anchored to sessions by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren attributes unread state to a session (sidebar entry), not to a pane it cannot see. Target: `C-DUAL`. Owner: `codex-lab`. |
| Jump to notification source | Planned | cmux documents clicking a notification or using a shortcut to jump to its workspace ([notifications](https://cmux.com/docs/notifications)). | `parity_notification_jump`: emit uniquely tagged notifications from three **sessions**, invoke jump by UI and shortcut, and assert the exact source **session** gains selection without replaying input. Re-anchored to sessions by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren can select a session but cannot focus a pane inside it. Target: `C-DUAL`. Owner: `codex-lab`. |
| AI CLI state display | Planned | cmux documents agent-related notification/status surfaces and resource attribution ([notifications](https://cmux.com/docs/notifications), [Task Manager](https://cmux.com/docs/task-manager)). | `parity_agent_state_trust`: feed a fake adapter signed lifecycle events and assert the expected states; then expose only a matching process name and assert Noren displays `Unknown`. Target: `C-DUAL`. Owner: `codex-lab`. |
| CLI workspace operations | Planned | cmux documents CLI create/list/select/close operations for workspaces ([CLI reference](https://cmux.com/docs/api)). | `parity_cli_workspace_crud`: invoke the future Noren CLI create/list/select/close commands against a disposable instance, validate the versioned JSON schema, stable IDs, exit codes, and UI state after each command. Target: `C-DUAL`. Owner: `codex-lab`. |
| Local IPC | Planned | cmux documents a Unix-socket API and configurable access policy ([CLI reference](https://cmux.com/docs/api)). | `parity_ipc_boundary`: exercise same-user round trips, malformed/version-mismatched/oversized frames, socket mode, stale socket recovery, and a second account's denial; assert no crash or payload in logs. Target: `C-SEC`. Owner: `codex-lab`; security review: `Claude Code`. |
| SSH configuration loading | Planned | cmux says its SSH workflow reads host aliases, identities, and proxy settings from `~/.ssh/config` ([SSH](https://cmux.com/docs/ssh)). This does not prove complete OpenSSH semantics. | `parity_ssh_config_fixture`: resolve a synthetic config containing `Host`, `HostName`, `User`, `Port`, `IdentityFile`, `Include`, `ProxyJump`, and `ProxyCommand`; compare structured resolution with the pinned OpenSSH oracle without exposing credentials. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| SSH reconnect | Planned | cmux describes reconnect with capped exponential backoff and keepalives ([SSH](https://cmux.com/docs/ssh)). | `parity_ssh_reconnect`: interrupt transport with `tc netem`, assert a visible disconnect, bounded backoff schedule, cancellation, eventual recovery, and uninterrupted local UI responsiveness. Target: `C-SSH`. Owner: `codex-lab`. |
| Remote session persistence | Planned | cmux describes a remote daemon that keeps PTYs across reconnects and states that daemon's responsibilities ([SSH](https://cmux.com/docs/ssh)). This is not evidence for Noren's daemonless behavior. | `parity_remote_pty_persistence`: run a monotonic sentinel in a remote PTY, sever only transport, reconnect, and assert the same remote process/PTY continues with no duplicated or lost acknowledged input. Then test the separate Noren requirement with every remote helper blocked or disabled: show a visible semantic/accessibility state that persistence is unavailable, allocate an interactive PTY, verify `isatty` and the negotiated size, and run a raw-mode helper that reads a fixed numbered byte sentinel and emits its expected digest and sequence exactly once. Make the helper exit `37` and assert exact client exit-code propagation. Repeat with an abrupt disconnect while a uniquely tagged helper waits; from a clean control connection, assert its recorded PID is gone within the frozen timeout and no residual tagged process remains. The UI must continue ordinary structured OpenSSH while visibly degrading persistence rather than implying restore or continuity. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| New session on an SSH host | Planned | cmux describes remote workspaces and native panes in remote workflows ([SSH](https://cmux.com/docs/ssh), [remote tmux](https://cmux.com/docs/remote-tmux)). | `parity_remote_new_session`: add a **session** from a connected SSH target and assert its host fingerprint, user, CWD policy, PTY size, and failure isolation match the frozen requirement. Renamed from `parity_remote_new_pane` by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren creates a session (one visible terminal), not a pane; in-terminal splits remain Zellij's job. Target: `C-SSH`. Owner: `codex-lab`. |
| SSH session restore | Planned | cmux documents app-owned layout restoration and reconnectable remote sessions, with limitations on arbitrary process restoration ([session restore](https://cmux.com/docs/session-restore), [SSH](https://cmux.com/docs/ssh)). | `parity_ssh_session_restore`: save an SSH session's sidebar metadata, restart Noren, and assert host alias/session metadata restore without a key, passphrase, token, or raw command being persisted; require explicit reconnect behavior from the frozen policy. Renamed from `parity_ssh_workspace_restore` and rewritten by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren restores its own sidebar state (which SSH targets/sessions exist), never Zellij's layout — it holds none to restore. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |

## Initial-stable candidates

These rows are comparison candidates from the project goal. They are not
accepted delivery commitments; promotion requires a requirement, security
boundary, and release milestone.

| Feature | Noren state | cmux public behavior (not independently verified) | Conditional executable promotion test, target, and evidence owner |
| --- | --- | --- | --- |
| SFTP/SCP transfer | Not planned | cmux documents remote drag-and-drop uploads through `scp` over an existing SSH connection ([SSH](https://cmux.com/docs/ssh)). | If promoted, `candidate_remote_transfer_roundtrip`: round-trip binary, Unicode-name, empty, large, permission-denied, symlink, and interrupted files; verify hashes, modes, atomicity, path confinement, and redacted logs. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| File drag-and-drop | Not planned | cmux documents dragging a local file into a remote terminal to upload it ([SSH](https://cmux.com/docs/ssh)). | If promoted, `candidate_file_drop_routing`: drop multiple adversarially named paths into local and remote **sessions** and assert the configured insert/upload action, structured argument handling, cancel path, and correct destination **session**. Wording re-anchored to sessions by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md) so a future promotion does not reintroduce pane assumptions. Target: `C-DUAL` plus `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| Remote notifications | Not planned | cmux says remote processes can relay notifications to the local sidebar ([SSH](https://cmux.com/docs/ssh)). | If promoted, `candidate_remote_notification`: emit authenticated, duplicate, reordered, oversized, and post-disconnect synthetic events; assert host/**session** attribution, deduplication, bounds, and no execution side effect. Wording re-anchored to sessions by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren attributes to a session, not a pane it cannot see. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| Remote port list | Not planned | cmux says remote workspace metadata includes detected listening ports ([cmux SSH post](https://cmux.com/blog/cmux-ssh)). | If promoted, `candidate_remote_ports`: open recorded IPv4/IPv6 TCP listeners as two users, assert the permitted list and lifecycle, and assert unrelated users' process details are not exposed. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| Connect a remote port in the local browser | Not planned | cmux documents routing browser-pane traffic through the remote host without manual `-L` flags ([SSH](https://cmux.com/docs/ssh)). | If promoted, `candidate_remote_port_open`: expose an isolated HTTP sentinel remotely, open it in the system browser through Noren's bounded tunnel, assert response identity, bind address, teardown, collision handling, and no cookie capture. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| Per-session task display | Not planned | cmux's Task Manager claims attribution across windows, workspaces, panes, processes, agents, and webviews ([Task Manager](https://cmux.com/docs/task-manager)). | If promoted, `candidate_session_tasks`: launch a controlled process tree in two **sessions**, move and terminate children, and compare displayed attribution/PIDs/exits with the OS fixture; unknown ownership must remain explicit. Renamed and wording re-anchored to sessions by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md): Noren attributes to a session, not a pane inside it. Target: `C-DUAL`. Owner: `codex-lab`. |
| CPU and memory display | Not planned | cmux's Task Manager claims CPU and memory visibility for attributed activity ([Task Manager](https://cmux.com/docs/task-manager)). | If promoted, `candidate_resource_metrics`: run idle, CPU, and bounded-memory fixtures; compare sampled values with a pinned OS oracle within requirement-defined tolerance and assert sampling does not stall input. Target: `C-DUAL`. Owner: `codex-lab`. |
| Agent state badge | Not planned | cmux documents agent status metadata and agent-related notifications ([CLI reference](https://cmux.com/docs/api), [notifications](https://cmux.com/docs/notifications)). | If promoted, `candidate_agent_badge_trust`: drive a fake official adapter through each frozen state and stale/error transitions; a process-name-only fixture must show `Unknown`. Target: `C-DUAL`. Owner: `codex-lab`. |
| Project-local configuration | Not planned | cmux documents project-local configuration, precedence, reload, and trust prompts ([configuration](https://cmux.com/docs/configuration), [custom commands](https://cmux.com/docs/custom-commands)). | If promoted, `candidate_project_config_boundary`: test precedence and hot reload, reject traversal/unknown schema/command injection, preserve the last valid state on failure, and require trust for executable entries. Target: `C-SEC`. Owner: `codex-lab`; security review: `Claude Code`. |
| External notification hook | Not planned | cmux documents opt-in notification hooks that receive and return structured JSON, including timeout/failure behavior ([notifications](https://cmux.com/docs/notifications)). | If promoted, `candidate_notification_hook`: pass versioned JSON on stdin to success, failure, timeout, malformed-output, and recursion fixtures; assert bounded execution, redaction, fallback, and no shell concatenation. Target: `C-SEC`. Owner: `codex-lab`; security review: `Claude Code`. |

## Future candidates

| Feature | Noren state | cmux public behavior (not independently verified) | Conditional executable promotion test, target, and evidence owner |
| --- | --- | --- | --- |
| Embedded browser | Not planned | cmux advertises an in-app browser and scriptable browser API ([product page](https://cmux.com/), [browser automation](https://cmux.com/docs/browser-automation)). | If promoted, `candidate_embedded_browser_isolation`: load local fixtures for navigation, TLS error, popup, download, permission, cookie partition, crash, and automation authorization; assert terminal input remains isolated. Target: `C-DUAL` where a supported web engine exists. Owner: `codex-lab`; security review: `Claude Code`. |
| File browser | Not planned | cmux documents file-explorer configuration and public file-preview behavior ([configuration](https://cmux.com/docs/configuration), [cmux Finder post](https://cmux.com/blog/cmux-finder)). | If promoted, `candidate_file_browser_boundary`: enumerate a tree containing Unicode, symlinks, permission errors, churn, and traversal attempts; assert safe navigation, correct refresh, and no implicit execution. Target: `C-DUAL` plus `C-SSH` if remote browsing is in scope. Owner: `codex-lab`; security review: `Claude Code`. |
| Diff viewer | Not planned | cmux's changelog describes a diff viewer and large-diff streaming ([changelog](https://cmux.com/docs/changelog)). | If promoted, `candidate_diff_viewer_fidelity`: render a pinned Git repository fixture with rename, deletion, mode-only, binary, Unicode, long-line, and large diffs; compare parsed hunks and light/dark snapshots, with no content execution. Target: `C-DUAL`. Owner: `codex-lab`; security review: `Claude Code`. |
| Remote file editing | Not planned | cmux publicly describes remote file browsing, but no Noren-equivalent edit contract follows from that description ([SSH](https://cmux.com/docs/ssh), [configuration](https://cmux.com/docs/configuration)). | If promoted, `candidate_remote_edit_atomicity`: edit a synthetic remote file under concurrent change, disconnect, permission denial, symlink, and disk-full fixtures; assert conflict detection, atomic replacement, mode retention, and path confinement. Target: `C-SSH`. Owner: `codex-lab`; security review: `Claude Code`. |
| Mixed-host workspace | Not planned | cmux describes SSH workspaces and remote sessions; this report found no official claim that one workspace mixes multiple hosts ([SSH](https://cmux.com/docs/ssh)). Upstream equivalence is unknown. | If promoted, `candidate_mixed_host_routing`: combine local plus two SSH hosts, tag every PTY, route create/input/close actions, and fail one host; assert exact ownership and isolation of the others. Target: `C-SSH` with two VMs. Owner: `codex-lab`; security review: `Claude Code`. |
| Plugin registry | Not planned | cmux documents skills and an action registry, but those are not evidence of a Noren plugin-registry contract ([skills](https://cmux.com/docs/skills), [custom commands](https://cmux.com/docs/custom-commands)). | If promoted, `candidate_registry_supply_chain`: reject unsigned, tampered, traversal, downgrade, over-privileged, and unavailable packages while proving reproducible install/uninstall and offline behavior. Target: `C-SEC`. Owner: `codex-lab`; security review: `Claude Code`. |
| Agent Team integration | Not planned | cmux documents mapping Claude Code teammates to native splits and describes its tmux-compatible shim boundary ([Claude Code Teams](https://cmux.com/docs/agent-integrations/claude-code-teams)). | If promoted, `candidate_agent_team_mapping`: use a fake team protocol to create/reorder/exit teammates and assert **session** identity, lifecycle, authorization, and clean fallback when the upstream capability is absent. Wording re-anchored to sessions by [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md) so a future promotion maps teammates to Noren sessions, never to native panes/splits Noren does not own. Target: `C-DUAL` plus `C-SSH` if remote teams are in scope. Owner: `codex-lab`; security review: `Claude Code`. |
| Approval from an external messaging service | Not planned | cmux documents a Feed/notification policy surface, but this is not evidence for safe external approval semantics in Noren ([notifications](https://cmux.com/docs/notifications), [Dock](https://cmux.com/docs/dock)). | If promoted, `candidate_external_approval_boundary`: replay signed/expired/wrong-user/duplicate/modified approval fixtures and assert fail-closed identity, scope, expiry, audit, and that messages can never introduce an arbitrary shell command. Target: `C-SEC`. Owner: `codex-lab`; security review: `Claude Code`. |

## Open questions and non-claims

- No cmux binary was installed or executed for this report. UI details, defaults,
  error behavior, performance, accessibility, and macOS-version behavior remain
  unverified even where an official page describes a feature.
- Similar labels do not establish equivalent semantics. In particular,
  `workspace`, `tab`, `pane`, restore, reconnect, agent state, and notification
  trust boundaries require Noren requirements and black-box evidence.
- cmux is macOS-oriented; its behavior does not establish Linux feasibility or
  parity. Every planned Noren row therefore includes Linux evidence.
- Remote daemon adoption, browser-engine choice, process inspection, and plugin
  design remain architecture/RFC decisions. This matrix does not select them.
- The exact initial-stable and future scope is unresolved. `Not planned` means
  no commitment today, not a permanent rejection.

## Independent review record

Issue #4 assigns compatibility review to `codex-lab` and security review to
`Claude Code`. For this document the assigned scopes are exclusive:
`codex-lab` reviews compatibility and testability only, `Claude Code` reviews
security and privacy only, and the two scopes do not duplicate each other.
The stable review record is the discussion on
[PR #11](https://github.com/ta-061/noren/pull/11); findings and dispositions
are tracked there, and this section mirrors them for offline reading instead
of claiming any pending or final outcome.

Initial findings recorded as of the snapshot date:

- Security review (`Claude Code`) corrected the
  `parity_remote_pty_persistence` plan to require every remote helper to be
  blocked or disabled, a visible persistence-unavailable state, `isatty` and
  negotiated-size verification, exact exit-code propagation, and
  tagged-process cleanup after abrupt disconnect. Disposition: incorporated
  into the current text; reviewer sign-off is not claimed.
- Compatibility review (`codex-lab`) raised the resume-checkpoint finding on
  review-record stability, which applies to both matrices. Disposition:
  addressed by this section and the PR #11 record; reviewer confirmation
  remains open on the PR.

No reviewer verdict is recorded, and this document does not anticipate a
future clean verdict. Before a row advances beyond its current state, those
reviewers must inspect the cited claim, proposed fixture, target coverage,
secret redaction, and the actual diff; assignment alone is not review
evidence.

## Legal and trademark boundary

cmux is named only to identify the compared product. Noren is not affiliated
with or endorsed by cmux or Manaflow, and this repository must not reuse cmux
names as Noren branding, logos, screenshots, videos, UI artwork, or other marks.
No cmux code or assets were copied for this matrix.

The upstream LICENSE states that cmux is dual-licensed under
`GPL-3.0-or-later` or a commercial license; see its
[version-pinned license](https://github.com/manaflow-ai/cmux/blob/14e3400b95daedd652d0b6f395d0777c41e39eef/LICENSE)
and [release](https://github.com/manaflow-ai/cmux/releases/tag/v0.64.20).
That license evidence is a boundary, not legal advice and not a dependency
decision. Any later source reuse requires a separate license review; public
feature observation alone authorizes neither code nor asset copying.
