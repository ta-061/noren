# Milestone 0 risk register

Status: Milestone 0 integration artifact for
[Issue #8](https://github.com/ta-061/noren/issues/8), delivered as the
compressed checkpoint of Draft PR
[#14](https://github.com/ta-061/noren/pull/14). Created 2026-08-03
(Asia/Tokyo) against `main` at `b37126c` (merge of PR
[#12](https://github.com/ta-061/noren/pull/12)).

This register integrates already-merged Discovery evidence only. It
selects no architecture, library, or dependency and does not open the
production implementation gate ([D-0001](../coordination/decisions.md),
[design-process.md](../coordination/design-process.md)). Planned behavior
is not implemented behavior: every Noren row in the source matrices is
`Planned` or `Not planned`, and unresolved evidence stays `Unknown`.

Evidence base (merged reports, linked once here and abbreviated in the
tables below): [terminal-landscape.md](../research/terminal-landscape.md)
(`TL`) and [library-comparison.md](../research/library-comparison.md)
(`LC`) — Issue [#3](https://github.com/ta-061/noren/issues/3), PR
[#13](https://github.com/ta-061/noren/pull/13);
[cmux-parity.md](../compatibility/cmux-parity.md) (`CP`) and
[zellij.md](../compatibility/zellij.md) (`ZJ`) — Issue
[#4](https://github.com/ta-061/noren/issues/4), PR
[#11](https://github.com/ta-061/noren/pull/11);
[ssh-and-remote.md](../research/ssh-and-remote.md) (`SR`) and
[agent-integrations.md](../research/agent-integrations.md) (`AI`) — Issue
[#5](https://github.com/ta-061/noren/issues/5), PR
[#12](https://github.com/ta-061/noren/pull/12);
[project-principles.md](../project-principles.md) (`PP`),
[design-process.md](../coordination/design-process.md) (`DP`),
[open-questions.md](../coordination/open-questions.md) (`OQ`), historical agent
calibration evidence reviewed in PR [#14](https://github.com/ta-061/noren/pull/14)
(`AC`, no longer shipped in the product repository),
[status.md](../coordination/status.md) (`ST`).

## Independent reviews (assigned, not performed)

Per Issue #8: `codex-lab` reviews testability and gates; `Claude Code`
reviews security and maintainability; `Codex` integrates findings and
decides. No verdict is claimed in this draft; dissent and `Unknown` states
are preserved. SSH transport rows carry a deferred SSH design proposal
owner because SSH design work has not started and no agent is assigned to
it before Milestone 1.

## Scales and gate vocabulary

Ordinal `(L, I)` pairs drive screening prioritization; there is no numeric
score. Likelihood is the chance the risk materializes if unmitigated at
the current evidence level; impact is the worst credible outcome.

| L | Meaning | I | Meaning |
| --- | --- | --- | --- |
| L1 unlikely | No credible path in merged evidence | I1 negligible | Cosmetic or self-healing |
| L2 low | Path exists but requires several preconditions | I2 minor | Degraded experience, obvious workaround |
| L3 possible | Merged evidence shows a credible path | I3 moderate | A feature/workflow broken, no data loss or security exposure |
| L4 likely | Failure recurs in normal use if unaddressed | I4 major | Data loss, security exposure, or broken core workflow |
| L5 almost certain | Failure is proven by evidence unless behavior changes | I5 catastrophic | Irreversible loss, credential/system compromise, or a release violating project principles |

A **design gate** is a named design-council question in
[open-questions.md](../coordination/open-questions.md) (`OQ`) or a
Milestone 1 RFC/ADR prerequisite. A **release gate** is a named executable
test or fixture suite from the merged matrices (`SSH-01`–`SEC-06`,
`REM-01`–`REM-09`, `CORE-01`–`OC-07`, `zellij_*`, `parity_*`/
`candidate_*`); it passes only with current executed evidence on every
named target environment.

## Risk table

Rows merged from the first draft keep the envelope `(max L, max I)` of the
merged rows; no requirement field was dropped. Owners are evidence or
mitigation owners, not implementation assignments.

| ID | Category | Risk | Evidence | L | I | Owner | Mitigation | Trigger | Gate |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| R-IN-01 | Input loss | Shortcuts/presets/pass-through consume bytes owed to the child; unsettled key protocol negotiation (KKP/CSI-u/modifyOtherKeys/legacy) forwards wrong bytes or unprovable capabilities | PP rule 1; ZJ pass-through rows + `Z-2L` oracle + KKP matrix; TL lesson 4; AC pass-through finding | L3 | I5 | Design council; `codex-lab` byte oracles | Minimal interception manifest; every binding rebindable/disableable; pointer-reachable recovery; `$TERM`/brand never proves capability; per-protocol byte fixtures; mode restoration after exit and failure | M1 keybinding/pass-through spec; first parser/state PoC | Design: OQ key protocol + pass-through questions. Release: `zellij_*preset*`/`zellij_unlock_first_trace`/`noren_zellij_pass_through` on `Z-PROTO`+`Z-SSH`; `zellij_kkp_byte_trace`, `zellij_legacy_key_trace`, `zellij_modify_other_keys_trace` |
| R-DL-01 | Data loss | Persistence format, crash consistency, migration, or failed configuration reload loses or corrupts saved state or valid configuration | OQ persistence question; PP rule 4; LC §10 (reload safety is Noren-owned); CP `parity_layout_roundtrip`; SR SSH-11 | L3 | I4 | `Codex` integration; `codex-lab` evidence | Versioned format; atomic writes; last-valid state kept on failed reload; transactional activation with rollback; hostile-config fixtures; no keys/tokens/raw commands persisted | M1 persistence requirement; configuration schema definition | Design: OQ persistence question; schema/reload policy (Issue [#6](https://github.com/ta-061/noren/issues/6) scope). Release: `parity_layout_roundtrip`, `parity_ssh_workspace_restore`, SSH-11, LC §10 validating PoCs |
| R-SEC-01 | Security | SSH destination handling or agent resume triggers unintended local/remote command execution (option injection via leading dash, `%h`/`%n` token expansion to `Match exec`, shell metacharacters) | LC §8; SR SEC-01/02/04; AI CORE-06/13; PP rule 2 | L3 | I5 | Deferred SSH design owner; `Claude Code` review | Structured argv only; reject leading-dash destinations until a version-tested end-of-options contract; never concatenate shell commands; framed stdin to remote helpers; hostile-input property tests | Any work invoking `ssh`, building remote commands, or enabling resume | Design: OQ OpenSSH subprocess vs library. Release: SR SEC-01/02/04; AI CORE-06/13 |
| R-SEC-02 | Security | Untrusted terminal bytes/OSC spoof state, exfiltrate or overwrite clipboard, or invoke IPC; secrets, prompts, or protected input leak via logs, dumps, notifications, accessibility, telemetry | ZJ OSC 52/8 rows; AI CORE-05/07; TL lesson 5; PP rules 1+3; SR SEC-06; LC §9/§11/§14 | L4 | I5 | Design council trust policy; `Claude Code` review; `codex-lab` canaries | Bounded OSC; read queries denied; terminal bytes are data, never authority; IPC local-only, peer-authenticated, versioned; closed redacted event schema with field allowlists; canary credentials asserted absent; bounded a11y scrollback | Parser/state PoC; OSC policy; IPC design; first logging/notification/a11y implementation | Design: OQ trust boundary + permissions; M1 threat model (DP round 3). Release: `zellij_osc52_security`, `zellij_osc8_trace`, CORE-05/07, SEC-06, LC §9/§11/§14 PoCs |
| R-SSH-01 | SSH | Noren silently reinterprets OpenSSH config semantics (Host/Match/Include/ProxyJump precedence, pinned `russh-config` limits) or weakens host-key verification or identity/agent-forwarding policy, enabling wrong security settings or MITM | SR reinterpretation table + pinned `russh-config` limits + SSH-03 + SSH-05 + SEC-03; LC §7–§8; OQ host-key UI question | L4 | I5 | Deferred SSH design owner; `Claude Code` review; `codex-lab` differential fixtures | Agent forwarding stays default-off and only explicit, destination-scoped; destinations whose `IdentityAgent`/`IdentitiesOnly` or other semantics cannot be honored fall back to ordinary OpenSSH marked unsupported, never approximated; differential fixtures vs pinned `ssh -G` (kept redacted); strict known-hosts preserved; never auto-accept prompts; atomic persisted accepted keys; host-key mismatch never auto-retries | SSH transport/config resolver selection; embedded-transport PoC; first-use UI design | Design: OQ OpenSSH subprocess vs library; first-use host-key confirmation. Release: SR SSH-01–SSH-05, SEC-03; CP `parity_ssh_config_fixture` |
| R-SSH-02 | SSH | Reconnect logic misclassifies failures (exit-255 ambiguity, spoofable TCP keepalive, stale/colliding control sockets) and retries the wrong way or reports wrong state | SR SSH-06/07/08, REM-09, never-auto-retry rules; ZJ SSH segmentation unknowns | L3 | I4 | Deferred SSH design owner; `codex-lab` fault-injection fixtures | Separate transport and remote-session state; classify `Unknown` unless attributable; retry only classified transient failures with bounded, cancelable, jittered delays; multiplex reuse binds full destination/agent context | Reconnect state-machine design | Design: OQ daemon/remote-session question. Release: SR SSH-06/07/08, REM-09; CP `parity_ssh_reconnect` |
| R-AGT-01 | Agent trust | Agent state guessed from process names/text/forgeable OSC shows false status or fires wrong automation; hook/plugin trust bypass or silent install runs repository code or leaks prompts and session IDs | AI state contract "What is insufficient" + CORE-01/02/04/05/07/09 + Codex hook-trust findings; CP `parity_agent_state_trust`, `parity_project_launch` | L4 | I4 | Design council adapter requirements; `Claude Code` review; `codex-lab` fixtures | Version-scoped adapters with capability snapshots; source-provided ordering tokens/leases; no process-name scanning; unsolved states stay `Unknown` and suppress automation; explicit trust UI with exact-file/hash provenance; passive fail-open observer; per-event field allowlists | First adapter implementation; hook/plugin distribution; notification design | Design: OQ trust boundary + permissions; M1 adapter requirements. Release: AI CORE-01/02/04/05/07/09 + `CX-*`/`CL-*`/`OC-*` suites; CP `parity_agent_state_trust`, `parity_project_launch` |
| R-PORT-01 | Portability | macOS/Wayland/X11 input/windowing/IME/a11y divergence with a single window/event candidate; no Rust toolchain installed, so buildability and MSRV are unproven and every executable gate in this register is blocked | LC §3B gap + §6 IME coupling; TL unknowns; ZJ `Z-MAC`/`Z-WAYLAND`/`Z-X11`; ST blocked section; historical AC/inventory record (cargo/rustc/rustup absent at capture time) | L4 | I4 | Design council; human owner (ta-061) approves toolchain; `codex-lab` measurements | Install and pin a toolchain before the first compile experiment and record versions/targets; identical lifecycle/input/IME traces on AppKit, Wayland, X11 before any adoption ADR; second window candidate or recorded market-gap justification; X11 support is a product decision | M1 window/event selection; first experiment requiring compilation | Design: OQ stack question + MSRV/toolchain question. Prerequisite gate: Issue [#6](https://github.com/ta-061/noren/issues/6) approves/pins the Rust toolchain and MSRV, records exact rustc/cargo versions and installed targets, and the first implementation CI builds and tests the minimal workspace before compilation-dependent suites run. Release: LC §3B `winit` trace PoC; ZJ `zellij_japanese_ime` on `Z-MAC`/`Z-WAYLAND`/`Z-X11` |
| R-PERF-01 | Performance | Rendering, parsing, IME, or throughput claims are made without measurements and regressions are discovered late | TL lessons 1–2; ZJ large-output/scrollback unknown; AC rejected unmeasured microsecond claim | L4 | I3 | `codex-lab` measurements; `GLM` bounded proposals under review | M1 defines frozen latency/memory/throughput budgets as measurable NFRs; deterministic numbered/hashed corpora; p50/p95 frame time, idle CPU, resize latency per PoC; no claim without measurement | Renderer/parser PoCs; M1 NFR definition | Design: M1 non-functional requirements (DP round 3). Release: ZJ `zellij_large_output_soak` on `Z-LOAD`; LC §3A renderer measurements |
| R-DEP-01 | Dependencies/licenses | GPL references cross the MIT/Apache boundary or candidates advance with incomplete license/unsafe/advisory evidence; a single-candidate or unstable 0.x dependency reaches adoption | TL legal/clean-room boundary + lesson 6; CP legal boundary; LC method limits, §3B/§12B gaps, evidence-to-decision gate | L3 | I4 | `Codex` integration; `Claude Code` review; human legal line; design council gate | Specification-driven implementation, no translated upstream code or copied assets/marks; every PoC keeps lockfile, license/advisory scan dates, recursive unsafe inventory; second-candidate PoC or recorded justification before any adoption ADR; exact version + source-commit pinning | Any candidate advancing to PoC or adoption; any adoption ADR | Design: LC evidence-to-decision gate applied by the design council. Release: per-candidate license/advisory/unsafe inventory and per-category PoC/drop gates (LC) |
| R-A11Y-01 | Accessibility | Terminal-scale dynamic text is not actually reachable by screen readers, or the accessibility tree leaks protected input or consumes unbounded memory | LC §14 (AccessKit rich-text limits; a11y is a release gate); TL lesson 7 + explicit unknown | L4 | I4 | `Qwen` UI/accessibility; `Claude Code` review | Bounded semantic snapshot (grid, selection, focus, bounded scrollback) independent of renderer pixels; VoiceOver and AT-SPI tests for reading order, navigation, rapid updates, resize, IME preedit, protected input, memory, latency | Renderer/window selection (composes with R-PORT-01) | Design: OQ stack question. Release: LC §14 validating PoCs on macOS NSAccessibility + Linux AT-SPI |
| R-REL-01 | Release integrity | Unsigned, substitutable, replayed, or downgraded artifacts and update paths; planned features advertised as supported or claims outrunning executed evidence | LC §12/12A/12B; SR REM-07; OQ signing identity question; PP rules 2+5; CP/ZJ state legends; [ROADMAP.md](../../ROADMAP.md) | L4 | I5 | Human owner (ta-061) for any signing identity; `Codex` release gates; every document owner | Unsigned test artifacts first in a locked CI sandbox; independent signed manifest + digest before replacement; atomic install with health check and rollback; no automatic upload; keep `Planned`/`Supported` vocabulary with the CI documentation checker; Preview claims require executed evidence | Packaging PoC; Preview release planning; every documentation or release-notes change | Design: OQ signing/notarization identity; M1 release plan. Release: LC §12/§12B PoCs; SR REM-07; Milestone 8 claim-versus-evidence review (ROADMAP.md) |
| R-PTY-01 | Process/PTY | Cross-pane FD/input leakage, resize/EOF/signal/exit races, orphaned child, or child failure blocking/crashing the local UI | LC §2; PP security/reliability rule 5; CP `parity_pane_lifecycle` | L3 | I4 | Milestone 1 PTY design owner; `codex-lab` evidence | Spawn without shell; minimal/inheritable-descriptor control; explicit reader/writer/child ownership; failure isolation; idempotent shutdown/reap | PTY abstraction selection and first local spawn PoC | Design: OQ PTY abstraction/platform ownership. Release: LC §2 validating suite plus CP `parity_pane_lifecycle` |

## Architecture-changing unknowns and dispositions

Every open question that can change architecture maps below to a bounded
experiment, a named design-council/RFC question, or an explicit deferral.
No row is an answer; `Unknown` states persist until the referenced gates
pass (for example, remote PTY persistence stays `Unknown` until `REM-02`).

| Unknown (sources: OQ "Design council must decide", merged report unknowns) | Disposition | Bound |
| --- | --- | --- |
| Terminal parser/state library boundary and replacement strategy | Bounded experiment feeding the design council | LC §1 + §1A identical corpus + fuzz for `vte`/`vtparse`/`avt`/`alacritty_terminal` with drop gates |
| Window, GPU renderer, font shaping, IME, and accessibility stack | Bounded experiments feeding the design council | LC §3A/3B/4/6/14 PoCs with identical traces (Metal and Vulkan/GL); window gap requires a second candidate or recorded justification |
| PTY abstraction and platform-specific ownership | Bounded experiment | LC §2 `portable-pty` vs `nix` suite: spawn without shell, resize storms, EOF, signal/exit races, descriptor leakage, sanitizers |
| OpenSSH subprocess/config integration versus an SSH library | Bounded experiment | SR SSH-01–SSH-04, SEC-01/02 fixtures against pinned OpenSSH + differential `ssh -G` resolution PoC (LC §8) |
| Remote daemon justification for Preview and repository boundary | Explicit deferral | Daemonless OpenSSH is the required fallback until REM-01–REM-09 pass; persistence stays `Unknown` until REM-02 |
| Workspace persistence format, crash consistency, and migrations | Named design-council question → M1 RFC | Release gates `parity_layout_roundtrip`, `parity_ssh_workspace_restore`, SSH-11 |
| Key protocol negotiation and exact pass-through escape semantics | Named design-council question → M1 RFC | Release gates `zellij_kkp_byte_trace`, `zellij_modify_other_keys_trace`, `noren_zellij_pass_through` |
| Trust boundary and permissions (agent hooks, OSC, IPC, webhooks, plugins) | Named design-council question → M1 threat model | Release gates CORE-01, CORE-07, `zellij_osc52_security`, LC §9 adversarial IPC PoC |
| Preview MSRV and reproducible Rust toolchain installation/pinning | Named design-council question → Issue [#6](https://github.com/ta-061/noren/issues/6) toolchain/MSRV decision | Yields the R-PORT-01 prerequisite gate: approved/pinned toolchain and MSRV, recorded exact rustc/cargo versions and installed targets, first implementation CI builds and tests the minimal workspace before compilation-dependent suites run; no version selected here, nothing installed |

## Maintenance

Created under Issue #8 by the draft implementer (`Qwen`,
`qwencloud/qwen3.8-max-preview`); integrator is `Codex`. No research was
re-run and no upstream source re-fetched; every citation points to a
merged report or a stable GitHub artifact, and rows whose reports say
`Unknown` keep that label. This register is the shared input required by
the design council protocol review record
([protocol-codex-lab.md](../coordination/reviews/protocol-codex-lab.md))
and must exist at the recorded evidence commit before Round 1 execution.
Update rule: when a gate passes, a row is resolved, or Milestone 1
evidence contradicts a rating, update the affected row and record the
change here with date and PR link; do not silently re-rate.
