# Agent calibration

Captured: 2026-08-03 (Asia/Tokyo)

## Method

All six candidates received the exact same read-only keybinding-conflict API/test
design task in separate empty temporary directories. The prompt and preserved,
unedited responses are under [`calibration/`](calibration/). Commands used model
IDs demonstrated by local configuration or `opencode models`; no candidate could
see another response.

The task capped output at 1,200 words, prohibited repository/tool/network access,
required nine exact headings, and tested Rust API quality, deterministic
diagnostics, terminal input preservation, pass-through policy, table/property
tests, security bounds, uncertainty, and deferral discipline.

## Execution evidence

| Candidate | Model evidence | Isolation/non-interactive path | Outcome | Words |
| --- | --- | --- | --- | ---: |
| Codex | configured `gpt-5.6-sol` | `codex exec --ephemeral --sandbox read-only` | Completed | 1,089 |
| codex-lab | isolated config also `gpt-5.6-sol` | `codex-lab exec --ephemeral --sandbox read-only` | Completed | 1,025 |
| Claude Code | result reported canonical `claude-opus-5` | `claude --print --permission-mode plan --tools "" --no-session-persistence` | Completed; exceeded limit | 3,048 |
| Qwen | `qwencloud/qwen3.8-max-preview` from model list | `opencode run --pure --agent plan` | Completed; exceeded limit | 1,339 |
| GLM | `zai-coding-plan/glm-5.2` from model list | `opencode run --pure --agent plan` | Completed; exceeded limit | 1,258 |
| Fugu | `sakana/fugu-ultra` from model list | `opencode run --pure --agent plan` | Completed; high latency | 1,184 |

Authentication identifiers, organization details, session IDs, and credential
configuration are intentionally excluded.

## Rubric

Scores are 1 (materially deficient) through 5 (strong) and apply only to this one
task. They do not establish general model quality.

| Candidate | Correctness | Tests | Spec adherence | Rust quality | Clarity | Uncertainty | Scope discipline | Security | Total / 40 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Codex | 4 | 5 | 5 | 4 | 5 | 5 | 5 | 5 | 38 |
| codex-lab | 4 | 4 | 5 | 4 | 4 | 5 | 5 | 5 | 36 |
| Claude Code | 4 | 5 | 2 | 4 | 3 | 5 | 2 | 5 | 30 |
| Qwen | 3 | 4 | 3 | 3 | 4 | 5 | 4 | 4 | 30 |
| GLM | 2 | 4 | 3 | 2 | 4 | 4 | 4 | 4 | 27 |
| Fugu | 4 | 5 | 5 | 4 | 5 | 5 | 5 | 5 | 38 |

Response-to-review behavior was not exercised and is **not evaluated**. Rust
signatures were inspected but not compiled because no Rust toolchain is installed;
Rust-quality scores therefore remain provisional.

## Evidence notes

### Codex

Strong deterministic model, policy diagnostics, test coverage, input bounds, and
explicit unknowns. Its proposed scope-overlap rules still need an authoritative
capture-context model; `analyze` also needs a clearer error contract for malformed
input.

### codex-lab

Strong testable baseline and concise uncertainty handling. The
`configurable || disableable` invariant may be weaker than Noren's intended
ability to both rebind and disable every shortcut. Its treatment of some
Global-GUI versus terminal cases also needs review against real event routing.

### Claude Code

Strongest discussion of pass-through exit liveness, failed-config rollback,
combinatorial bounds, corpus drift, modal behavior, and protocol ambiguity. It
violated the explicit length cap by more than 2.5×, proposed a static upstream
corpus before verification, and asserted likely microsecond performance without a
benchmark. Use as a critical reviewer with strict output/evidence constraints, not
as an unchecked specification source.

### Qwen

Clear data model and useful UI-facing remediation shape. A material policy error
silently deactivates forbidden default Ctrl/Alt/function bindings and expects no
diagnostic; Noren needs an actionable configuration violation while preserving
input. Complexity discussion understates worst-case collision bucket growth, and
some types are left undefined.

### GLM

Useful test categories and a deterministic-report goal, but the proposed public
surface has unresolved Rust issues: undefined `NorenId`/`KeySeqRef`, an
implausible `&'static` return tied to policy data, inconsistent Shift
normalization, and no represented configurable/disableable state despite a
validator. It is not assigned unsupervised Rust-core ownership at this stage.

### Fugu

Strong validation/limit model, pass-through role constraints, structured errors,
non-keyboard fallback, and conservative security notes. Platform-shadow
classification needs refinement, and `NonZeroU8` alone does not cap function keys.
The response was much slower than peers, so remote-design work should be bounded
and checkpointed.

## Shared findings adopted for later design

- Invalid default captures must produce policy diagnostics while forwarding input;
  silently disabling a requested binding is not an acceptable configuration UX.
- Disabled and GUI-only actions are excluded from keyboard collision analysis.
- Pass-through entry is invalid unless a configurable, reachable exit and a
  non-keyboard fallback exist.
- Third-party defaults are versioned input data, never hard-coded truth inferred
  from model memory.
- Diagnostic order and wording keys must be deterministic; untrusted IDs,
  sequences, binding counts, and diagnostic counts must be bounded.
- Logical/physical keys, keyboard layouts, Alt/Option/AltGr, legacy aliases, Kitty
  keyboard protocol, and modal application state remain research questions.
- Complexity claims and public signatures require compiled tests and measurements.

## Initial role assignment

| Role | Assignment after calibration | Guardrails |
| --- | --- | --- |
| Codex | Integration, requirements synthesis, repository contracts, release gates | Independent review for every implementation/merge |
| codex-lab | Testability review, black-box plans, regression/release evidence | Must challenge scope semantics and full-result assertions |
| Claude Code | Threat model, unsafe/SSH/IPC/plugin review, failure and rollback critique | Enforce output cap; require citations/tests for upstream and performance claims |
| Qwen | UI/UX, themes, accessibility, site information architecture | No input-policy decision without terminal/compatibility review |
| GLM | Bounded Rust algorithm/test proposals and later small isolated implementation tasks | Compile/test gate plus separate Rust review before broader ownership |
| Fugu | SSH/remote state-machine proposal and failure-recovery review | Small checkpoints, explicit timeout, OpenSSH/source evidence, security review |

These roles are provisional and will change if proposal/review evidence contradicts
this calibration.
