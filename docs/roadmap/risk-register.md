# Milestone 0 risk register

Status: Milestone 0 integration artifact for
[Issue #8](https://github.com/ta-061/noren/issues/8)

Created: 2026-08-03 (Asia/Tokyo) against `main` at
`b37126cd0f40c350f0ea4e28661aa7bdcd3dd3ac` (merge of PR
[#12](https://github.com/ta-061/noren/pull/12))

This register integrates already-merged Discovery evidence. It does not
select architecture, libraries, or dependencies, and it does not open the
production implementation gate
([D-0001](../coordination/decisions.md),
[design-process.md](../coordination/design-process.md)). Planned behavior
is not implemented behavior: every current Noren row in the source
matrices is `Planned` or `Not planned`, and unresolved evidence remains
`Unknown` rather than assumed.

Evidence is limited to merged repository reports and stable GitHub
artifacts:

- [terminal-landscape.md](../research/terminal-landscape.md) and
  [library-comparison.md](../research/library-comparison.md) — Issue
  [#3](https://github.com/ta-061/noren/issues/3), PR
  [#13](https://github.com/ta-061/noren/pull/13)
- [cmux-parity.md](../compatibility/cmux-parity.md) and
  [zellij.md](../compatibility/zellij.md) — Issue
  [#4](https://github.com/ta-061/noren/issues/4), PR
  [#11](https://github.com/ta-061/noren/pull/11)
- [ssh-and-remote.md](../research/ssh-and-remote.md) and
  [agent-integrations.md](../research/agent-integrations.md) — Issue
  [#5](https://github.com/ta-061/noren/issues/5), PR
  [#12](https://github.com/ta-061/noren/pull/12)
- [project-principles.md](../project-principles.md),
  [design-process.md](../coordination/design-process.md),
  [open-questions.md](../coordination/open-questions.md), and
  [agent-calibration.md](../coordination/agent-calibration.md)

## Independent review assignment (not yet performed)

Per [Issue #8](https://github.com/ta-061/noren/issues/8), the reviews of
this artifact are assigned but have not happened yet:

- `codex-lab`: testability and gate review (evidence completeness,
  measurable triggers, executable gate references). Assigned; no verdict.
- `Claude Code`: security and maintainability review. Assigned; no verdict.
- `Codex`: integrator and final decision maker. Makes no merge or closure
  decision in this draft; dissent and `Unknown` states are preserved.

## Scales

Likelihood (`L`) is the chance the risk materializes if it remains
unmitigated at the current evidence level. Impact (`I`) is the worst
credible outcome for users or the project. The `(L, I)` pair drives
prioritization; there is no numeric score.

| Likelihood | Meaning |
| --- | --- |
| L1 unlikely | No credible path in merged evidence. |
| L2 low | A path exists but requires several preconditions. |
| L3 possible | Merged evidence shows a credible path. |
| L4 likely | Merged evidence suggests the failure recurs in normal use if unaddressed. |
| L5 almost certain | The failure is proven by evidence unless behavior changes. |

| Impact | Meaning |
| --- | --- |
| I1 negligible | Cosmetic or self-healing. |
| I2 minor | Degraded experience with an obvious workaround. |
| I3 moderate | A feature or workflow is broken; no data loss or security exposure. |
| I4 major | Data loss, security exposure, or a broken core workflow. |
| I5 catastrophic | Irreversible data loss, credential/system compromise, or a release that violates the project principles. |

## Gate vocabulary

- A **design gate** is a named question under "Design council must decide"
  in [open-questions.md](../coordination/open-questions.md), or an
  RFC/ADR that Milestone 1 must resolve before the implementation gate in
  [design-process.md](../coordination/design-process.md) can open.
- A **release gate** is a named executable test or fixture suite from the
  merged matrices: `SSH-01`–`SEC-06` and `REM-01`–`REM-09`
  ([ssh-and-remote.md](../research/ssh-and-remote.md)),
  `CORE-01`–`OC-07` ([agent-integrations.md](../research/agent-integrations.md)),
  `zellij_*` ([zellij.md](../compatibility/zellij.md)), and
  `parity_*`/`candidate_*` ([cmux-parity.md](../compatibility/cmux-parity.md)).
  A release gate passes only when the named test has current executed
  evidence on every named target environment; until then the behavior is
  `Planned` or `Unknown`.

## Risk summary

| ID | Category | Risk | L | I | Owner | Gate |
| --- | --- | --- | --- | --- | --- | --- |
| [R-IN-01](#r-in-01) | Input loss | Shortcuts or pass-through consume bytes owed to the child | L3 | I5 | Design council; `codex-lab` | Design + release |
| [R-IN-02](#r-in-02) | Input loss | Unsettled key protocol negotiation forwards wrong bytes | L3 | I4 | Design council; `codex-lab` | Design + release |
| [R-DL-01](#r-dl-01) | Data loss | Persistence format, crash consistency, or migration loses saved state | L3 | I4 | `Codex`; `codex-lab` | Design + release |
| [R-DL-02](#r-dl-02) | Data loss | Failed configuration reload discards valid configuration | L3 | I4 | `Codex`; `codex-lab` | Design + release |
| [R-SEC-01](#r-sec-01) | Security | Destination/resume injection runs unintended local or remote commands | L3 | I5 | `Fugu` proposal; `Claude Code` review | Design + release |
| [R-SEC-02](#r-sec-02) | Security | Untrusted terminal bytes/OSC spoof state, clipboard, or IPC | L4 | I5 | Design council; `Claude Code` | Design + release |
| [R-SEC-03](#r-sec-03) | Security | Secrets leak via logs, dumps, notifications, or accessibility | L4 | I4 | `Claude Code`; `codex-lab` | Design + release |
| [R-SSH-01](#r-ssh-01) | SSH | Noren reinterprets OpenSSH configuration semantics | L4 | I4 | `Fugu` proposal; `codex-lab` fixtures | Design + release |
| [R-SSH-02](#r-ssh-02) | SSH | Host-key verification weakening enables MITM | L2 | I5 | `Claude Code`; `Fugu` | Design + release |
| [R-SSH-03](#r-ssh-03) | SSH | Reconnect misclassification retries or reports wrongly | L3 | I4 | `Fugu`; `codex-lab` | Design + release |
| [R-AGT-01](#r-agt-01) | Agent trust | Agent state guessed from names/text/OSC shows false status | L4 | I3 | Design council; `codex-lab` | Design + release |
| [R-AGT-02](#r-agt-02) | Agent trust | Hook/plugin bypass or unsafe install leaks secrets or forges state | L3 | I4 | `Claude Code`; `codex-lab` | Design + release |
| [R-PORT-01](#r-port-01) | Portability | macOS/Wayland/X11 divergence with a single window/event candidate | L4 | I4 | Design council; `codex-lab` | Design + release |
| [R-PORT-02](#r-port-02) | Portability | No Rust toolchain; buildability and MSRV unproven | L5 | I3 | Human owner (ta-061); `codex-lab` record | Design; blocking dependency |
| [R-PERF-01](#r-perf-01) | Performance | Rendering/parsing/IME throughput unmeasured | L4 | I3 | `codex-lab`; `GLM` proposals under review | Design + release |
| [R-DEP-01](#r-dep-01) | Dependencies/licenses | GPL boundary crossed; license/unsafe/advisory audit incomplete | L2 | I4 | `Codex`; `Claude Code`; human legal line | Design + release |
| [R-DEP-02](#r-dep-02) | Dependencies/licenses | Single-candidate or unstable dependency reaches adoption | L3 | I4 | Design council; `Codex` | Design + release |
| [R-A11Y-01](#r-a11y-01) | Accessibility | Terminal text unreachable by AT; tree leaks protected input | L4 | I4 | `Qwen` UI/a11y; `Claude Code` | Design + release |
| [R-REL-01](#r-rel-01) | Release integrity | Unsigned, substitutable, or unprovable artifacts and updates | L2 | I5 | Human owner (ta-061); `Codex` | Design + release |
| [R-REL-02](#r-rel-02) | Release integrity | Planned features advertised as supported | L4 | I3 | All owners; `codex-lab` evidence | Release |

## Risk detail

### R-IN-01

**Risk.** Noren shortcuts, presets, or pass-through logic consume terminal
bytes owed to the child, violating the first project principle.

- **Category:** Input loss.
- **Likelihood / Impact:** L3 possible / I5 catastrophic.
- **Evidence:** [project-principles.md](../project-principles.md) product
  integrity rule 1; [zellij.md](../compatibility/zellij.md) "Noren Zellij
  Pass-through Mode" row and two-layer `Z-2L` oracle;
  [terminal-landscape.md](../research/terminal-landscape.md) lesson 4
  (multiplexers are guests, not hidden dependencies);
  [agent-calibration.md](../coordination/agent-calibration.md) shared
  finding that pass-through entry is invalid without a configurable,
  reachable exit and a non-keyboard fallback.
- **Owner:** Design council for shortcut policy; `codex-lab` for byte-oracle
  evidence; `Claude Code` reviews clipboard/OSC edges.
- **Mitigation:** minimal interception manifest; every shortcut
  independently rebindable and disableable; always-reachable pointer-invoked
  palette/GUI recovery; no reserved bytes in a focused terminal pane;
  keyboard traps are test failures.
- **Trigger:** Milestone 1 specification of keybinding configuration,
  presets, or pass-through entry/exit.
- **Gate.** Design: pass-through/keybinding questions in
  [open-questions.md](../coordination/open-questions.md). Release:
  `zellij_default_preset_trace`, `zellij_unlock_first_trace`,
  `noren_zellij_compatible_preset`, `noren_zellij_unlock_first_preset`,
  `noren_zellij_pass_through` on `Z-PROTO`+`Z-SSH`, including
  pointer/accessibility recovery runs
  ([zellij.md](../compatibility/zellij.md)).

### R-IN-02

**Risk.** Unsettled keyboard protocol negotiation (Kitty keyboard protocol,
CSI-u, modifyOtherKeys, legacy xterm encoding) forwards wrong bytes or
advertises capabilities Noren cannot preserve.

- **Category:** Input loss (correctness).
- **Likelihood / Impact:** L3 possible / I4 major.
- **Evidence:** [zellij.md](../compatibility/zellij.md) protocol matrix
  (KKP is a deliberate subset; modifyOtherKeys is not KKP) and
  "Negotiation, layout, and IME unknowns";
  [open-questions.md](../coordination/open-questions.md) key protocol
  negotiation question.
- **Owner:** Design council; `codex-lab` fixtures.
- **Mitigation:** byte-for-byte fixtures per protocol; `$TERM` or brand
  strings are never proof of a negotiated capability; Noren reports and
  emits only the subset it implements; mode restoration after normal exit
  and forced failure.
- **Trigger:** Milestone 1 key protocol decision or the first parser/state
  PoC.
- **Gate.** Design: key protocol question in
  [open-questions.md](../coordination/open-questions.md). Release:
  `zellij_kkp_byte_trace`, `zellij_legacy_key_trace`,
  `zellij_modify_other_keys_trace`
  ([zellij.md](../compatibility/zellij.md)).

### R-DL-01

**Risk.** Workspace persistence format, crash consistency, or migration
loses saved layouts, sessions, favorites, or history.

- **Category:** Data loss.
- **Likelihood / Impact:** L3 possible / I4 major.
- **Evidence:** [open-questions.md](../coordination/open-questions.md)
  workspace persistence question;
  [project-principles.md](../project-principles.md) security/reliability
  rule 4 (persisted data versioned, atomic writes, valid state preserved);
  [cmux-parity.md](../compatibility/cmux-parity.md) `parity_layout_roundtrip`
  and `parity_ssh_workspace_restore`;
  [ssh-and-remote.md](../research/ssh-and-remote.md) SSH-11
  favorites/history persistence and corruption.
- **Owner:** `Codex` integration; `codex-lab` executable evidence.
- **Mitigation:** versioned persistence format; atomic writes; last-valid
  state preserved on failed reload; no keys, passphrases, tokens, or raw
  commands persisted; explicit migration and corruption tests.
- **Trigger:** Milestone 1 persistence requirement or the first
  save/restore implementation.
- **Gate.** Design: workspace persistence question in
  [open-questions.md](../coordination/open-questions.md). Release:
  `parity_layout_roundtrip`, `parity_ssh_workspace_restore`
  ([cmux-parity.md](../compatibility/cmux-parity.md)), `SSH-11`
  ([ssh-and-remote.md](../research/ssh-and-remote.md)).

### R-DL-02

**Risk.** A failed configuration reload silently discards or corrupts a
valid configuration.

- **Category:** Data loss.
- **Likelihood / Impact:** L3 possible / I4 major.
- **Evidence:** [project-principles.md](../project-principles.md)
  security/reliability rule 4;
  [library-comparison.md](../research/library-comparison.md) section 10
  (failed-reload preservation, atomic writes, and transactional activation
  are Noren-owned, not parser-provided);
  [agent-calibration.md](../coordination/agent-calibration.md) shared
  finding that invalid configuration must produce diagnostics while
  forwarding input.
- **Owner:** `Codex` integration; `codex-lab` fixtures.
- **Mitigation:** a `ConfigCodec` boundary returning versioned raw config
  plus diagnostics; bounded hostile-input tests (invalid, duplicate,
  unknown, huge, deeply nested); transactional activation with rollback.
- **Trigger:** Milestone 1 configuration schema definition.
- **Gate.** Design: configuration schema/reload policy in Milestone 1
  requirements (Issue [#6](https://github.com/ta-061/noren/issues/6)
  scope). Release: section 10 validating PoCs
  ([library-comparison.md](../research/library-comparison.md)) plus
  `candidate_project_config_boundary` if promoted
  ([cmux-parity.md](../compatibility/cmux-parity.md)).

### R-SEC-01

**Risk.** SSH destination handling or agent resume triggers unintended
local or remote command execution (option injection, shell metacharacters,
leading-dash argv).

- **Likelihood / Impact:** L3 possible / I5 catastrophic.
- **Category:** Security.
- **Evidence:** [library-comparison.md](../research/library-comparison.md)
  section 8: a destination starting with `-` is parsed as an `ssh` option
  because the official synopsis lists no `--` marker, and `%h`/`%n` token
  expansion reaches configured `Match exec` shell text even under
  structured argv; [ssh-and-remote.md](../research/ssh-and-remote.md)
  process and command boundaries plus SEC-01/SEC-02/SEC-04;
  [agent-integrations.md](../research/agent-integrations.md) CORE-06 and
  CORE-13 resume injection matrices;
  [project-principles.md](../project-principles.md) security rule 2.
- **Owner:** `Fugu` SSH/remote state-machine proposal; `Claude Code`
  security review; `codex-lab` fixtures.
- **Mitigation:** structured argv only; reject leading-dash destinations
  until a version-tested end-of-options contract exists; never concatenate a
  shell command; remote helpers receive values over framed stdin, never
  interpolated into the command string; property-test hostile inputs
  including quotes, newlines, metacharacters, and option prefixes.
- **Trigger:** any work that invokes `ssh`, constructs a remote command, or
  enables an agent resume path.
- **Gate.** Design: OpenSSH subprocess versus library question in
  [open-questions.md](../coordination/open-questions.md). Release:
  `SEC-01`, `SEC-02`, `SEC-04`
  ([ssh-and-remote.md](../research/ssh-and-remote.md)), `CORE-06`,
  `CORE-13` ([agent-integrations.md](../research/agent-integrations.md)).

### R-SEC-02

**Risk.** Untrusted terminal byte streams or OSC sequences spoof trusted
state, exfiltrate or overwrite the clipboard, or invoke IPC.

- **Category:** Security.
- **Likelihood / Impact:** L4 likely / I5 catastrophic.
- **Evidence:** [zellij.md](../compatibility/zellij.md) OSC 52 row (Noren
  must never answer an OSC 52 read query), OSC 8 unknowns, and
  "Security, legal, and trademark boundary";
  [agent-integrations.md](../research/agent-integrations.md) CORE-07 and
  the terminal-content threat boundary;
  [terminal-landscape.md](../research/terminal-landscape.md) lesson 5
  (versioned, authorized control surfaces; fuzzed framing; an untrusted
  terminal byte stream must not invoke IPC);
  [project-principles.md](../project-principles.md) security rule 3.
- **Owner:** Design council trust policy; `Claude Code` review; `codex-lab`
  fixtures.
- **Mitigation:** bounded OSC payloads and untrusted configuration; IPC is
  local-only, peer-authenticated, versioned, least-privilege, with frame
  caps and timeouts; clipboard writes permissioned and read queries denied;
  terminal bytes are data everywhere, never authority.
- **Trigger:** parser/state PoC, OSC policy, or IPC design.
- **Gate.** Design: trust boundary and permissions question in
  [open-questions.md](../coordination/open-questions.md). Release:
  `zellij_osc52_security`, `zellij_osc8_trace`,
  `zellij_bracketed_paste_trace` ([zellij.md](../compatibility/zellij.md)),
  `CORE-07` ([agent-integrations.md](../research/agent-integrations.md)),
  and the section 9 adversarial IPC PoC
  ([library-comparison.md](../research/library-comparison.md)).

### R-SEC-03

**Risk.** Secrets, prompts, topology, session IDs, or protected input leak
through logs, crash dumps, notifications, accessibility trees, or
telemetry.

- **Category:** Security.
- **Likelihood / Impact:** L4 likely / I4 major.
- **Evidence:** [project-principles.md](../project-principles.md) security
  rule 1; [agent-integrations.md](../research/agent-integrations.md)
  CORE-05 and generic OS notification text rule;
  [ssh-and-remote.md](../research/ssh-and-remote.md) SEC-06 canaries
  covering `ssh -G` oracle output;
  [library-comparison.md](../research/library-comparison.md) section 11
  (logging field and minidump exposure) and section 14 (accessibility tree
  can leak protected input or grow unbounded).
- **Owner:** `Claude Code` review; `codex-lab` canary fixtures.
- **Mitigation:** closed redacted event schema; per-event field allowlists;
  unique canary credentials seeded in every input field and asserted absent
  from logs, notifications, webhook fixtures, crash reports, and artifacts;
  bounded scrollback window in the accessibility tree; dump consent and
  retention policy before any collection.
- **Trigger:** first logging, crash-reporting, notification, or
  accessibility implementation.
- **Gate.** Design: Milestone 1 threat model
  ([design-process.md](../coordination/design-process.md) round 3).
  Release: `CORE-05`
  ([agent-integrations.md](../research/agent-integrations.md)), `SEC-06`
  ([ssh-and-remote.md](../research/ssh-and-remote.md)), the section 11
  redaction PoC with synthetic secrets, and the section 14 protected-input
  accessibility PoC ([library-comparison.md](../research/library-comparison.md)).

### R-SSH-01

**Risk.** Noren silently reinterprets OpenSSH configuration (Host aliases,
Match/Match exec, Include, IdentityAgent/IdentitiesOnly, ProxyJump
precedence) and applies the wrong connection or security settings.

- **Category:** SSH.
- **Likelihood / Impact:** L4 likely / I4 major.
- **Evidence:** [ssh-and-remote.md](../research/ssh-and-remote.md)
  "OpenSSH behavior Noren must not silently reinterpret" table and
  "Confirmed russh-config limits at the pinned source" (no Include/Match/
  IdentityAgent/IdentitiesOnly handling; StrictHostKeyChecking collapsed to
  a boolean; ProxyJump parsed but unused);
  [library-comparison.md](../research/library-comparison.md) section 8
  screening result (a Rust parser is not equivalent to `ssh -G` until the
  differential suite proves a defined subset).
- **Owner:** `Fugu` proposal; `codex-lab` fixtures; `Claude Code` security
  review.
- **Mitigation:** destinations whose evaluated configuration depends on
  unsupported semantics fall back to the ordinary OpenSSH path and are
  marked unsupported, never silently approximated; differential fixtures
  against pinned `ssh -G`; `ssh -G` output stays redacted or in isolated
  fixture storage.
- **Trigger:** SSH transport or config resolver selection.
- **Gate.** Design: OpenSSH subprocess versus library question. Release:
  `SSH-01`–`SSH-04` ([ssh-and-remote.md](../research/ssh-and-remote.md)),
  `parity_ssh_config_fixture`
  ([cmux-parity.md](../compatibility/cmux-parity.md)).

### R-SSH-02

**Risk.** Host-key verification is weakened (connect-anyway, auto-accept,
wrong lookup name for aliases/ports/proxies), enabling MITM or credential
exposure.

- **Category:** SSH.
- **Likelihood / Impact:** L2 low / I5 catastrophic.
- **Evidence:** [ssh-and-remote.md](../research/ssh-and-remote.md) "Host
  identity and credentials" and SSH-05;
  [library-comparison.md](../research/library-comparison.md) section 7
  (`russh` client handler defaults to rejection, so policy must be
  implemented; `ssh2` requires explicit verification and persistence);
  [open-questions.md](../coordination/open-questions.md) first-use
  host-key confirmation UI question.
- **Owner:** `Claude Code` review; `Fugu` proposal.
- **Mitigation:** preserve the user's strict known-host policy; changed,
  revoked, or refused keys are terminal for automatic reconnect; never
  parse a prompt to auto-accept; an embedded store must persist accepted
  keys atomically with safe permissions; host-key mismatches never
  auto-retry.
- **Trigger:** embedded-transport PoC or first-use UI design.
- **Gate.** Design: first-use host-key confirmation question in
  [open-questions.md](../coordination/open-questions.md). Release:
  `SSH-05` ([ssh-and-remote.md](../research/ssh-and-remote.md)).

### R-SSH-03

**Risk.** Reconnect logic misclassifies failures (exit-255 ambiguity,
spoofable TCP keepalive, stale or colliding control sockets) and retries
the wrong way or reports the wrong state.

- **Category:** SSH.
- **Likelihood / Impact:** L3 possible / I4 major.
- **Evidence:** [ssh-and-remote.md](../research/ssh-and-remote.md)
  exit-255 ambiguity (SSH-08), ServerAliveInterval versus spoofable
  TCPKeepAlive (SSH-06), ControlMaster collision and stale-master rows
  (SSH-07), the connection/reconnect state model (REM-09), and
  "never automatically retry a host-key change, trust rejection, invalid
  destination, authentication failure, or protocol/hash incompatibility";
  [zellij.md](../compatibility/zellij.md) SSH segmentation unknowns.
- **Owner:** `Fugu`; `codex-lab` fault-injection fixtures.
- **Mitigation:** keep transport state and remote-session state separate;
  classify `Unknown` unless a separately attributable client/protocol
  signal exists; retry only classified transient failures with bounded,
  cancelable, jittered delays; multiplex reuse binds destination, user,
  port, proxy context, and agent/security context or opens a new
  connection.
- **Trigger:** reconnect state-machine design.
- **Gate.** Design: daemon/remote-session question in
  [open-questions.md](../coordination/open-questions.md). Release:
  `SSH-06`, `SSH-07`, `SSH-08`, `REM-09`
  ([ssh-and-remote.md](../research/ssh-and-remote.md)),
  `parity_ssh_reconnect` ([cmux-parity.md](../compatibility/cmux-parity.md)).

### R-AGT-01

**Risk.** Agent state is guessed from process names, terminal text, or
forgeable OSC and shows false `Completed`/`Error`/awaiting states or fires
wrong automation.

- **Category:** Agent trust.
- **Likelihood / Impact:** L4 likely / I3 moderate.
- **Evidence:** [agent-integrations.md](../research/agent-integrations.md)
  normalized state contract "What is insufficient" column, CORE-07
  spoofing fixtures, "Fallback behavior" (`Unknown` fallback is the honest
  level); [project-principles.md](../project-principles.md) product
  integrity rule 4; [cmux-parity.md](../compatibility/cmux-parity.md)
  `parity_agent_state_trust` (process-name-only fixture must show
  `Unknown`).
- **Owner:** Design council adapter requirements; `codex-lab` fixtures.
- **Mitigation:** version-scoped adapters with capability snapshots;
  source-provided ordering tokens, session correlation, and lease expiry;
  passive, bounded observer hooks; no process-name scanning; unsolved
  states stay `Unknown` and suppress automation.
- **Trigger:** first adapter implementation or notification design.
- **Gate.** Design: trust boundary and permissions question in
  [open-questions.md](../coordination/open-questions.md) plus Milestone 1
  adapter requirements. Release: `CORE-02`, `CORE-04`, `CORE-07`, and the
  `CX-*`, `CL-*`, `OC-*` version-scoped suites
  ([agent-integrations.md](../research/agent-integrations.md)).

### R-AGT-02

**Risk.** Hook/plugin trust bypass or silent installation lets repository
code execute as the user or forge state, and hook payloads leak prompts,
session IDs, or secrets.

- **Category:** Agent trust.
- **Likelihood / Impact:** L3 possible / I4 major.
- **Evidence:** [agent-integrations.md](../research/agent-integrations.md)
  Codex section (matching hooks run concurrently; the local `0.146.0` CLI
  advertises `--dangerously-bypass-hook-trust`, which Noren must not use as
  an installation shortcut), CORE-01/CORE-05, and the hook-configuration
  threat boundary; [cmux-parity.md](../compatibility/cmux-parity.md)
  `parity_project_launch` (untrusted project files must not execute).
- **Owner:** `Claude Code` review; `codex-lab` fixtures.
- **Mitigation:** explicit trust/install UI with exact-file/hash
  provenance; hash or configuration change invalidates trust; observer is
  passive and fail-open for observation only; removal path; strict
  per-event field allowlists with redaction before formatting.
- **Trigger:** any hook/plugin distribution, installation, or project-local
  command feature.
- **Gate.** Design: trust boundary and permissions question. Release:
  `CORE-01`, `CORE-05`, `CORE-09`
  ([agent-integrations.md](../research/agent-integrations.md)),
  `parity_project_launch` ([cmux-parity.md](../compatibility/cmux-parity.md)).

### R-PORT-01

**Risk.** macOS/Wayland/X11 input, windowing, IME, or accessibility
behavior diverges, and the window/event function currently has one
supported candidate, concentrating platform risk.

- **Category:** Portability.
- **Likelihood / Impact:** L4 likely / I4 major.
- **Evidence:** [library-comparison.md](../research/library-comparison.md)
  section 3B "Supported-candidate gap" and its PoC/drop gate (a second
  supportable candidate or explicit market-gap justification is required
  before an adoption ADR), section 6 IME window-ownership coupling;
  [terminal-landscape.md](../research/terminal-landscape.md) explicit
  unknowns (IME equivalence across macOS/Wayland/X11; Linux X11 support is
  itself an open product question); [zellij.md](../compatibility/zellij.md)
  `Z-MAC`/`Z-WAYLAND`/`Z-X11` target definitions.
- **Owner:** Design council; `codex-lab` platform measurements.
- **Mitigation:** pin and run the identical lifecycle/input/IME trace on
  AppKit, Wayland, and X11 before any adoption ADR; treat identical API
  shapes as not proving identical platform behavior; record X11 support as
  a product decision, not an assumption.
- **Trigger:** Milestone 1 window/event selection.
- **Gate.** Design: window/renderer/font/IME/accessibility stack question
  in [open-questions.md](../coordination/open-questions.md). Release: the
  section 3B `winit` trace PoC and `zellij_japanese_ime` on
  `Z-MAC`/`Z-WAYLAND`/`Z-X11`
  ([zellij.md](../compatibility/zellij.md)).

### R-PORT-02

**Risk.** No Rust toolchain is installed, so buildability, MSRV, and
reproducible toolchain pinning remain unproven and every executable gate in
this register is blocked.

- **Category:** Portability.
- **Likelihood / Impact:** L5 almost certain (current state) / I3 moderate.
- **Evidence:** [status.md](../coordination/status.md) blocked section;
  [agent-inventory.md](../coordination/agent-inventory.md) development
  environment row (`cargo`, `rustc`, `rustup` not installed);
  [open-questions.md](../coordination/open-questions.md) MSRV/toolchain
  question; Dependabot `cargo` ecosystem runs fail on `main` because the
  repository has no Cargo manifest or toolchain (see
  [status.md](../coordination/status.md) for run IDs).
- **Owner:** Human owner (ta-061) approves installation; `codex-lab`
  records versions.
- **Mitigation:** install a pinned toolchain before the first Rust
  experiment; record `rustc`/`cargo` versions and target triples; define
  MSRV evidence before any adoption ADR.
- **Trigger:** first experiment that requires compilation.
- **Gate.** Design: MSRV/toolchain question in
  [open-questions.md](../coordination/open-questions.md). Release: none
  directly; this is a blocking dependency of every executable gate above.

### R-PERF-01

**Risk.** Rendering, parsing, IME, or output throughput claims are made
without measurements, and performance regressions are discovered late.

- **Category:** Performance.
- **Likelihood / Impact:** L4 likely / I3 moderate.
- **Evidence:** [terminal-landscape.md](../research/terminal-landscape.md)
  lessons 1 and 2 (parser corpus and renderer trace measurements);
  [zellij.md](../compatibility/zellij.md) "Large output and scrollback"
  unknown (measurable NFRs are required);
  [agent-calibration.md](../coordination/agent-calibration.md) evidence
  note that an unmeasured microsecond performance assertion was rejected.
- **Owner:** `codex-lab` measurements; `GLM` bounded proposals under
  compile/test review.
- **Mitigation:** Milestone 1 must define frozen latency, memory, and
  throughput budgets as measurable NFRs; deterministic numbered/hashed load
  corpora; p50/p95 frame time, upload bytes, idle CPU, and resize latency
  recorded per PoC; no performance claim without measurement.
- **Trigger:** renderer/parser PoCs or Milestone 1 NFR definition.
- **Gate.** Design: Milestone 1 non-functional requirements
  ([design-process.md](../coordination/design-process.md) round 3
  artifacts). Release: `zellij_large_output_soak` on `Z-LOAD`
  ([zellij.md](../compatibility/zellij.md)) plus the section 3A renderer
  measurements ([library-comparison.md](../research/library-comparison.md)).

### R-DEP-01

**Risk.** GPL-licensed references or assets cross Noren's intended
MIT/Apache-2.0 boundary, or candidates advance with incomplete license,
unsafe, or advisory evidence.

- **Category:** Dependencies/licenses.
- **Likelihood / Impact:** L2 low / I4 major.
- **Evidence:** [terminal-landscape.md](../research/terminal-landscape.md)
  legal and clean-room boundary (kitty GPL-3.0 and iTerm2 GPL-2.0 are
  behavior/protocol references only; no source or asset reuse proposed);
  [cmux-parity.md](../compatibility/cmux-parity.md) legal and trademark
  boundary (cmux `GPL-3.0-or-later` or commercial);
  [library-comparison.md](../research/library-comparison.md) method and
  limits (no locked dependency graph, recursive unsafe inventory, or
  complete advisory history exists yet for any Rust candidate).
- **Owner:** `Codex` integration; `Claude Code` review; human owner makes
  the final legal line.
- **Mitigation:** specification-driven implementation, not translated
  upstream code; no copied screenshots, icons, themes, or marks; every PoC
  preserves lockfile, license and advisory scan dates, enabled features,
  and a recursive unsafe inventory; a license identifier is screening
  evidence, not legal advice.
- **Trigger:** any candidate advancing to PoC or adoption.
- **Gate.** Design: evidence-to-decision gate in
  [library-comparison.md](../research/library-comparison.md) applied by the
  design council. Release: per-candidate license/advisory/unsafe inventory
  required by the common validation rules in
  [library-comparison.md](../research/library-comparison.md).

### R-DEP-02

**Risk.** A single-candidate gap or an unstable 0.x/release-candidate
dependency reaches adoption without a second-candidate PoC or an explicit,
recorded market-gap justification.

- **Category:** Dependencies/licenses.
- **Likelihood / Impact:** L3 possible / I4 major.
- **Evidence:** [library-comparison.md](../research/library-comparison.md)
  section 3B window/event gap and section 12B installer gap with their
  PoC/drop gates; the evidence-to-decision gate ("no candidate may advance
  from this report directly to production");
  [terminal-landscape.md](../research/terminal-landscape.md) lesson 6
  (measure release and source cadence separately; pin versions and commits,
  never "latest").
- **Owner:** Design council; `Codex` gate enforcement.
- **Mitigation:** before any adoption ADR, run a second candidate under the
  identical corpus or record an explicit single-candidate justification and
  replacement plan; exact version plus source-commit pinning with lockfile;
  reject claims based only on stars, download counts, or unversioned
  default branches.
- **Trigger:** any adoption ADR in Milestone 1.
- **Gate.** Design: evidence-to-decision gate enforced by the design
  council. Release: the per-category PoC/drop gates recorded in
  [library-comparison.md](../research/library-comparison.md).

### R-A11Y-01

**Risk.** Terminal-scale dynamic text is not actually reachable by screen
readers, or the accessibility tree leaks protected input or consumes
unbounded memory.

- **Category:** Accessibility.
- **Likelihood / Impact:** L4 likely / I4 major.
- **Evidence:** [library-comparison.md](../research/library-comparison.md)
  section 14 (AccessKit documents rich-text/hypertext limitations; a custom
  GPU-rendered terminal does not automatically get correct text semantics;
  "Accessibility is a functional requirement and release gate, not an
  optional polish layer");
  [terminal-landscape.md](../research/terminal-landscape.md) lesson 7 and
  explicit unknown (no candidate tested with terminal-sized dynamic text,
  large scrollback, rapid updates, or protected input).
- **Owner:** `Qwen` UI/accessibility; `Claude Code` review.
- **Mitigation:** expose a bounded semantic snapshot (grid, selection, focus,
  title, bounded scrollback window) independent of renderer pixels; test
  VoiceOver and AT-SPI for reading order, navigation, rapid updates,
  resize, IME preedit, protected input, memory, and latency.
- **Trigger:** renderer/window selection (composes with R-PORT-01).
- **Gate.** Design: window/renderer/font/IME/accessibility stack question.
  Release: the section 14 validating PoCs on macOS NSAccessibility and
  Linux AT-SPI ([library-comparison.md](../research/library-comparison.md)).

### R-REL-01

**Risk.** Preview artifacts ship without trustworthy signing, notarization,
checksums, provenance, or rollback, or an update path accepts substituted,
replayed, or downgraded binaries.

- **Category:** Release integrity.
- **Likelihood / Impact:** L2 low / I5 catastrophic.
- **Evidence:** [library-comparison.md](../research/library-comparison.md)
  sections 12, 12A, and 12B (signature/checksum policy, downgrade, archive
  traversal, atomic replacement, rollback, and the installer PoC/drop gate;
  a digest sent over the same untrusted channel detects only transfer
  corruption); [ssh-and-remote.md](../research/ssh-and-remote.md) REM-07
  upgrade-channel threat; [open-questions.md](../coordination/open-questions.md)
  signing/notarization identity question.
- **Owner:** Human owner (ta-061) for any signing identity; `Codex` release
  gates; `codex-lab` release evidence.
- **Mitigation:** produce unsigned test artifacts first in a locked CI
  sandbox; inspect generated workflows, SBOM/licenses, checksums, and
  provenance; independent signed manifest plus digest verification before
  replacement; atomic install with health check and rollback; no automatic
  upload; signing identity is an explicit human decision and none is
  assumed.
- **Trigger:** packaging PoC or Preview release planning.
- **Gate.** Design: signing/notarization identity question in
  [open-questions.md](../coordination/open-questions.md) plus the Milestone
  1 release plan. Release: section 12 packaging PoC and section 12B
  installer PoC ([library-comparison.md](../research/library-comparison.md)),
  `REM-07` ([ssh-and-remote.md](../research/ssh-and-remote.md)).

### R-REL-02

**Risk.** Planned or unimplemented features are advertised as supported, or
release claims outrun executed evidence.

- **Category:** Release integrity.
- **Likelihood / Impact:** L4 likely / I3 moderate.
- **Evidence:** [project-principles.md](../project-principles.md) product
  integrity rules 2 and 5; the state legends in
  [cmux-parity.md](../compatibility/cmux-parity.md) and
  [zellij.md](../compatibility/zellij.md) ("Planned" means no runtime
  evidence exists); [ROADMAP.md](../../ROADMAP.md) marks only
  evidence-backed work complete.
- **Owner:** every document owner; `codex-lab` release evidence.
- **Mitigation:** keep the `Planned`/`Not planned`/`Experimental`/`Partial`/
  `Supported`/`Tested` vocabulary in all matrices; the dependency-free
  documentation checker runs in CI; Preview release review requires
  executed evidence for every public claim.
- **Trigger:** every documentation or marketing change; Preview release
  notes.
- **Gate.** Release: the Milestone 8 release review
  ([ROADMAP.md](../../ROADMAP.md)) verifies each public claim against
  executed evidence; the design-process round 3 artifacts supply the
  release plan.

## Architecture-changing unknowns and dispositions

Every open question that can change architecture is mapped below to a
bounded experiment, a named design-council/RFC question, or an explicit
deferral. Sources are [open-questions.md](../coordination/open-questions.md)
("Design council must decide") and the explicit unknown sections of the
merged reports. No row here is an answer; `Unknown` states are preserved.

| Unknown | Disposition | Bound |
| --- | --- | --- |
| Terminal parser/state library boundary and replacement strategy | Bounded experiment feeding the design council | Identical parser/state corpus plus fuzz for `vte`/`vtparse` (section 1) and `avt` versus `alacritty_terminal` under the same corpus with drop gates (section 1A), [library-comparison.md](../research/library-comparison.md) |
| Window, GPU renderer, font shaping, IME, and accessibility stack | Bounded experiments feeding the design council | Sections 3A/3B/4/6/14 PoCs with identical traces on Metal and Vulkan/GL paths; window gap requires a second candidate or recorded justification, [library-comparison.md](../research/library-comparison.md) |
| PTY abstraction and platform-specific ownership | Bounded experiment | `portable-pty` versus `nix` suite (spawn without shell, resize storms, EOF, signal/exit races, descriptor leakage, sanitizer runs), [library-comparison.md](../research/library-comparison.md) section 2 |
| OpenSSH subprocess/config integration versus an SSH library | Bounded experiment | `SSH-01`–`SSH-04`, `SEC-01`, `SEC-02` fixtures against pinned OpenSSH plus differential `ssh -G` resolution PoC, [ssh-and-remote.md](../research/ssh-and-remote.md) and [library-comparison.md](../research/library-comparison.md) section 8 |
| Whether a remote daemon is justified for Preview and its repository boundary | Explicit deferral | Daemonless OpenSSH is the required fallback and recovery path until `REM-01`–`REM-09` pass; persistence stays `Unknown` until `REM-02`, [ssh-and-remote.md](../research/ssh-and-remote.md) |
| Workspace persistence format, crash consistency, and migrations | Named design-council question → Milestone 1 RFC | Release gates `parity_layout_roundtrip`, `parity_ssh_workspace_restore`, `SSH-11` |
| Key protocol negotiation and exact pass-through escape semantics | Named design-council question → Milestone 1 RFC | Release gates `zellij_kkp_byte_trace`, `zellij_modify_other_keys_trace`, `noren_zellij_pass_through` |
| Trust boundary and permissions for agent hooks, OSC notifications, IPC, webhooks, and future plugins | Named design-council question → Milestone 1 threat model | Release gates `CORE-01`, `CORE-07`, `zellij_osc52_security`, section 9 adversarial IPC PoC |
| Preview MSRV and reproducible Rust toolchain installation/pinning | Explicit deferral until the first Rust experiment; design-council decides with recorded toolchain evidence | Blocking dependency recorded in R-PORT-02 |

## Maintenance and audit record

- Created under Issue #8 by the draft implementer (`Qwen`,
  `qwencloud/qwen3.8-max-preview`) against `main` at `b37126c`; integrator
  is `Codex`. No research was re-run and no upstream source was re-fetched;
  every citation points to a merged report or a stable GitHub artifact.
- Claim-to-source audit: each Evidence field was written from the cited
  section of the merged reports listed above; rows whose reports say a
  behavior is `Unknown` keep that label. The manual risk-to-gate audit is
  recorded in the Issue #8 handoff checkpoint comment.
- This register is a shared input required by the design council protocol
  review record ([protocol-codex-lab.md](../coordination/reviews/protocol-codex-lab.md))
  and must exist at the recorded evidence commit before Round 1 execution.
- Update rule: when a gate passes, a row is resolved, or Milestone 1
  evidence contradicts a rating, update the affected row and record the
  change here with date and PR link; do not silently re-rate.
