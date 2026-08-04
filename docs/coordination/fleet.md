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

### Running several lanes per engine

A lane name is a prefix plus a suffix (`glm-core`, `glm-b`, `qwen-c`), and each
suffix gets its own persistent session, so one engine can run several concurrent
lanes. GLM and Qwen recover their 5-hour windows in minutes and hold ~93% and
~99% weekly, so they are the engines to scale — running review, implementation,
tests, and analysis at the same time rather than in sequence.

Concurrent lanes take a worktree each from the pool at
`../noren-worktrees/pool-p*`, all branched from `main`. **Assign file ownership
per lane before dispatching** — two lanes editing one file is the failure this
pool exists to prevent. When overlap is genuinely unavoidable (two input-encoding
stages, say), accept it deliberately and resolve by merge order, landing one and
rebasing the other.

Kimi is the opposite case: it works, but its 5-hour window empties after roughly
one substantial task and takes hours to return. Spend it on independent
verification bursts rather than standing work — its one sweep found two real
parser bugs that the GLM sweep had missed.

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

### The merge gate

`scripts/fleet/merge_gate.py <pr>` exits 0 only when every required check passed,
**a review has actually been submitted**, and no review thread is unresolved. Run
it before every merge.

The review condition is the one that matters. Automated reviewers post *after* the
checks finish, so a PR can report `CLEAN` while findings are still in flight.
Three PRs in one session needed follow-up Issues because they were merged, or
nearly merged, on green CI alone:

| PR | What review caught that CI could not |
| --- | --- |
| #52 | The premise was wrong — output-side `CSI M` is Delete Line, not a mouse report. The "fix" would have broken legitimate DL. |
| #53 | `lines()` collapsed the second column of every wide character, so the renderer misaligned — the exact breakage the PR fixed at the state layer. |
| #56 | Combined `fg;bg` truecolor needs 10 parameter slots against a cap of 8, so both colors were silently lost — the ordinary emission pattern. |

None would have been caught by more tests. They were wrong premises and untested
*combinations*, not wrong code. The recurring coordinator failure was verifying
each case in isolation and never the combination.

### Dispatch pitfalls

Three failure modes cost real time and are worth knowing:

- **opencode's `external_directory` permission stalls a lane silently.** Running
  a lane in a worktree outside the project root can trigger an interactive
  permission prompt that a non-interactive run can never answer; the log simply
  stops growing at `message=asking`. Check for a stalled log size, and prefer
  running a lane in a worktree the engine already treats as its project, or
  pre-approve the path.
- **A backgrounded dispatch has stdin on `/dev/null`.** A heredoc piped into
  `ssh` therefore delivers nothing. Copy the prompt with `scp` and use `ssh -n`.
- **Never pass a prompt as `-p "$(cat file)"` across `ssh`.** Prompts contain
  markdown backticks, which the remote shell re-expands as command
  substitution — the run produces empty output with exit 0. Feed the prompt on
  stdin instead.

- **A lane can die during `init`, before it ever reaches the model.** The
  symptom is a log that stops at `message=init` and never grows past roughly
  1.4 KB, with no session id recorded and no branch created. It appears when
  several opencode instances start at once. Distinguish it from a slow start by
  checking whether a branch exists and whether the log is still growing; a lane
  killed this way has produced nothing, so relaunching it costs only time.

Distinguishing the two stall modes matters:

| Symptom | Meaning | Action |
| --- | --- | --- |
| Log frozen at `message=init`, ~1.4 KB, no branch | died before starting | relaunch, nothing lost |
| Log frozen at `message=asking`, large, branch exists | blocked on a permission prompt | inspect the branch — the work is usually **already done** |
| Log still growing | working, just slow | leave it alone |

A lane that stalls after starting has usually still done useful work. Inspect its
worktree and commits before discarding the run — that has been true every time so
far, including for two lanes whose complete, correct fixes were sitting
uncommitted or committed-but-unreported.

Uncommitted changes are worth reading rather than discarding for a second reason:
a lane fixing behavior often has to update a *sibling* test suite that asserted
the old behavior, and may leave that edit unstaged. Deleting it silently
reintroduces a failing gate.

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
