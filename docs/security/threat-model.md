# Threat model: first macOS local-PTY PoC

- Status: Accepted for scoped implementation; requires Claude Code review
- Date: 2026-08-03
- Scope: FR-001 through FR-007 in
  [v0.1 requirements](../requirements/v0.1.md)

## Assets and trust boundaries

The assets are user keystrokes, local environment values, current directory,
terminal contents, child-process lifetime, UI availability, and process/file
descriptor integrity. The user and reviewed Noren code are trusted to initiate
the local session. PTY output, control sequences, terminal replies, fonts,
window events, dependency/native boundaries, and the child after spawn are
untrusted inputs. Local privilege escalation, malware already running as the
same user, SSH, IPC, plugins, and release signing are outside this PoC model.

## Threats and required controls

| ID | Threat | Required control | Verification |
| --- | --- | --- | --- |
| TM-01 | Shell injection or executing a different program through concatenated input. | Spawn fixed `/bin/zsh` with structured argv through `PtyBackend`; no `sh -c`, `zsh -c`, interpolation, or user command field. Use an explicit cwd and documented environment inheritance policy. | Unit inspection of the spawn request plus a helper that records argv/cwd; Claude boundary review. |
| TM-02 | Descriptor inheritance, controlling-terminal mistakes, orphan, zombie, or signal race. | Confine handles to the PTY supervisor/reader; close unused ends; make Close idempotent; kill only an unreaped owned child; wait and join on every path. | Descriptor snapshots, controlling-TTY assertion, EOF/exit/signal races, repeated-close test, post-test process scan. |
| TM-03 | Child output or OSC causes clipboard, filesystem, IPC, network, command, notification, or credential action. | Treat bytes as data. `TerminalEngine` returns bounded metadata only; disable every authority-bearing OSC/action in the PoC and never execute terminal replies as local actions. | Hostile OSC corpus asserts no authority calls and bounded state. |
| TM-04 | Malformed or high-rate terminal output exhausts CPU/memory or freezes the UI. | Bound read chunks, channels, per-turn drain, snapshots, title/OSC fields, glyph atlas, and grid dimensions; apply backpressure and retain typed failure paths. | 100 MiB burst, malformed corpus, resize storm, memory/latency measurements, fuzz/no-panic target. |
| TM-05 | Keystrokes are lost, duplicated, transformed, logged, or mistaken for trusted synthetic activation. | Encode only documented pressed events; drop unsupported Cmd/IME/dead-key paths explicitly; keep provenance/timestamp in app events; never log input bytes. | Exact-byte fixtures including repeats/releases/modifiers and log-capture assertions. |
| TM-06 | Resize arithmetic underflows/overflows or sends invalid dimensions to kernel/parser/renderer. | Use checked conversion to bounded non-zero `u16` rows/columns; skip zero-size surfaces; coalesce duplicates; apply one final consistent size. | Boundary/property tests, zero-size trace, `TIOCGWINSZ` final-size oracle. |
| TM-07 | Parser, font, GPU, or native-library defect crosses a broad privilege boundary or crashes teardown. | Keep replaceable contracts narrow; minimize features and `unsafe`; require `SAFETY` comments; treat device loss and malformed font as recoverable; retain a teardown path independent of renderer. | Dependency/feature inventory, Clippy, malformed-font fixtures, injected renderer/parser failures. |
| TM-08 | Logs, panic reports, snapshots, CI artifacts, or review comments disclose terminal contents, environment values, secrets, or private paths. | Log only component/operation/status and coarse sizes; redact cwd to an opt-in diagnostic; do not persist terminal frames or input outside local test fixtures. | Secret-pattern check plus a test logger that rejects known sentinel terminal/input/environment values. |
| TM-09 | A full output channel or blocked reader prevents close forever. | Receiver disconnect must wake blocked send; closing/reaping the child must cause reader EOF; supervisor uses bounded status polling and a 2 s shutdown deadline with reported forced termination. | Fill-channel close test, blocked-read close test, deadline assertion, worker-join assertion. |

## Security invariants

1. No terminal-controlled byte obtains local authority in the PoC.
2. No external value is concatenated into a command line.
3. Noren owns and reaps exactly the child it created; shutdown may be repeated.
4. Untrusted allocations and dimensions have explicit bounds.
5. Terminal payloads, keystrokes, environment values, and secrets do not enter
   normal logs or committed artifacts.
6. Expected hostile input and component failure produce errors, not panics.

## Residual risk and cut line

`portable-pty`, AppKit/winit, wgpu/Metal, the parser, and font code retain native
and transitive-unsafe risk until dependency inventory and executable tests run.
The PoC is local and inherits the user's environment, so a child can access the
same-user resources that `/bin/zsh` normally can; Noren is not a sandbox. Full
terminal protocol coverage, paste/clipboard, hyperlinks, notifications, IME,
accessibility, SSH/agent credentials, IPC, persistence, packaging, and updates
require new threat-model sections before implementation.
