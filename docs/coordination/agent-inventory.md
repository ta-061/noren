# Agent and tool inventory

Captured on 2026-08-03 in Asia/Tokyo from local commands and filtered,
non-secret configuration fields. Raw help output is stored in
[`cli-help/`](cli-help/).

| Role | Command and version | Verified model/config | Non-interactive | Resume | State |
| --- | --- | --- | --- | --- | --- |
| Codex integrator | `codex`, `codex-cli 0.146.0` | `gpt-5.6-sol`, reasoning `max` in local config | `codex exec` | `codex resume [SESSION_ID]` or `--last` | Authenticated; calibration completed |
| codex-lab QA | `codex-lab`, `codex-cli 0.146.0` | wrapper selects isolated `CODEX_HOME`; `gpt-5.6-sol`, reasoning `max` | `codex-lab exec` | `codex-lab resume [SESSION_ID]` or `--last` | Calibration completed |
| Claude critical review | `claude`, `2.1.220` | configured alias `opus[1m]`; calibration reported canonical `claude-opus-5` | `claude --print` | `claude --resume [SESSION_ID]` or `--continue` | Authenticated; calibration completed |
| Qwen UI/UX candidate | `$HOME/.opencode/bin/opencode`, `1.18.11` | `qwencloud/qwen3.8-max-preview` from the selected binary's model list | `$HOME/.opencode/bin/opencode run` | `--continue` or `--session ID` | Calibration completed |
| GLM Rust-core candidate | `$HOME/.opencode/bin/opencode`, `1.18.11` | `zai-coding-plan/glm-5.2` from the selected binary's model list | `$HOME/.opencode/bin/opencode run` | `--continue` or `--session ID` | Calibration completed |
| Fugu remote candidate | `$HOME/.opencode/bin/opencode`, `1.18.11` | `sakana/fugu-ultra` and dated variant from the selected binary's model list | `$HOME/.opencode/bin/opencode run` | `--continue` or `--session ID` | Calibration completed; high latency observed |

## OpenCode executable provenance

The initial help header incorrectly paired `/opt/homebrew/bin/opencode` with
version `1.18.11`. Revalidation found two installations. The shadowed path is a
symlink to the JavaScript launcher from the global npm package
`opencode-ai@1.14.31` under the Homebrew prefix; `brew info opencode` reports the
Homebrew formula as not installed. That launcher executes a package-specific
native payload. The help and model-list body preserved in
[`cli-help/opencode.txt`](cli-help/opencode.txt) matches the selected `1.18.11`
executable; future runs must use its explicit path instead of relying on `PATH`
order.

| Disposition | Artifact | Version | SHA-256 |
| --- | --- | --- | --- |
| Selected for Noren evidence | `$HOME/.opencode/bin/opencode` native executable | `1.18.11` | `f554a08dee4c34f4f43df63af72f0a6afbe57f955496853f411767718927bf2c` |
| Shadowed, not selected | `/opt/homebrew/bin/opencode` symlink target: global npm JavaScript launcher | `opencode-ai@1.14.31` | `3ab08cfdb3cf1213eaeae45f557fb3220e0999862d8dc90eb17ba4cacf97c57b` |
| Shadowed, not selected | npm launcher's `opencode-darwin-arm64` native payload | `1.14.31` | `40d5686fc86e94f833ac3e5855e12802464f2de0bcc1616013c62844bf6996d4` |

No executable was installed, updated, or removed during this correction.

## Development environment

| Tool/state | Evidence |
| --- | --- |
| Git repository | clean `main` at initial commit `c692ad1` before this branch |
| Remote | `https://github.com/ta-061/noren.git` |
| GitHub CLI | authenticated as repository owner; identity/token details are not stored here |
| Rust toolchain | `cargo`, `rustc`, and `rustup` were not installed or not on `PATH` |
| Target host | macOS Apple Silicon |

The absence of a Rust toolchain blocks Rust compilation but does not block
Discovery and design. A pinned toolchain will be installed and recorded before
the first Rust experiment or production crate is accepted.
