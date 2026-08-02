# SSH and remote execution research

Status: evidence report; no transport or daemon adoption decision

Issue: [#5](https://github.com/ta-061/noren/issues/5)

Evidence retrieved: 2026-08-03

Repository baseline: `7cd56c689546bcfc38f083551813abe32e48469f`

Review gates: pending. This artifact does not claim completion of either the
required Claude Code security review or the codex-lab testability review.

## Scope and decision boundary

This report records the behavior Noren would have to preserve when opening a
local-to-remote workspace. It compares implementation surfaces and turns
uncertainty into experiments. It does **not** select an SSH library, decide
whether a remote daemon exists, define a repository split, or authorize a
network listener.

The desired contract is narrower than “implement SSH”: use the user's existing
OpenSSH destination, identity, agent, host-key, proxy, and keepalive policy
where possible; never store a private key or passphrase in a Noren-specific
format; isolate failures to the affected connection/session; and preserve a
normal OpenSSH path when enhanced remote features are unavailable.

## Evidence baseline

OpenSSH `10.4/10.4p1`, released 2026-07-06, was the current stable release at
retrieval time. The official release page supplies the release date and source
checksums. The OpenBSD manual pages describe the latest development version, so
each compatibility test must also record the installed client version rather
than assuming the live manual exactly matches every system client. The portable
tree was inspected at
`ec0485632885e0c533b35e5970e4b583781db83f` (2026-08-01). OpenSSH is covered
by the upstream multi-file, BSD-style and component-specific license notices;
it must not be summarized as a single MIT or Apache-2.0 work.

Primary sources:

- [OpenSSH release notes](https://www.openssh.com/releasenotes.html)
- [OpenSSH manuals](https://www.openssh.com/manual.html)
- [ssh_config(5)](https://man.openbsd.org/ssh_config)
- [ssh(1)](https://man.openbsd.org/ssh)
- [ssh-agent(1)](https://man.openbsd.org/ssh-agent)
- [portable source at the inspected commit](https://github.com/openssh/openssh-portable/tree/ec0485632885e0c533b35e5970e4b583781db83f)
- [portable license notices](https://github.com/openssh/openssh-portable/blob/ec0485632885e0c533b35e5970e4b583781db83f/LICENCE)

## OpenSSH behavior Noren must not silently reinterpret

OpenSSH obtains configuration in command-line, per-user, then system order.
For each parameter, the first obtained value wins; consequently, host-specific
declarations normally need to appear before general declarations. This
first-value rule is material when evaluating aliases and included files.

| Surface | Upstream meaning | Noren contract risk | Future gate |
| --- | --- | --- | --- |
| `Host` | Conditional pattern matched against the destination name given on the command line; it can be an alias and can use negation/wildcards. | Treating it as a DNS name loses the alias and may select the wrong block. | SSH-01 |
| `HostName` | Real host name or address selected after a matching block; tokens are allowed. | Resolving the alias in Noren can disagree with OpenSSH token/canonicalization behavior. | SSH-01, SSH-02 |
| `User`, `Port` | Remote login name and server port, subject to first-value precedence. | Rebuilding a destination string can override or drop configuration. | SSH-01 |
| `Include` | Includes one or more files; paths may use globbing, tokens, environment variables, or `~`; wildcard expansion is lexical. | A partial parser can miss nested or ordered policy, including security settings. | SSH-02 |
| `IdentityFile` | May appear multiple times. A public-key filename can identify the corresponding private key held by an agent. | Selecting only one path or reading private-key bytes changes authentication behavior. | SSH-03 |
| `IdentityAgent` | Selects the agent socket and overrides `SSH_AUTH_SOCK`; `none` disables agent use. | Blindly inheriting an environment agent defeats explicit user policy. | SSH-03 |
| `ProxyJump` | Connects through one or more jump hosts. The first value between `ProxyJump` and `ProxyCommand` wins. Destination configuration does not generally become jump-host configuration. | Flattening the hop list or applying destination settings to jumps changes routing/authentication. | SSH-04 |
| `ProxyCommand` | Executes a command via the shell; percent tokens are expanded and are not shell-escaped by OpenSSH. `CheckHostIP` is unavailable for connections using it. | The user's config is an executable-code trust boundary. Reconstructing it from tokens is unsafe and can be semantically wrong; hostname-only checks cannot be assumed. | SSH-04, SEC-02 |
| `ForwardAgent` | Disabled by default. A remote party with access to the forwarded socket cannot extract key material, but can ask the agent to authenticate/sign while the socket is available. | Agent forwarding is delegated signing authority, not a harmless convenience. Never enable it implicitly. | SSH-03, SEC-03 |
| `StrictHostKeyChecking` | `yes` refuses new/changed keys; `accept-new` adds new but refuses changed keys; `ask` is the default; `no/off` permits more changes subject to restrictions. | “Connect anyway” can turn a host-key failure into credential/data exposure. | SSH-05 |
| `UserKnownHostsFile` | Selects one or more user host-key databases; defaults include the user's known-hosts files. | A library-owned store can ignore hashed names, aliases, ports, certificates, or user-selected files. | SSH-05 |
| `ServerAliveInterval`, `ServerAliveCountMax` | Encrypted protocol-level liveness messages; interval defaults to zero and count defaults to three. | TCP silence is not proof of a dead server, and overriding values changes user policy. | SSH-06 |
| `TCPKeepAlive` | OS TCP keepalive, enabled by default; unlike server-alive messages it is spoofable. | It is not an authenticated application heartbeat and must not drive semantic session state alone. | SSH-06 |
| `ControlMaster`, `ControlPath`, `ControlPersist` | Reuse sessions through a Unix socket. `auto` can fall back to a new connection. A path should uniquely encode host/port/user (or `%C`) and live in a directory not writable by others. | Cross-destination socket collisions can attach to the wrong security context; a stale master can look like a successful new connection. | SSH-07 |

The option definitions and precedence are in
[ssh_config(5)](https://man.openbsd.org/ssh_config). `ssh -G destination`
prints evaluated configuration and is useful as an oracle, but its output can
contain usernames, paths, proxy commands, and topology, so raw output is test
evidence only and must not enter normal logs. `ssh -O check` and
`ssh -O exit` are control-socket operations, not generic health checks for
every transport; see [ssh(1)](https://man.openbsd.org/ssh).

OpenSSH 10.3 tightened command-line `ProxyJump` validation after documenting
that hostile command-line usernames could reach shell expansion through proxy
tokens. That fix does not make arbitrary user-derived SSH options safe. See the
[OpenSSH 10.3 release notes](https://www.openssh.com/releasenotes.html#10.3).

Host discovery is also lossy unless OpenSSH evaluates it. A `Host` entry can
be a wildcard/negation instead of a selectable destination, and includes may be
dynamic. A future host browser must show source/provenance, avoid DNS/network
scanning, and label partial discovery rather than invent a complete list.
Favorites and connection history are user/host/topology metadata: keep them
user-scoped, redact them from diagnostics, provide deletion, and test migration
and corruption behavior in SSH-11.

## Process and command boundaries

There are two different command-construction boundaries:

1. **Local launch.** A subprocess implementation can invoke a resolved
   `ssh` executable with an argument vector, without a local shell. The
   destination must be a separately validated argument; a leading option-like
   value is rejected until a version-tested end-of-options contract exists.
   Noren must not concatenate a hostname, `-o` value, jump host, path, or user
   text into a shell command. It also must not expose an “extra SSH arguments”
   string without a separately specified parser and threat model.
2. **Remote command.** The SSH protocol sends a command string for server-side
   execution; normal servers commonly pass it to the login shell. Therefore,
   local argv separation does **not** prove that a dynamic remote command is
   injection-safe. An enhanced transport should launch a fixed, versioned
   helper command and send CWD/session/request values inside a framed stdin
   protocol. Quoting arbitrary remote paths into a command string remains
   unverified.

The user's OpenSSH configuration can itself execute `ProxyCommand`,
`Match exec`, or `LocalCommand`. Reusing it means accepting the same
user-controlled configuration trust boundary as invoking `ssh` in a terminal.
Noren may display this boundary and restrict which config source it opts into;
it cannot both promise exact config reuse and claim that configuration is inert
data.

OpenSSH returns the remote command's exit status, or `255` when an SSH error
occurs. A remote program can also exit `255`, so exit status alone cannot
unambiguously classify transport failure. Classification needs stderr/event
evidence or an authenticated helper protocol; otherwise it is `Unknown`.

## Host identity and credentials

Host-key verification belongs to the SSH transport that performs the
handshake:

- When OpenSSH is the transport, let it apply the user's
  `StrictHostKeyChecking`, `UserKnownHostsFile`, host certificates,
  hashing, aliases, `known_hosts` files, and prompts. Do not parse a prompt
  to auto-accept it.
- When an embedded library is the transport, Noren becomes responsible for
  loading the applicable host-key databases, computing the lookup name
  correctly for aliases/non-default ports/proxies, distinguishing unknown from
  changed keys, and persisting an accepted key atomically with safe
  permissions. This is a release gate, not UI polish.
- A host-key mismatch, revoked key, authentication failure, or user rejection
  is terminal for automatic reconnect. Only explicit user action may retry
  after the cause is visible.

Private keys and passphrases remain outside Noren persistence. Candidate
implementations can ask OpenSSH or the user's agent to authenticate. If an
embedded stack needs direct key-file loading, that is a new credential-handling
surface requiring a separate threat review; this report does not approve it.
Never log identity-file contents, passphrases, agent socket paths, expanded
config, authentication methods offered, or raw server banners.

## Implementation surfaces compared

The following are candidates for a PoC, not an adoption ranking.

| Candidate and pinned evidence | Config/proxy fidelity | Host keys and agent | Runtime/dependency boundary | License and currentness | Unknowns that block adoption |
| --- | --- | --- | --- | --- | --- |
| Installed OpenSSH subprocess; stable reference `10.4p1`; portable source `ec048563…` | Highest available reuse because OpenSSH itself evaluates `Host`, `Include`, tokens, jump/proxy rules, and system/user files. `ssh -G` can serve as an oracle. | OpenSSH owns known-hosts prompts/checks and agent selection. | External binary and OS packaging vary. Process supervision, stderr parsing, control sockets, and remote-command shell semantics stay in Noren's boundary. | Upstream multi-license notices; 10.4p1 released 2026-07-06; portable head inspected 2026-08-01. | Executable discovery/integrity, Apple-vs-portable behavior, noninteractive prompts, exit-255 ambiguity, cancellation/process-tree cleanup, and multiplex failure isolation. |
| [Russh](https://github.com/Eugeny/russh/tree/4882af71cf27ea5293636bf4985ef296dcf20896) `0.62.5` plus [russh-config](https://github.com/Eugeny/russh/tree/4882af71cf27ea5293636bf4985ef296dcf20896/russh-config) `0.58.0` | Native Tokio SSH client/server with channels, PTY, forwarding, keepalive, and agent features. The pinned config source implements only a subset and has the confirmed incompatibilities below. It is not an OpenSSH config-equivalence layer. | Client `Handler::check_server_key` defaults to rejection, so an application must implement policy. Agent support exists, but the pinned config parser has no `IdentityAgent` support. | Rust/Tokio stack. [0.62.4](https://github.com/Eugeny/russh/releases/tag/v0.62.4) fixed malformed-input panics; 0.62.5 added a security fix and channel backpressure. Malformed-input robustness remains a fuzz target. | Apache-2.0; [0.62.5 release](https://github.com/Eugeny/russh/releases/tag/v0.62.5), 2026-07-31, commit `4882af71…`; [license](https://github.com/Eugeny/russh/blob/4882af71cf27ea5293636bf4985ef296dcf20896/LICENSE.txt). | Equivalence-fixture results, certificate/revocation behavior, algorithm/platform coverage, cancellation, and security-update response. Unsupported config semantics require the ordinary OpenSSH path. |
| [ssh2-rs](https://github.com/rust-lang/ssh2-rs/tree/5b39b5fabb6b5a6b953519a571cd6af30d460ac3) `0.9.6` over [libssh2](https://libssh2.org/) `1.11.1` | Provides SSH primitives, not an evidenced OpenSSH-config evaluator. Config and ProxyJump/ProxyCommand compatibility would be application work or another dependency. | APIs expose host keys, known-host checks/files, agent authentication, PTYs, channels, and keepalive. The application must explicitly perform verification and select/persist policy. | Rust wrapper over the C libssh2 library with native TLS/crypto/build concerns and an FFI/unsafe audit boundary. | ssh2-rs is MIT OR Apache-2.0 ([MIT](https://github.com/rust-lang/ssh2-rs/blob/5b39b5fabb6b5a6b953519a571cd6af30d460ac3/LICENSE-MIT), [Apache-2.0](https://github.com/rust-lang/ssh2-rs/blob/5b39b5fabb6b5a6b953519a571cd6af30d460ac3/LICENSE-APACHE)); versioned [0.9.6 docs](https://docs.rs/crate/ssh2/0.9.6) and the [annotated 0.9.6 tag](https://github.com/rust-lang/ssh2-rs/tree/0.9.6) resolve to release commit `5b39b5fabb6b5a6b953519a571cd6af30d460ac3` (2026-06-30). libssh2 1.11.1 released 2024-10-16 under its [revised BSD license](https://libssh2.org/license.html). | OpenSSH config layer, jump chaining, host certificates/revocation parity, crypto packaging, async integration, cancellation, and dependency vulnerability/update ownership. |

### Confirmed russh-config limits at the pinned source

These are source-confirmed properties of Russh tag `v0.62.5`,
`russh-config` crate `0.58.0`, commit `4882af71…`; they are not
`Unknown`:

- The parser has no handlers for `Include`, `Match`, or `IdentityAgent`,
  and implements no hostname canonicalization.
- `parse_home()` reads only `~/.ssh/config`. `parse_path()` can read one
  caller-selected file, but there is no automatic system
  `/etc/ssh/ssh_config` load or multi-file merge.
- `StrictHostKeyChecking` is stored as `Option<bool>`: exactly `no`
  becomes false and every other parsed value becomes true. This collapses
  OpenSSH's distinct `ask`, `accept-new`, `yes`, `off`, and invalid
  value behavior.
- `ProxyJump` is parsed into `HostConfig.proxy_jump`, but
  `Config::stream()` never reads that field: it uses `ProxyCommand` when
  present and otherwise opens a direct TCP stream.
- `ProxyCommand` is expanded, split with `split(' ')`, and started directly
  as executable plus arguments. It therefore does not implement OpenSSH's
  user-shell execution, quoting, or whitespace semantics.

SSH-02, SSH-04, and SSH-05 must compare each candidate against `ssh -G` and
real connections. For any destination whose evaluated configuration depends on
one of these unsupported semantics, the enhanced Russh path cannot claim
equivalence; the ordinary OpenSSH path is required.

Supporting upstream APIs inspected:

- [russh client host-key callback](https://github.com/Eugeny/russh/blob/4882af71cf27ea5293636bf4985ef296dcf20896/russh/src/client/mod.rs)
- [russh-config parser](https://github.com/Eugeny/russh/blob/4882af71cf27ea5293636bf4985ef296dcf20896/russh-config/src/lib.rs)
- [russh-config proxy launcher](https://github.com/Eugeny/russh/blob/4882af71cf27ea5293636bf4985ef296dcf20896/russh-config/src/proxy.rs)
- [ssh2-rs known-host API](https://github.com/rust-lang/ssh2-rs/blob/5b39b5fabb6b5a6b953519a571cd6af30d460ac3/src/knownhosts.rs)
- [libssh2 API reference](https://libssh2.org/docs.html)
- [libssh2 1.11.1 release](https://github.com/libssh2/libssh2/releases/tag/libssh2-1.11.1)

No row demonstrates complete parity yet. In particular, a green handshake is
not evidence of correct OpenSSH config or host-key behavior.

## Connection and reconnect state model

Transport state and remote-session state must be separate. A possible
implementation vocabulary for tests is:

`Configured -> Connecting -> HostKeyDecision? -> Authenticating -> Connected`

and independently:

`Absent -> Starting -> Attached -> Detached? -> Exited | Lost | Unknown`

This is a test vocabulary, not a UI/API decision. Required behavior:

- A clean shell exit is not a disconnect (SSH-08, REM-01).
- EOF, keepalive exhaustion, control-master loss, local suspend/resume, and
  remote reboot are different failure causes even if the UI ultimately offers
  reconnect (SSH-06, SSH-10, REM-09).
- Retry may be automatic only for failures classified as transient. Use a
  bounded, cancelable delay with jitter in experiments; retry timing is not
  selected here (REM-09).
- Never automatically retry a host-key change, trust rejection, invalid
  destination, authentication failure, or protocol/hash incompatibility
  (SSH-05, SSH-09, REM-05, REM-07, REM-09).
- A connection worker owns only its channels and sessions. Its failure cannot
  close local panes or unrelated remote hosts (REM-06).
- Multiplex reuse must bind destination, remote user, port, relevant proxy
  context, and agent/security context. If that equivalence cannot be proven,
  open a separate connection (SSH-07).
- After any reconnect, the old channel is dead. Only a confirmed persistent
  remote owner may reattach to an existing PTY; otherwise start a new shell
  with an explicit loss notice (REM-02, REM-04, REM-09).

## Remote execution options

### Option A: daemonless OpenSSH

Launch a normal OpenSSH shell/PTY and use the terminal stream only. This is the
required fallback and supplies the least new remote code. It cannot by itself
prove the remote process tree, authoritative CWD, or PTY survival after
transport loss. Whether a child survives a real disconnect depends on the
remote shell, PTY, signals, service manager, and tool (for example tmux or
Zellij), and is therefore `Unknown` until REM-02.

### Option B: transient helper over standard input/output

Start a fixed helper through SSH. Reserve stdout for length-delimited protocol
frames, use bounded/redacted stderr for diagnostics, and send all values after
startup rather than interpolating them into the remote command. The helper owns
PTY allocation, process metadata, and CWD while the SSH channel lives. This
reduces parsing ambiguity but does not provide persistence across channel loss.

### Option C: persistent per-user helper

A user-scoped helper can own PTYs/processes after a client disconnect and allow
reattachment by opaque session ID. Reach it either through a fixed stdio
bootstrap or an SSH-forwarded Unix-domain socket. OpenSSH supports local and
remote Unix-socket forwarding via `-L`; a public TCP listener is unnecessary.
This option adds lifecycle, installation, authentication, upgrade, and stale
state risks. It remains a candidate pending the gates below.

### Protocol properties to test before a daemon decision

- The first exchange carries protocol major/minor, capabilities, helper build
  version, target OS/architecture, and a fresh connection nonce. Unknown major
  versions fail closed; minor/capability downgrade behavior is explicit
  (REM-05).
- Frames have a fixed maximum length and reject truncation, duplicate IDs,
  invalid UTF-8 where text is required, unknown mandatory fields, and
  out-of-order lifecycle transitions without unbounded allocation (REM-03).
- Session IDs are random opaque capabilities scoped to the authenticated OS
  user. They are compared exactly, never used as paths, and never logged
  (REM-04, REM-08).
- The helper, not terminal OSC or shell-title text, is authoritative for its
  child PID, PTY ownership, exit status, and current directory. Reported CWD is
  still remote-user-controlled display data and cannot authorize a local path
  (REM-04).
- The helper runs as the SSH user, never root; uses a user-only directory
  (mode `0700` candidate), user-only files/socket (mode `0600` candidate),
  no ambient listener, no shell expansion for protocol fields, and no broader
  filesystem/network rights than the launched program already has (REM-08).
- Distribution needs a separately specified trusted manifest/signature and
  digest algorithm. A digest sent by the same untrusted channel as the binary
  only detects transfer corruption, not substitution. The signing/root-of-trust
  design is `Unknown`.
- A candidate upgrade writes a temporary file in the same user-owned directory,
  verifies it, atomically installs it, health-checks the new version, and keeps
  the prior known-good binary for rollback. Crash consistency, concurrent
  clients, filesystem semantics, and rollback are REM-07 gates, not established
  facts.
- Missing helper, incompatible version, failed hash/signature, failed health
  check, or denied installation must leave daemonless OpenSSH available without
  weakening host-key/authentication policy (REM-01, REM-07).

OpenSSH's [Unix-socket forwarding and PTY behavior](https://man.openbsd.org/ssh)
support the transport experiments; they do not settle whether Noren should ship
a helper.

## Threat boundaries

| Boundary | Credible failure | Required control | Gate |
| --- | --- | --- | --- |
| UI/config to local process | Option injection or local shell injection through alias/options/path. | Structured argv, destination validation, no shell, no unrestricted option string. | SEC-01 |
| OpenSSH config to host OS | `ProxyCommand`, `Match exec`, or `LocalCommand` runs local commands. | Treat selected config as trusted executable user configuration; show provenance; never import remote/untrusted config silently. | SEC-02 |
| Local agent to remote host | Forwarded agent can be used for signing while accessible. | Preserve default-off policy; never enable forwarding implicitly; warn with destination scope. | SEC-03 |
| Network to client | Unknown/changed host key or downgrade/MITM. | Preserve strict known-host policy; changed key is non-retryable; no silent acceptance. | SSH-05 |
| Remote command string to login shell | Path/session text becomes shell syntax. | Fixed bootstrap command; framed stdin values; hostile-character fixtures. | SEC-04 |
| Remote helper to filesystem/processes | Path traversal, symlink replacement, cross-user socket access, arbitrary signal. | Opaque IDs, descriptor-relative operations, ownership/mode checks, process ownership validation, least privilege. | SEC-05 |
| Protocol/logging | Tokens, prompts, paths, usernames, topology, agent socket, or terminal contents leak. | Field allowlist; structured redaction before formatting; bounded payloads; no raw frames or environment dumps. | SEC-06 |
| Upgrade channel to executed binary | Substituted or partial helper runs. | Independent signed manifest, digest verification, atomic install, health check, rollback; fallback on failure. | REM-07 |
| Control socket to connection reuse | Attacker-created/stale/colliding socket selects wrong master. | User-only parent, ownership checks, unique context key, fail closed or new connection. | SSH-07 |

## Executable validation matrix

All fixtures use generated throwaway host/user keys and isolated temporary
configuration. They must never read a developer's real `~/.ssh`. Each run
records client/server versions and OS/architecture.

Target environments:

- **M-ARM**: the observed macOS 26.4.1 (build 25E253) arm64 host with
  Apple-shipped OpenSSH 10.2p1/LibreSSL 3.3.6. A later support matrix must add
  the minimum macOS release.
- **L-X64**: Ubuntu 24.04 LTS x86_64 client with pinned portable OpenSSH
  10.4p1; record the image digest and build options.
- **S-X64**: disposable Ubuntu 24.04 LTS x86_64 VM/container running pinned
  portable OpenSSH 10.4p1 `sshd`; a VM is required for reboot/suspend and PTY
  signal tests.
- **PAIR**: M-ARM→S-X64 and L-X64→S-X64, with a network fault proxy.

| ID | State/failure exercised | Executable assertion | Environment |
| --- | --- | --- | --- |
| SSH-01 | Alias selects destination/user/port | Compare actual server-observed peer user/port and `ssh -G` oracle for overlapping specific/general blocks. | M-ARM, L-X64 → S-X64 |
| SSH-02 | Include/token/precedence | Generate nested includes, lexical globs, `~`, and first-value conflicts; candidate output must match `ssh -G` or mark unsupported. | M-ARM, L-X64 |
| SSH-03 | Identity/agent success, disabled agent, forwarding | Use two keys and isolated agent sockets; prove repeated identity selection, `IdentityAgent none`, no implicit forwarding, and clean refusal. | PAIR |
| SSH-04 | ProxyJump/ProxyCommand success and malformed input | Traverse two disposable jumps; compare OpenSSH oracle; inject whitespace/metacharacter/percent-token fixtures without interpolating secrets. | L-X64 lab |
| SSH-05 | New, trusted, changed, revoked, non-default-port host keys | Assert the configured policy and file are used; changed/revoked keys never auto-retry or auto-accept. | PAIR |
| SSH-06 | Healthy idle, blackhole, spoofable TCP keepalive | Fault proxy drops packets independently; encrypted server-alive exhaustion drives transport loss only at configured thresholds. | PAIR |
| SSH-07 | New/reused/stale/colliding/control-socket permissions | Assert context isolation, parent ownership/mode, safe fallback, `-O check` behavior, and cleanup after master death. | M-ARM, L-X64 |
| SSH-08 | Remote exit 0, nonzero, 255; SSH error 255 | Demonstrate ambiguity; candidate must not label remote 255 as transport failure without corroborating protocol evidence. | PAIR |
| SSH-09 | Authentication rejection/cancel/timeout | No automatic retry loop; error is scoped to one connection and other panes/hosts remain live. | PAIR |
| SSH-10 | Local cancel, client crash, suspend/resume, server reboot | Descendants/channels/sockets are cleaned or retained only by documented persistent owner; no orphaned UI state. | M-ARM + S-X64 VM |
| SSH-11 | Host search, favorite, and history privacy/corruption | Wildcards are not offered as concrete hosts; no network scan; provenance is visible; delete/migrate/corrupt-store fixtures cannot leak or cross users. | M-ARM, L-X64 |
| REM-01 | Daemonless shell start/exit | Open shell in requested remote directory, preserve terminal bytes, report clean exit, and keep unrelated local/remote panes live. | PAIR |
| REM-02 | PTY during cable loss/client kill/server reboot | Record child signal/process/PTY outcome for bash, zsh, tmux, and Zellij. Until each observation passes, persistence is `Unknown`. | M-ARM + S-X64 VM |
| REM-03 | Transient framed helper | Fragment/coalesce frames; reject oversized/truncated/duplicate/out-of-order input; stdout remains protocol-only. | L-X64 unit + PAIR integration |
| REM-04 | Persistent attach/session/CWD/process | Start in paths containing spaces/newlines/non-ASCII, disconnect, reconnect, and verify the same PTY/process descriptor and authoritative remote CWD. | PAIR |
| REM-05 | Major/minor/capability mismatch | Matrix old/new client/helper; unsupported major fails closed; optional capability downgrade never changes security semantics. | L-X64 |
| REM-06 | Worker crash and connection fan-out | Crash one session/helper/channel; only dependent views degrade, and daemonless sessions plus another host continue. | PAIR with two S-X64 hosts |
| REM-07 | Hash/signature/install/upgrade/recovery | Corrupt download, wrong signature/digest, disk full, kill at each install step, concurrent upgrade, failed health check, rollback, and daemonless fallback. | S-X64 VM filesystems |
| REM-08 | Socket and least-privilege boundary | Reject wrong owner/mode, symlink/path traversal, foreign PID/signal, public bind, and cross-user attach. | Multi-user S-X64 VM |
| REM-09 | Reconnect classification | Drop network, expire keepalive, kill master/helper, rotate host key, reject auth, and return protocol mismatch; only classified transient cases retry and all retries are cancelable/bounded. | PAIR |
| SEC-01 | Local argv injection | Property-test destination/path bytes including option prefixes, spaces, quotes, newlines, and metacharacters; inspect exec argv and prove no local shell. | M-ARM, L-X64 |
| SEC-02 | Executable config boundary | Run isolated harmless marker commands through `ProxyCommand` and `Match exec`; confirm provenance warning and no loading from untrusted config. | L-X64 |
| SEC-03 | Agent delegation | A compromised fixture host can request signatures only when forwarding was explicitly enabled; no key bytes or socket paths enter logs. | S-X64 lab |
| SEC-04 | Remote-command injection | Hostile CWD/session strings arrive as exact framed values and cannot add a remote shell command. | PAIR |
| SEC-05 | Helper authorization | Fuzz IDs/paths and race symlinks/PIDs; only resources owned by the authenticated user/session are reachable. | S-X64 |
| SEC-06 | Redaction and bounds | Seed unique canary credentials/paths/prompts in every input field; captured logs/crash reports contain none and remain bounded under large input. | M-ARM, L-X64, S-X64 |

## Open questions carried forward

1. Which system OpenSSH versions define Noren's minimum macOS/Linux support
   matrix, and is a bundled executable permissible?
2. Is exact OpenSSH config reuse a hard requirement for enhanced sessions, or
   may an embedded path expose a documented subset and fall back?
3. What UI owns first-use host-key confirmation without encouraging blind
   acceptance?
4. Does multiplexing measurably improve pane startup enough to justify its
   socket and failure-coupling risks?
5. Which shells and remote operating systems are in the first supported set?
6. Do PTY persistence and process/CWD awareness justify a persistent helper
   after REM-01/REM-02 observation?
7. What release-signing root, manifest format, digest, rollback window, and
   revocation path would make helper installation trustworthy?
8. Can an enhanced helper use only stdio, or is a forwarded Unix socket needed
   for multiple clients and reattachment?

These questions belong in a later PoC/RFC/ADR. Until a gate passes, the
corresponding behavior is unsupported or `Unknown`, and ordinary OpenSSH
remains the recovery path.
