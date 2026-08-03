# CLI agent integration research

Status: evidence report; no adapter/API adoption decision

Issue: [#5](https://github.com/ta-061/noren/issues/5)

Evidence retrieved: 2026-08-03

Repository baseline: `7cd56c689546bcfc38f083551813abe32e48469f`

Review artifacts: [PR #12 discussion](https://github.com/ta-061/noren/pull/12)
and the [2026-08-03 Claude follow-up](https://github.com/ta-061/noren/pull/12#issuecomment-5161076007).

## Scope and evidence rule

Noren is intended to host existing CLI agents, not embed an LLM or infer an
agent's intent from terminal text. This report evaluates status and session
signals for Codex, Claude Code, and OpenCode in the required order:

1. official hook;
2. official plugin/API;
3. structured output;
4. explicit shell integration;
5. OSC notification;
6. process information.

A higher-priority signal is used only when it actually proves the state. Hook
delivery proves that a documented lifecycle point was reached; it does not
automatically prove that all other hooks finished, a prompt is currently on
screen, or a turn cannot continue. Process name alone never establishes agent
identity or semantic state. Missing, stale, conflicting, version-incompatible,
or undocumented evidence maps to `Unknown`.

This report does not choose an adapter architecture, alter agent settings,
install hooks/plugins, launch authenticated tasks, or define a public plugin
API.

## Versioned local evidence

The repository stores sanitized, reproducible `--help` captures:

| Agent | Local executable evidence | Version inspected | Relevant locally advertised capabilities |
| --- | --- | --- | --- |
| Codex | [codex.txt](../coordination/cli-help/codex.txt) plus exact [`exec resume --help`](../coordination/cli-help/codex-lab-exec-resume.txt) | `codex-cli 0.146.0` | hook-trust bypass flag (negative evidence only); plugins; `exec --json`; output schema/file; top-level resume and exact `exec resume` argv/flags contract; experimental app server |
| Claude Code | [claude.txt](../coordination/cli-help/claude.txt) | `2.1.220` | hooks/plugins; `--output-format json/stream-json`; `--include-hook-events`; JSON schema; session/resume |
| OpenCode | Current corrected [opencode.txt](../coordination/cli-help/opencode.txt), including selected and shadowed artifact provenance | Selected `1.18.11`; shadowed npm-global `1.14.31` also inventoried | plugins; `run --format json`; serve/attach; ACP; session continuation |

Authentication checks in those captures are redacted. This document does not
repeat credential locations or account/provider details.

### OpenCode executable provenance correction

An absolute-path re-inventory on 2026-08-03 found the intended executable plus
a separate, rejected global-npm launcher/payload chain:

| Sanitized canonical label | Sanitized path and provenance | Version or role | SHA-256 of that artifact |
| --- | --- | --- | --- |
| `user-local-opencode` (intended) | `~/.opencode/bin/opencode` | `--version`: `1.18.11` | `f554a08dee4c34f4f43df63af72f0a6afbe57f955496853f411767718927bf2c` |
| `global-npm-opencode-launcher` (rejected/shadowing) | `/opt/homebrew/bin/opencode` is a symlink to `../lib/node_modules/opencode-ai/bin/opencode` | Global npm package `opencode-ai@1.14.31`; Node.js launcher; `--version`: `1.14.31` | `3ab08cfdb3cf1213eaeae45f557fb3220e0999862d8dc90eb17ba4cacf97c57b` for the resolved JS launcher |
| `global-npm-opencode-native` (rejected payload) | `/opt/homebrew/lib/node_modules/opencode-ai/node_modules/opencode-darwin-arm64/bin/opencode` | arm64 native payload selected by the `1.14.31` launcher | `40d5686fc86e94f833ac3e5855e12802464f2de0bcc1616013c62844bf6996d4` for the native payload |

PATH ordering varies by execution and launch context. In one captured
research-worktree shell, `type -a opencode` listed the rejected global npm
symlink under the Homebrew prefix first. In the calibration shell, it listed
the intended user-local `1.18.11` file first. The npm artifact is not
Homebrew-managed: `npm list -g` reports `opencode-ai@1.14.31`, while
`brew info opencode` reports the formula as not installed. The prefix alone is
therefore not package-manager provenance, and neither PATH ordering proves
executable identity. The current stored capture is the corrected, current
local evidence: it records the selected user-local file and digest, both
shadowed npm artifacts and digests, and the package-manager check. Its
absolute-path `--version`, `--help`, and `run --help` revalidation uses the
intended user-local file and supports the capabilities summarized here without
querying credentials. The original pre-PR #10 evidence incorrectly paired
`/opt/homebrew/bin/opencode` with `1.18.11`; PR #10 corrected that historical
error, so it is not a competing current authority.

For this report and every A-MAC OpenCode/help fixture, “OpenCode 1.18.11” means
the resolved absolute file represented by the
`user-local-opencode` label **and** the full digest above. A bare PATH launch,
version-only match, or either artifact in the shadowing global npm chain fails
provenance and leaves the adapter disabled/`Unknown` until explicitly
corrected. Because PATH ordering changes with launch context, every evidence
capture and automation fixture must use the approved absolute path. The
implementation must retain that absolute executable/argv internally while
displaying only the sanitized label; it must re-check identity around
capability probing and launch to detect replacement (OC-00).

The Codex and Claude documentation sites are live documents and do not expose a
commit that guarantees correspondence to the installed binary. Their mappings
therefore remain version-gated experiments. OpenCode
[`v1.18.11`](https://github.com/anomalyco/opencode/releases/tag/v1.18.11) was
the current immutable release (2026-08-01) and resolves to commit
`012c2f57f976489d88bd4598a056b4bdcdd428ee`; its generated SDK types were
inspected at that commit.

## Normalized state and event contract

The following vocabulary is proposed only to make tests precise:

| Noren value | Meaning that must be proven | What is insufficient |
| --- | --- | --- |
| `Running` | A known session has begun or resumed a turn and no later conclusive transition supersedes it. | A matching process name, recent terminal output, or an old start hook. |
| `AwaitingApproval` | A documented approval request for the known session is presently unresolved. | A pre-tool event or hook that may itself approve/deny. |
| `AwaitingInput` | A documented user-input request, or a completed response explicitly waiting for the next prompt, is presently unresolved. | A quiet PTY, shell prompt heuristic, or “stop candidate” hook. |
| `Completed` | A conclusive successful turn/response completion transition. It can emit a notification and then settle at `AwaitingInput`. | Process exit alone, session end, a hook that another hook can reverse, or a return-to-idle notification that only proves the prompt is free after a cancelled or errored turn. |
| `Error` | The agent's documented terminal error transition for this turn/session. | An observer-hook failure, nonzero child tool exit, or SSH loss alone. |
| `ProcessExited` | The supervised OS process exited. This is a notification/lifecycle fact, not successful completion. | Process disappearance found by name scanning. |
| `Unknown` | No fresh, mutually consistent documented signal proves another value. | Guessing from elapsed time, text, color, title, or executable basename. |

Each semantic event carries an adapter ID, version/capability snapshot, opaque
session correlation key, pane/host identity, a source-provided ordering token
(sequence number, turn/monotonic counter, or equivalent), source class, and a
local monotonic receive timestamp. State is rejected if the session/pane does
not match, the ordering token regresses, or its source lease expires.

Ordering between two events is trusted only when a source-provided
sequence/turn token orders them, or when the delivery channel is documented to
serialize them (for example, a single in-order stream). Local receive time
alone does not order concurrently delivered events: when matching hooks or
plugins run in parallel, receive order reflects scheduling, not causality. If
the relative order of two events would change the emitted transition and no
sequence/turn token or proven serialized channel establishes it, the affected
transition is `Unknown` rather than a guessed `Completed`/`Error` (CORE-02).

`Completed` and `Error` are transitions suitable for notifications; a
still-open interactive agent can subsequently be `AwaitingInput`.

“Long unresponsive” is a timer alert, not a semantic state. It may say that no
trusted event arrived for a configured interval while the process/transport was
live, but the agent state becomes `Unknown`, not `Error` (CORE-10).

## Codex

### Current official surfaces

The current [Codex hooks guide](https://developers.openai.com/codex/hooks)
documents `SessionStart`, `UserPromptSubmit`, `PreToolUse`,
`PermissionRequest`, `PostToolUse`, `Stop`, `SessionEnd`, and subagent
and compaction events. Common hook input includes `session_id`,
`transcript_path`, `cwd`, event name, and model; turn-scoped events add a
turn ID. The transcript format is explicitly not a stable hook interface.
These lifecycle points are derived from the official documentation, not
attested by the local help capture, and remain exact-version-gated by CX-01.

Matching hook commands from multiple sources run concurrently. Non-managed
hooks require trust for their exact definition, and plugins can bundle the same
lifecycle hooks. A Noren observer must be passive, bounded, and separately
trusted; it cannot assume it runs last. The local `0.146.0` CLI advertises
`--dangerously-bypass-hook-trust`, but Noren must not use a dangerous trust
bypass as an installation shortcut. Plugin packaging is documented by the
[official plugin guide](https://developers.openai.com/plugins/build/plugins).

The [non-interactive guide](https://developers.openai.com/codex/noninteractive)
documents `codex exec --json` as JSONL with events including
`thread.started`, `turn.started`, `turn.completed`, `turn.failed`,
`item.*`, and `error`. This is a strong source only for a Noren-launched
non-interactive run whose stdout remains reserved for that stream.

The [Codex app-server protocol](https://developers.openai.com/codex/app-server)
documents turn lifecycle notifications and explicit command/file approval and
user-input server requests. It can prove unresolved requests when Noren is the
protocol client. However, the local `0.146.0` help labels `app-server`
experimental, so it is a separately gated candidate, not a compatibility
baseline for ordinary interactive CLI panes.

### Version-scoped mapping

| Signal | Candidate Noren transition | Confidence and limitation | Gate |
| --- | --- | --- | --- |
| Hook `UserPromptSubmit`; activity hooks | `Running` with a short renewable lease | Official lifecycle point. A dropped observer or later unobserved transition makes it stale, so lease expiry becomes `Unknown`. | CX-01, CORE-02 |
| Hook `PermissionRequest` | Approval-request candidate only | The hook can allow/deny and other matching hooks run concurrently. It does not prove that a user-visible request remains unresolved. Do not expose `AwaitingApproval` from this alone. | CX-02 |
| Hook `Stop` | Completion candidate only | A Stop hook can continue/stop behavior and another matching hook may alter the aggregate result. Wait for a conclusive source; otherwise `Unknown`. | CX-03 |
| Hook `SessionEnd` | Session closed/idle cleanup | It can fire on archive/delete, normal close, or extended idle and currently reports only `other`; it is not task completion. | CX-04 |
| `exec --json` `turn.started` | `Running` | Strong for the supervised exec invocation; not evidence for a separate interactive pane. | CX-05 |
| `exec --json` `turn.completed` | emit `Completed` | Terminal for that exec turn, correlated to the supervised stream. | CX-05 |
| `exec --json` `turn.failed` or terminal `error` | emit `Error` | Schema/order/exit discrepancies become `Unknown` plus adapter diagnostic. | CX-05, CORE-03 |
| Experimental app-server `turn/started`, `turn/completed` | `Running`; then `Completed` or `Error` from documented final status | Semantically rich but version/feature gated. A completed turn can be `completed`, `interrupted`, or `failed`; these must not collapse to success. | CX-06 |
| Experimental app-server approval request until `serverRequest/resolved` | `AwaitingApproval` | Strong only while Noren owns that initialized connection/request. | CX-06 |
| Experimental app-server `item/tool/requestUserInput` until resolved | `AwaitingInput` | Strong only for the protocol session; interruption/turn completion can clear it. | CX-06 |
| Supervised OS exit | `ProcessExited`; semantic state `Unknown` unless a terminal event was seen | Exit code is corroboration, not a replacement for the event stream. | CORE-04 |

For a stock interactive Codex `0.146.0` pane with only a passive hook, a
durable `AwaitingApproval`, `AwaitingInput`, and `Completed` mapping is
not yet established. Those states remain `Unknown` after candidate hooks
unless CX-02/CX-03 finds a documented, observable follow-up.

## Claude Code

### Current official surfaces

The [Claude Code hooks reference](https://code.claude.com/docs/en/hooks)
documents lifecycle hooks and, importantly, passive `Notification` types:

- `permission_prompt`: Claude needs approval for tool use;
- `idle_prompt`: Claude is done and waiting for the next prompt;
- `agent_needs_input`: a background session waits for input; and
- `agent_completed`: a background session finishes or fails.

The last two require Claude Code `2.1.198+` and fire only while the agent view
is open. `agent_completed` deliberately combines success and failure, so it
does not prove either `Completed` or `Error` by itself. The inspected
`agent_needs_input` payload carries no child/agent identifier, so with more
than one concurrent background agent it cannot be attributed to a specific
session and stays `Unknown` rather than marking one waiting (CL-06).
Notification hooks cannot block or modify the notification, which makes
`permission_prompt` and `idle_prompt` stronger observation points than
`PermissionRequest` or `Stop`.

`Stop` runs when Claude finishes responding, but Stop hooks can feed back and
continue the loop. `StopFailure` runs instead when a turn ends because of an
API error and supplies a documented error category. `SessionEnd` reasons
include clear/resume/logout/input-exit/bypass-disabled/other and do not mean
that the requested task completed.

Claude plugins can bundle hooks, as described in the
[plugin guide](https://code.claude.com/docs/en/plugins), but the inspected
official plugin documentation did not establish a separate status API stronger
than hooks. This is a documented evidence gap, not a claim that none can exist.

The [programmatic/headless guide](https://code.claude.com/docs/en/headless)
documents `text`, single-result `json`, and JSONL `stream-json` output.
The last stream record is a `result` message. The local `2.1.220` CLI also
advertises `--include-hook-events` with stream JSON, a JSON schema, explicit
session ID, and resume.

### Version-scoped mapping

| Signal | Candidate Noren transition | Confidence and limitation | Gate |
| --- | --- | --- | --- |
| Hook `UserPromptSubmit` and subsequent activity | `Running` with lease | Official start point; dropped/stale observer becomes `Unknown`. | CL-01, CORE-02 |
| Notification `permission_prompt` | `AwaitingApproval` until a later correlated activity/stop/idle event | Passive official notification explicitly says approval is needed. Event ordering and duplicate behavior still require fixture validation. | CL-02 |
| Notification `idle_prompt` | `AwaitingInput`; emit `Completed` first only when the immediately preceding correlated turn is known to have completed successfully | The notification proves the prompt is free, not that the last turn succeeded. Do not emit `Completed` for an initial idle process with no preceding turn, or when the preceding turn was cancelled or ended via `StopFailure`/error; those cases settle at `AwaitingInput` (or `Error` first) without a success notification. | CL-03 |
| Hook `PermissionRequest` | Approval-request candidate only | A hook can return a decision; it does not alone prove unresolved UI wait. | CL-02 |
| Hook `Stop` | Completion candidate only | Another Stop hook can continue the loop. Correlate to `idle_prompt` or structured terminal result. | CL-04 |
| Hook `StopFailure` | emit `Error` with allowlisted category | Official terminal API-error hook; never persist `error_details` or last message without redaction. Tool failure is a different event. | CL-05, CORE-05 |
| Notification `agent_needs_input` | `Unknown` for the ambiguous background session; do not emit a per-session `AwaitingInput` | Requires 2.1.198+ and an open agent view; absence is not evidence. The inspected notification carries no child/agent ID, so with two or more concurrent background agents it cannot be correlated to a specific session and must not mark the wrong one waiting. Only a documented correlating identifier upgrades it beyond `Unknown`. | CL-06 |
| Notification `agent_completed` | terminal-candidate, then inspect a stronger result; otherwise `Unknown` | The docs say “finishes or fails,” so outcome cannot be inferred. | CL-06 |
| Headless JSON/stream final `result` plus exit status | emit `Completed` or `Error` according to documented result/exit fields | Strong for the supervised `-p` process. Truncated/missing/conflicting final data becomes `Unknown`. | CL-07 |
| `system/api_retry` stream event | remain `Running` with retry detail | A retry is not terminal `Error`; alert only if product policy later requests it. | CL-07 |
| Supervised OS exit | `ProcessExited`; semantic state depends on terminal signal | SIGTERM and nonzero exit are lifecycle/error evidence but do not manufacture a successful result. | CORE-04 |

## OpenCode

### Current official surfaces

OpenCode's [plugin documentation](https://opencode.ai/docs/plugins/) lists
`permission.asked`, `permission.replied`, `session.error`,
`session.idle`, `session.status`, and tool/message events. It also states
that all loaded plugin hooks run in sequence. The official notification example
uses `session.idle` as session completion.

At the pinned `v1.18.11` commit, the
[generated SDK types](https://github.com/anomalyco/opencode/blob/012c2f57f976489d88bd4598a056b4bdcdd428ee/packages/sdk/js/src/v2/gen/types.gen.ts)
define `SessionStatus` as `idle`, `busy`, or `retry` (retry carries
attempt/message/next fields), and define `permission.asked/replied`,
`question.asked/replied/rejected`, `session.error`,
`session.status`, and `session.idle` event payloads. The implementation
publishes `session.idle` when status is set to idle; see the
[pinned status source](https://github.com/anomalyco/opencode/blob/012c2f57f976489d88bd4598a056b4bdcdd428ee/packages/opencode/src/session/status.ts).

The [server documentation](https://opencode.ai/docs/server/) exposes an OpenAPI
3.1 document, an SSE event stream, session status queries, and health/version.
The default listen address is loopback. Basic authentication is optional when a
password environment variable is set. If evaluated remotely, the server must
remain loopback-only and cross SSH through a private tunnel; this report does
not approve a LAN/public bind or storing the password.

The [CLI documentation](https://opencode.ai/docs/cli/) and local help document
`opencode run --format json` as raw JSON events, plus `serve`, `attach`,
session continuation, and ACP. The CLI page does not freeze the full raw-event
schema, so a versioned fixture must capture it before structured output becomes
a contract. OpenCode source at the inspected release is
[MIT licensed](https://github.com/anomalyco/opencode/blob/012c2f57f976489d88bd4598a056b4bdcdd428ee/LICENSE).

Initialization and reconnect start at `Unknown`. The inspected server
documentation and pinned types provide no documented event cursor/epoch or
atomic snapshot-subscription handshake. A candidate can establish the
subscription and buffer events before requesting a snapshot, then reconcile
the buffer, but it cannot claim gap-free continuity without such a protocol
guarantee or a separately specified adapter handshake. If continuity cannot be
proved, the snapshot is advisory and status remains `Unknown` until a fresh,
correlated post-subscription event arrives (OC-01, OC-07).

### Version-scoped mapping

| Signal | Candidate Noren transition | Confidence and limitation | Gate |
| --- | --- | --- | --- |
| Fresh post-subscription `session.status` with `busy` | `Running` | Official typed event for the correlated session. An initial snapshot cannot independently prove stream continuity. | OC-01 |
| `permission.asked` until correlated `permission.replied` | `AwaitingApproval` | Official request/reply IDs; never expose patterns/metadata as trusted display or log data. | OC-02 |
| `question.asked` until reply/reject | `AwaitingInput` | Present in pinned generated types. Current plugin page omits question events from its summary, so require pinned version capability discovery. | OC-03 |
| `session.idle` after known active status | emit `Completed`, then `AwaitingInput`, only when no correlated `session.error` or abort preceded this idle for the same turn | Official docs use it for completion, but idle is also the resting state after a failed or aborted turn. Initial/default idle without an observed active turn does not prove a completed turn, and idle following a correlated `session.error`/abort settles at `AwaitingInput` (after `Error`) without a success notification. | OC-04, OC-05 |
| `session.status` with `retry` | remain `Running` with retry detail | Retry is not terminal error. Message content is untrusted/sensitive and must be redacted or omitted. | OC-05 |
| `session.error` | emit `Error` when session correlation is present | The pinned type permits an absent session ID/error. An uncorrelated event is an adapter diagnostic and affected sessions become `Unknown`. | OC-05 |
| `run --format json` raw events | Only transitions proven by the captured 1.18.11 schema | Officially structured, but exact events/order are not frozen on the CLI page. No text inference. | OC-06 |
| SSE disconnect/server restart/plugin failure | `Unknown` immediately for stream-derived semantic state | Re-subscribe, buffer, snapshot, and reconcile; without cursor/epoch/handshake continuity, wait for a fresh post-subscription event. Never retain stale busy/wait state. | OC-07 |
| Supervised OS exit | `ProcessExited`; no automatic success | Applies to TUI/run/server processes actually launched and owned by Noren. | CORE-04 |

## Metadata, resume, and remote correlation

Agent name comes from the selected launcher/adapter, not executable scanning.
Host/local-vs-SSH and pane come from Noren's own workspace graph. CWD, Git
repository, branch, session ID, and resume command have separate trust rules:

| Field | Candidate source | Rule |
| --- | --- | --- |
| Agent/version | Exact launcher path plus `--version` capability probe | Cache per executable identity; a changed binary invalidates capability mappings. |
| CWD | Official hook/API field for the correlated session; otherwise Noren's spawn CWD | Agent-reported remote paths are display data only. Shell child `cd` may not update the agent, and terminal-title text is not authoritative. |
| Git repository/branch | Noren queries the scoped local/remote workspace repository | Never accept a hook string as authorization for filesystem or Git actions. |
| Session ID | Official hook/API/structured event | Keep an opaque in-memory correlation value; redact from logs, notifications, telemetry, and URLs. Persist only after a separate storage threat decision. |
| Resume command | Adapter template plus a version-verified session argument | Never render a raw shell string for automatic execution. Store executable and argv fields separately and require explicit user action. |
| Last activity | Monotonic receipt time of a validated event or terminal input/output | It is not proof of `Running`; terminal bytes are untrusted and can be generated by unrelated children. |

For Codex, the repository's exact
[`exec resume --help` capture](../coordination/cli-help/codex-lab-exec-resume.txt)
records the `0.146.0` argv/flags contract. Its provenance is valid for this
report because the [capture inventory](../coordination/cli-help/codex-lab.txt)
shows that the isolated-`CODEX_HOME` `codex-lab` wrapper resolves the same
`/opt/homebrew/bin/codex` executable and `codex-cli 0.146.0` version as the
direct [Codex evidence](../coordination/cli-help/codex.txt). The optional Codex
`SESSION_ID` domain is a UUID or thread name; when the value parses as a UUID,
UUID interpretation takes precedence. Claude `--resume` and OpenCode
`--session/--continue` are present in their local evidence. Session scope,
retention, remote path behavior, and behavior after upgrades are not assumed.
CORE-06 must pin the exact installed version's resume help/argv contract before
enabling each resume adapter; the stored capture satisfies that documentary
gate for this Codex executable/version, and any identity or version change
invalidates it. CORE-06 verifies valid correlated-session resume. Valid-looking
thread names and hostile or malformed session IDs/arguments are exercised
separately under CORE-13 so that a green valid-resume path is never mistaken for
injection safety or UUID-only validation. Noren must not surface session IDs in
OS notifications or webhook payloads.

For an SSH pane, event correlation must originate on the machine where the
agent runs and travel over the authenticated workspace channel. A remote hook
must not dial a new public listener. Candidate transports are a bounded frame
on a Noren-owned helper channel or a user-owned Unix socket carried by SSH.
Terminal OSC is forgeable by any remote process and cannot upgrade an
`Unknown` state unless a future authenticated, nonce-bound protocol is
specified and tested.

## Notification consequences

Only a validated state transition can trigger approval-wait, input-wait,
completion, or error notification. A supervised exit triggers
`ProcessExited`; the inactivity timer triggers long-unresponsive without
claiming an error. Deduplicate by adapter/session/turn/request/event identity,
and rate-limit repeated wait/retry events (CORE-10, CORE-11).

The Noren notification center can retain an internal opaque workspace/pane
locator. OS notification text is generic and contains no prompt, command, CWD,
repository, branch, host alias, session ID, or provider detail. Clicking it or
using the notification shortcut resolves the internal locator and focuses the
exact local or SSH pane; stale/deleted locators fail safely (CORE-11).

Local-hook and webhook sinks are future opt-in outputs. They receive a
versioned, minimal event allowlist after redaction, use bounded queues and
timeouts, and cannot return a shell command or state mutation. Slack/Discord
execution is outside this report (CORE-12).

## Fallback behavior

- If a compatible hook/plugin/API is not installed or trusted, the agent still
  runs as an ordinary terminal program and status is `Unknown`.
- If structured mode was explicitly selected and its parser fails, preserve
  the raw terminal session for the user, disable semantic notifications for the
  affected session, and report an adapter diagnostic without dumping payloads.
- Explicit shell integration can declare launcher identity and process
  ownership, but it cannot declare completion/approval/input state without a
  documented adapter event.
- OSC may request a generic notification, subject to terminal security policy;
  it is not agent-state evidence.
- A supervised child exit can emit `ProcessExited`. Searching the process
  table for `codex`, `claude`, or `opencode` cannot.
- Conflicting sources use the highest-priority *conclusive and fresh* event.
  If correlation or ordering cannot resolve the conflict, use `Unknown` and
  suppress completion/error automation.

## Threat boundaries

| Boundary | Threat | Required control | Gate |
| --- | --- | --- | --- |
| Workspace hook/plugin configuration | Repository code executes as the user or forges state. | Explicit trust/install UI, exact-file/hash provenance, passive observer, no automatic dangerous bypass, removal path. | CORE-01 |
| Hook/plugin payload to Noren | Prompt, command, tool input, transcript path, provider error, session ID, or secret reaches logs/notifications. | Strict per-event field allowlist, size/type limits, redaction before formatting, no raw payload logging. | CORE-05 |
| Event stream to state machine | Replay, duplicate, out-of-order, concurrently delivered, cross-session, stale, or snapshot/subscription-gap event creates false status. | Pane/session/source binding, ordering by a source sequence/turn token or a proven serialized channel rather than receive time, idempotence, lease expiry, buffered reconciliation only with documented continuity; otherwise `Unknown`. | CORE-02, CORE-03, OC-01 |
| Terminal content/OSC to UI | Agent or remote program spoofs trusted state/notification or escapes display. | Treat bytes as untrusted terminal data; sanitize notification text; OSC never supplies authoritative agent state. | CORE-07 |
| Adapter to process launcher | Resume command/ID/CWD becomes shell injection or resumes the wrong session. | Executable plus argv representation, no shell, typed fields, explicit user action; CORE-06 verifies successful correlated resume, while CORE-13 covers the documented thread-name domain and hostile-ID/argv handling without conflating either with UUID-only validation. | CORE-06, CORE-13 |
| Local/remote adapter channel | Other user/process injects events or reads session metadata. | User-only endpoint, peer ownership/authentication, SSH transport remotely, fresh nonce, bounded frames. | CORE-08 |
| Agent server/API | Optional server is exposed or unauthenticated. | Loopback/Unix socket, SSH tunnel, credentials outside logs/config exports, version/health check, least privilege. | OC-07 |
| Observer failure | Hook timeout/crash blocks or changes the agent. | Passive success response, tight resource bounds, fail-open for observation only, observer failure reported separately from agent `Error`. | CORE-09 |
| Notification sink | Webhook/local hook leaks metadata, replays, blocks panes, or turns a response into execution. | Opt in; minimal redacted schema; bounded async queue; destination policy; no response-driven command/state. | CORE-12 |

## Executable validation matrix

Tests use harmless prompts and tools against disposable repositories. Hook and
event fixtures contain unique fake canaries, never real credentials. A test
must not change the developer's global agent configuration.

Target environments:

- **A-MAC**: the observed macOS 26.4.1 (build 25E253) arm64 host with the
  stored local versions: Codex `0.146.0`, Claude Code `2.1.220`, and the
  exact `user-local-opencode` `1.18.11` file/digest recorded above. Every
  OpenCode help/test command uses its resolved absolute path, never bare PATH.
- **A-LNX**: Ubuntu 24.04 LTS x86_64 with those exact agent versions installed
  in an isolated home/config directory; record the base-image digest.
- **A-SSH**: A-MAC client to an A-LNX agent over the authenticated SSH/helper
  test channel; no public event listener.
- **A-PTY**: direct shell, tmux, and Zellij PTYs on A-MAC and A-LNX.

| ID | State/failure exercised | Executable assertion | Environment |
| --- | --- | --- | --- |
| OC-00 | OpenCode PATH-order variation, package provenance, launcher/payload digest mismatch, and replacement between probe/launch | Run isolated bare-PATH fixtures with the global npm symlink first and with the user-local file first; reject bare-command provenance in both. The npm-first inventory attributes its artifact to `opencode-ai@1.14.31`, confirms no Homebrew formula is installed, and records the JS launcher (`3ab08cf…`) separately from its native payload (`40d5686…`). Only the approved absolute 1.18.11 file with digest `f554a08…` enables A-MAC capabilities; replacement invalidates the probe without exposing the user path. | A-MAC |
| CORE-01 | Unconfigured, trusted, changed, disabled, and malicious hook/plugin | No silent install or trust bypass; hash/config change invalidates trust; absence keeps normal terminal use and `Unknown`. | A-MAC, A-LNX |
| CORE-02 | Running, duplicate/out-of-order/cross-session/stale, and concurrently delivered events | State machine accepts only correlated transitions ordered by a source sequence/turn token or a proven serialized channel; deliver two order-sensitive events with interleaved/reversed receive order and no ordering token and assert the transition is `Unknown`, not a guessed completion. Lease expiry and irreconcilable conflict also yield `Unknown`. | Unit fixtures + A-MAC |
| CORE-03 | Malformed, truncated, oversized, unknown-version structured stream | Parser remains bounded, does not crash/execute content, keeps the pane usable, and marks semantic state `Unknown`. | Unit/fuzz + A-LNX |
| CORE-04 | Clean/nonzero/signal process exit with/without terminal event | Always emit `ProcessExited`; emit `Completed`/`Error` only with the corresponding trusted semantic event. | A-MAC, A-LNX, A-PTY |
| CORE-05 | Error and payload redaction | Seed canaries in prompt/tool/path/session/error fields; logs, OS notifications, webhook fixtures, and crash reports contain none. | Unit + A-MAC |
| CORE-06 | Valid CWD/session/resume integration | Before use, capture and pin the exact installed executable/version's resume help and argv contract, including Codex `exec resume` flags; missing evidence disables that resume path. For Codex `0.146.0`, resume valid correlated sessions through both documented `SESSION_ID` forms (UUID and thread name), assert that UUID-parsing input takes UUID precedence, and use executable+argv in a working directory whose legitimate path contains spaces/Unicode. Confirm the exact session resumes with no shell expansion and no cross-project resume. | A-MAC, A-LNX, A-SSH |
| CORE-13 | Thread-name and hostile resume-ID/argv fuzzing | Exercise valid-looking thread names alongside malformed and hostile session IDs and resume arguments (spaces, quotes, newlines, Unicode, option-like `-`/`--` prefixes, separator and metacharacter injection). A valid correlated thread name remains one non-shell argv element and resumes only that exact session; hostile input is rejected or neutralized before launch, never expanded, never reinterpreted as an option, and never resumes a different or cross-project session. A UUID-parsing value follows UUID precedence and cannot fall back to a same-text thread name. | Unit/fuzz + A-MAC, A-LNX |
| CORE-07 | Terminal/OSC/process-name spoofing | A program named like each agent and forged output/title/OSC cannot leave `Unknown` or create a trusted completion/approval notification. | A-PTY, A-SSH |
| CORE-08 | Local/remote event-channel impersonation | Wrong owner, wrong nonce/session/pane, replay, and cross-host frames are rejected; endpoint is never publicly bound. | Multi-user A-LNX, A-SSH |
| CORE-09 | Observer timeout/crash/backpressure | Agent continues; observer failure is visible separately; event backlog is bounded and one pane cannot stall others. | A-MAC, A-LNX |
| CORE-10 | Live process with event silence and timer reset | Emit one rate-limited long-unresponsive alert, set semantic state Unknown rather than Error, and clear/reset only on a fresh correlated event or explicit policy action. | Fake-clock unit + A-MAC/A-SSH |
| CORE-11 | Approval/input/completion/error/exit notification dedupe and navigation | Emit once per transition; OS text remains generic; click/shortcut focuses the exact local/SSH pane; stale locator performs no unrelated navigation. | A-MAC, A-SSH |
| CORE-12 | Local-hook/webhook timeout, replay, malicious response, and secret canary | Sink receives only the minimal schema, cannot command execution/state, is bounded/cancelable, and leaks no canary under retry/error logging. | Loopback fixture, A-MAC/A-LNX |
| CX-01 | Codex prompt/start/activity | Pin the exact CLI version and verify the officially documented hook availability and event shape before enabling the lifecycle mapping; otherwise remain `Unknown`. Passive hooks then correlate session/turn/CWD and renew `Running`; dropped delivery expires to `Unknown`. | Codex 0.146.0, A-MAC/A-LNX/A-SSH |
| CX-02 | Codex approval allow/deny/interactive prompt with concurrent hook | `PermissionRequest` alone never claims durable wait; experimental app-server request does until resolved. | Codex 0.146.0, A-MAC/A-LNX |
| CX-03 | Codex Stop continued by another hook vs final stop | Stop candidate cannot emit completion before conclusive terminal event; concurrent ordering is deterministic or `Unknown`. | Codex 0.146.0, isolated config |
| CX-04 | Codex close/archive/idle SessionEnd | No SessionEnd reason is mislabeled as task completion; cleanup uses only allowlisted fields. | Codex 0.146.0, A-MAC |
| CX-05 | Codex exec success/failure/interruption/truncated JSONL | Exact JSONL transitions map to Running/Completed/Error; process exit conflict and missing terminal record become `Unknown`. | Codex 0.146.0, A-MAC/A-LNX |
| CX-06 | Experimental Codex app-server turn/approval/user-input lifecycle | Negotiate capability; validate request resolution, interrupted/failed/completed distinction, disconnect/reconnect, and experimental opt-out. | Codex 0.146.0, A-MAC/A-LNX |
| CL-01 | Claude prompt/activity and CWD change | Running/session/CWD correlation survives direct/nested PTY use and stales safely. | Claude 2.1.220, A-PTY/A-SSH |
| CL-02 | Claude permission prompt, hook auto-decision, denial | Only passive `permission_prompt` creates AwaitingApproval; later correlated activity/idle/error clears it exactly once. | Claude 2.1.220, isolated config |
| CL-03 | Claude successful, cancelled, and errored response followed by idle | A successful turn then `idle_prompt` emits exactly one completion followed by AwaitingInput; startup idle, a cancelled/interrupted turn then idle, and a `StopFailure`/error turn then idle each reach AwaitingInput (error first where applicable) and emit no false completion. | Claude 2.1.220, A-MAC/A-LNX |
| CL-04 | Claude Stop continuation and hook ordering | A Stop hook that feeds back prevents premature completion; observer cannot alter the turn. | Claude 2.1.220, isolated config |
| CL-05 | Claude StopFailure categories | Each documented category produces sanitized Error; hook failure/tool failure do not. | Claude 2.1.220, mocked API failure harness |
| CL-06 | Claude background needs-input/completed with view open/closed and two simultaneous agents | Version/view limitation is honored; combined completed-or-failed event stays `Unknown` without stronger outcome; with two concurrent background agents, an uncorrelated `agent_needs_input` marks neither session `AwaitingInput` and stays `Unknown` rather than guessing which agent waits. | Claude 2.1.220, A-MAC |
| CL-07 | Claude headless JSON/stream success/error/retry/SIGTERM/truncation | Final result and exit agree; retries remain Running; missing/conflicting final record becomes `Unknown`; no lost tail under backpressure. | Claude 2.1.220, A-MAC/A-LNX |
| OC-01 | OpenCode initialization and snapshot/subscription boundary transitions | Start Unknown; establish subscription and buffer before snapshot; force busy/idle changes across every boundary. Require a documented cursor/epoch or adapter handshake to claim continuity; otherwise remain Unknown until a fresh post-subscription event. | Intended OpenCode 1.18.11, A-MAC/A-LNX |
| OC-02 | OpenCode permission asked/replied/rejected/duplicate | AwaitingApproval is request-ID scoped and clears once without logging patterns/metadata. | Intended OpenCode 1.18.11, A-MAC/A-LNX |
| OC-03 | OpenCode question asked/replied/rejected | AwaitingInput is request-ID scoped; capability is disabled and state Unknown if the installed schema omits it. | Intended OpenCode 1.18.11, A-MAC/A-LNX |
| OC-04 | OpenCode initial idle, active-to-idle, and error/abort-then-idle | Only an observed successful active turn emits Completed; initial idle, and a correlated `session.error` or abort followed by `session.idle`, map to AwaitingInput (error first for the error case) and suppress the false completion; stable idle emits no duplicate notifications. | Intended OpenCode 1.18.11, A-MAC/A-LNX |
| OC-05 | OpenCode retry and correlated/uncorrelated error | Retry remains Running; correlated error emits Error; missing-session error degrades affected scope to Unknown. | Intended OpenCode 1.18.11, A-MAC/A-LNX |
| OC-06 | OpenCode run raw JSON events and schema drift | Capture/version the exact 1.18.11 sequence for success, denial, question, error, and cancel; unknown shapes fail safely. | Intended OpenCode 1.18.11, A-MAC/A-LNX |
| OC-07 | OpenCode server restart/auth failure/tunnel loss/plugin failure | No public bind; authenticated health/version check; reconnect reconciles state; fallback terminal remains usable and state becomes Unknown. | Intended OpenCode 1.18.11, A-SSH |

## Gaps and questions for later RFCs

1. Is Noren allowed to offer opt-in installation of a user-level observer, or
   must every integration be launch-only and ephemeral?
2. What lease duration and reconnect reconciliation avoid stale status without
   noisy `Unknown` transitions?
3. Should `Completed` be a transient event plus notification while the
   steady state becomes `AwaitingInput`, or should the UI retain it until
   focus?
4. Can Codex interactive status depend on its experimental app server, or must
   the first release expose only the hook-supported subset?
5. Do Claude notification hooks fire reliably in every supported PTY,
   permission mode, remote session, and with all other hooks/plugins enabled?
6. Is OpenCode's SSE/API compatibility stable enough to pin, and can a
   cursor/epoch or adapter handshake prove snapshot/subscription continuity?
7. Which fields, if any, may be persisted to support resume without turning
   session IDs and remote paths into durable sensitive metadata?
8. What authenticated remote event framing is shared with any future SSH
   helper, and how does it downgrade to ordinary terminal use?
9. Are OS notifications allowed to include repository/branch names, or must all
   external notification text remain generic until the pane is focused?

Until these gates pass, the honest feature level is version-scoped partial
awareness with `Unknown` fallback. No current evidence supports
process-name-only state detection.
