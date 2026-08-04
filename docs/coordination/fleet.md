# Agent fleet organization

Noren is developed by a fleet of CLI agents working parallel lanes on separate
branches. This file is the operating contract: who owns what, how work is
dispatched, and what happens when an engine runs out of quota.

## Roles

The Claude Code session is the **coordinator**. It assigns lanes, reads
results, decides merges, and deliberately spends few tokens: it does not write
feature code while a lane can do it.

| Lane prefix | Engine | Quota account | Owns | Host |
| --- | --- | --- | --- | --- |
| `glm-*` | GLM 5.2 via opencode | `glm-main` | `noren-terminal` core: state machine, parser, VT correctness | local |
| `qwen-*` | Qwen3.8-max via opencode | `qwen-main` | `noren-app`: window, renderer, input encoding | local |
| `lab-*` | codex-lab (`gpt-5.6-sol`) | `codex-main` | Integration, merge mechanics, evidence integrity, test strategy | local |
| `kimi-*` | Kimi Code CLI | `kimi-main` | Adversarial robustness: hostile input, panics, unbounded growth | kali over SSH |
| `codex-*` | Codex (`gpt-5.6-sol`) | `codex-tatu` | Reserve integrator; historically the lead | local |

Lanes are scoped so two engines never review the same code. That keeps findings
independent and makes disagreement meaningful rather than duplicated.

Only Kimi works over SSH, against the clone at
`/home/matsulab/tatuya/apps/noren` on `kali`. Every other lane runs locally in a
git worktree under `../noren-worktrees/`.

## Quota-aware dispatch

`scripts/fleet/quota.py` reads the local Agent Quota portal at
`http://192.168.50.63:5171/api/overview`. `--gate <account>` exits non-zero when
an account is unauthenticated, erroring, or below `FLOOR_PERCENT` remaining.

`scripts/fleet/dispatch.sh <lane> <worktree> <prompt-file>` runs one lane:

1. Gates on that lane's quota account. An exhausted engine exits **3** without
   starting work, so a lane is never abandoned half-finished.
2. Reuses the lane's persistent session from `.fleet/sessions/<lane>` so the
   same role keeps one continuous conversation across days and quota resets.
3. Logs to `.fleet/logs/<lane>.<timestamp>.log`.

`.fleet/` is runtime state and is gitignored; only this contract and the lane
prompts under `.fleet/prompts/` describe intent.

## Failover and resumption

When a lane exits 3 (no headroom), its work moves to the backup below and the
original lane resumes when its window resets — the reset time is in the quota
readout, so a resumption can be scheduled rather than polled.

| Exhausted | Backup | Note |
| --- | --- | --- |
| `glm-*` | `lab-*`, then `kimi-*` | Core review needs Rust depth |
| `qwen-*` | `glm-*` | App layer is smaller; core lane can absorb it |
| `lab-*` | `codex-*` (`codex-takashi`/`codex-main`) | Same model family, keeps integration judgment consistent |
| `kimi-*` | `glm-*` | Adversarial tests are portable; only the host changes |

Because a lane's session ID persists, a handed-off task returns to its original
owner with full context intact rather than restarting.

## Budget policy

The whole Codex family is weekly-limited and slow to recover, so it is a reserve
rather than a workhorse:

- **GLM and Qwen carry the routine load** — review, implementation, tests, docs.
  Their weekly budgets are near-untouched and their 5-hour windows recover fast.
- **Kimi** takes adversarial and robustness work.
- **codex-lab (`codex-main`) is a scarce reserve.** Its weekly window takes
  ~3.5 days to reset. Dispatch it only for integration judgment that genuinely
  needs it — merge mechanics, evidence integrity, release gating — one scoped
  task at a time, not as a standing lane.
- **codex (`codex-tatu`) is nearly exhausted** (~10%): it was drawn down running
  the Terminal Core stack, which is what forced the coordinator handoff. Leave it
  alone until it resets.

Check `scripts/fleet/quota.py` before every dispatch and prefer the engine with
the most headroom for any task that does not require a specific model.

The coordinator stays light on purpose: delegate, read results, decide. It does
not implement what a lane can implement.

## Deliberation

For direction-setting, lanes are asked the same scoped question in their own
persistent sessions and answer independently. The coordinator integrates in this
order: correctness, data-loss prevention, security, compatibility,
verifiability, maintainability, feasibility, performance, extensibility.

Consensus is not required. Unanimity is not a goal, and the coordinator closes a
question once the evidence is sufficient rather than continuing to poll lanes.
Dissent is recorded in the relevant review file instead of being resolved away.

## Non-negotiable evidence rule

A lane may never report a command result it did not run, and nothing is marked
complete on unproduced evidence. This predates the fleet and still governs it:
only evidence-backed work is marked complete.
