# Agent and tool inventory

Captured on 2026-08-03 in Asia/Tokyo from local commands and filtered,
non-secret configuration fields. Raw help output is stored in
[`cli-help/`](cli-help/).

| Role | Command and version | Verified model/config | Non-interactive | Resume | State |
| --- | --- | --- | --- | --- | --- |
| Codex integrator | `codex`, `codex-cli 0.146.0` | `gpt-5.6-sol`, reasoning `max` in local config | `codex exec` | `codex resume [SESSION_ID]` or `--last` | Authenticated; calibration completed |
| codex-lab QA | `codex-lab`, `codex-cli 0.146.0` | wrapper selects isolated `CODEX_HOME`; `gpt-5.6-sol`, reasoning `max` | `codex-lab exec` | `codex-lab resume [SESSION_ID]` or `--last` | Calibration completed |
| Claude critical review | `claude`, `2.1.220` | configured alias `opus[1m]`; calibration reported canonical `claude-opus-5` | `claude --print` | `claude --resume [SESSION_ID]` or `--continue` | Authenticated; calibration completed |
| Qwen UI/UX candidate | `opencode`, `1.18.11` | `qwencloud/qwen3.8-max-preview` from `opencode models` | `opencode run` | `--continue` or `--session ID` | Calibration completed |
| GLM Rust-core candidate | `opencode`, `1.18.11` | `zai-coding-plan/glm-5.2` from `opencode models` | `opencode run` | `--continue` or `--session ID` | Calibration completed |
| Fugu remote candidate | `opencode`, `1.18.11` | `sakana/fugu-ultra` and dated variant listed by `opencode models` | `opencode run` | `--continue` or `--session ID` | Calibration completed; high latency observed |

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
