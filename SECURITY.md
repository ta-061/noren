# Security policy

## Supported versions

Noren has no released or supported version yet. Security reports about repository
content, build/release infrastructure, or future Preview code are still welcome.

## Reporting a vulnerability

Use GitHub's private vulnerability reporting for this repository:

<https://github.com/ta-061/noren/security/advisories/new>

Do not open a public Issue for a suspected vulnerability. Include the affected
revision, environment, impact, minimal non-destructive reproduction, and any
suggested mitigation. Remove API keys, tokens, cookies, SSH keys, usernames,
hostnames, and unrelated personal data from evidence.

The maintainers will acknowledge the report when capacity permits, validate it,
coordinate a fix and disclosure window, and credit reporters who want credit.
No fixed response SLA is promised before the first Preview release.

## Security priorities

High-priority areas include PTY/process boundaries, SSH and host-key behavior,
remote-session IPC, command/argument construction, path traversal, OSC 8/52 and
terminal titles, clipboard access, URL opening, configuration persistence,
plugin/agent hook permissions, log redaction, release integrity, and unsafe Rust.
