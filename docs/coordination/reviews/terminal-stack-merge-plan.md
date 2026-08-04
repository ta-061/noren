# Terminal stack merge plan — codex-lab

## Recommended mechanism

**a** because the project requires an auditable landing record for every PR.
Merge the PRs bottom-up as individual GitHub merge commits, retargeting only the
next PR to `main` after its predecessor is present on `main`:

`#21 -> #23 -> #31 -> #29 -> #30 -> #32 -> #33`

Use GitHub's **Create a merge commit** mechanism (`gh pr merge NUMBER --merge`),
not squash or rebase. A merge commit retains the tested branch tips as ancestors
of `main`. Consequently, retargeting the next stacked PR removes already-landed
commits from that PR's effective diff without rewriting its head. Do not update
all bases at once, enable auto-merge for the whole stack, or delete a source
branch before its dependants have landed.

For each PR, the release agent must do the following and stop on any mismatch:

1. Fetch `main` and the PR head. Confirm the head OID is the one in the table
   below. A changed head invalidates the corresponding merge-tree result.
2. For #21, retain `base=main`. For every later PR, change only that PR's base
   to `main` after the preceding PR has merged.
3. Confirm the retargeted diff contains only that PR's intended unique commits,
   the PR has owner approval and is no longer Draft, `mergeable=MERGEABLE`,
   `mergeStateStatus=CLEAN`, and required checks are successful for the current
   head/base combination. If a retarget or status edit causes a new check run,
   wait for it rather than relying on an older run.
4. Merge with a merge commit. Fetch `main`, confirm the PR reports `MERGED`, and
   only then retarget the next PR.

Before #33 is merged, update its `docs/coordination/status.md` to the
post-landing target record in this plan. That commit changes #33's head, so the
`d9abe63` result below is evidence only for the currently reviewed head. Run
`git merge-tree --write-tree origin/main origin/agent/terminal-parallel-docs`
again at the final #33 head and require fresh successful checks. Do not merge
#33 while the file still calls #19 unmerged or calls #21-#33 open Drafts.

Mechanism **b** has fewer PR operations, but merging cumulative tip #30 first
would bring #21, #23, #31, and #29 onto `main` through #30's landing. Those PRs
would have no distinct per-PR landing merge, weakening the link from their
reviews and checks to repository history. Mechanism **c** centralizes conflict
resolution and CI on one integration change, but a one-commit landing discards
the seven independently reviewed landing records. Both mechanisms reduce
review traceability for little conflict benefit, so they are rejected.

The current GitHub facts in this plan are the verified facts supplied for this
release task, sanity-checked against the local `origin/*` refs and ancestry. A
live read-only refresh was attempted with `gh pr list` and `gh issue list`; both
returned `error connecting to api.github.com`. Therefore, the release agent
must repeat the live state queries from a networked environment immediately
before landing #33 and must not infer Issue closure merely from a merged PR.

## Verified merge order

Verification used a disposable, no-hardlink local clone so that the real
worktree, branches, PR bases, and remote were untouched. The setup actually run
was:

```sh
scratch_dir=$(mktemp -d /private/tmp/noren-terminal-stack-verify.XXXXXX)
git clone --quiet --no-hardlinks --local . "$scratch_dir"
cd "$scratch_dir"
git config user.name codex-lab
git config user.email codex-lab@example.invalid
git config commit.gpgsign false
git switch --quiet --detach origin/main
```

At each step, `git merge-tree --write-tree` predicted the result, a scratch
`git merge --no-ff --no-edit` advanced the simulated `main`, the actual merge
tree was compared with the predicted tree, and `git diff --check HEAD^1 HEAD`
was run. Synthetic scratch merge commit OIDs are intentionally omitted because
they are timestamp-dependent; branch heads, merge bases, and tree OIDs are
stable evidence.

| Step | Action | Command run | Result |
|---|---|---|---|
| 1 | Land #21 (`2543f18a06ed492f01e2336bcd42ca06a1cd6c6c`) on `main` | `git merge-tree --write-tree HEAD origin/agent/terminal-scroll-regions`<br>`git -c commit.gpgsign=false merge --no-ff --no-edit origin/agent/terminal-scroll-regions`<br>`git diff --check HEAD^1 HEAD` | Merge base `c695920d8bc99990447d0b451754ea96c91181fc`; merge-tree exit 0, output `468d19fcc9efe141c810d828a161df53f249bc39`; scratch merge exit 0 with `Merge made by the 'ort' strategy.`; actual tree matched; diff-check exit 0. |
| 2 | Retarget and land #23 (`c6a1e3fef469c09e243a0b5cc88c2bee2aedddb7`) | `git merge-tree --write-tree HEAD origin/agent/terminal-alternate-screen`<br>`git -c commit.gpgsign=false merge --no-ff --no-edit origin/agent/terminal-alternate-screen`<br>`git diff --check HEAD^1 HEAD` | Merge base `2543f18a06ed492f01e2336bcd42ca06a1cd6c6c`; merge-tree exit 0, output `8fb34d46ae30a22df8f1c8cfdd24293344996230`; scratch merge exit 0 with `Merge made by the 'ort' strategy.`; actual tree matched; diff-check exit 0. |
| 3 | Retarget and land #31 (`a630c93605e309c2fd23558c8807500ac12a684e`) | `git merge-tree --write-tree HEAD origin/agent/terminal-erase-ops`<br>`git -c commit.gpgsign=false merge --no-ff --no-edit origin/agent/terminal-erase-ops`<br>`git diff --check HEAD^1 HEAD` | Merge base `c6a1e3fef469c09e243a0b5cc88c2bee2aedddb7`; merge-tree exit 0, output `2634bb6ba2e0164c1a8523f371d3a3d0033c1548`; scratch merge exit 0 with `Merge made by the 'ort' strategy.`; actual tree matched; diff-check exit 0. |
| 4 | Retarget and land #29 (`0daa7d6aff2dbcdc547358288346a9804fa35011`) | `git merge-tree --write-tree HEAD origin/agent/terminal-sgr-attributes`<br>`git -c commit.gpgsign=false merge --no-ff --no-edit origin/agent/terminal-sgr-attributes`<br>`git diff --check HEAD^1 HEAD` | Merge base `a630c93605e309c2fd23558c8807500ac12a684e`; merge-tree exit 0, output `82a10b435aa5696623eef7aa42159fc6edb5f3c3`; scratch merge exit 0 with `Merge made by the 'ort' strategy.`; actual tree matched; diff-check exit 0. |
| 5 | Retarget and land #30 (`fd1ea69584acbfdf2d0c08debbd148989f3f9f6b`) | `git merge-tree --write-tree HEAD origin/agent/application-modes`<br>`git -c commit.gpgsign=false merge --no-ff --no-edit origin/agent/application-modes`<br>`git diff --check HEAD^1 HEAD` | Merge base `0daa7d6aff2dbcdc547358288346a9804fa35011`; merge-tree exit 0, output `c3f684f255d773d1c37a7ad01d4b4dcf6b7886b6`; scratch merge exit 0 with `Merge made by the 'ort' strategy.`; actual tree matched; diff-check exit 0. |
| 6 | Retarget and land #32 (`c03e8b30ec82597b32b597b7b8961c30d61c6556`) after cumulative #30 | `git merge-tree --write-tree HEAD origin/agent/vt-compat-suite`<br>`git -c commit.gpgsign=false merge --no-ff --no-edit origin/agent/vt-compat-suite`<br>`git diff --check HEAD^1 HEAD` | Divergent merge base `c6a1e3fef469c09e243a0b5cc88c2bee2aedddb7`; merge-tree exit 0, output `5377385265c420881397793bce36c2709ae8cf78`; scratch merge exit 0 with `Merge made by the 'ort' strategy.`; actual tree matched; diff-check exit 0. |
| 7 | Retarget and land #33 (`d9abe6365fe81e04ef8d688c18becaab68445ef2`) after #32 | `git merge-tree --write-tree HEAD origin/agent/terminal-parallel-docs`<br>`git -c commit.gpgsign=false merge --no-ff --no-edit origin/agent/terminal-parallel-docs`<br>`git diff --check HEAD^1 HEAD` | Divergent merge base `c6a1e3fef469c09e243a0b5cc88c2bee2aedddb7`; merge-tree exit 0, output `86a87bc05b45fc978d3fe604f7e72a651b16ee70`; scratch merge exit 0 with `Merge made by the 'ort' strategy.`; actual tree matched; diff-check exit 0. Re-run after the required status-only commit changes this head. |

The final simulated tree was
`86a87bc05b45fc978d3fe604f7e72a651b16ee70`. Cleanup actually reported
`scratch cleanup -> removed
/private/tmp/noren-terminal-stack-verify.2mB3uv`.

## Conflicts found

None at the seven exact heads and in the order above. All seven merge-tree
commands, all seven scratch merges, and all seven whitespace checks exited 0;
no conflict file or hunk was emitted. Conflict count: **0**.

This evidence is head- and order-specific. Any changed branch head, intervening
`main` commit, different order, squash/rebase, or required update-branch merge
invalidates that step and every later synthetic step. Re-run from the new
`origin/main`; if `git merge-tree --write-tree` exits nonzero, record its
`CONFLICT (content)` output and do not merge.

## Required status.md correction

PR #33 must be the final reconciliation PR. After #32 is merged, query the live
state and record the pre-#33 `main` head and current #33 head as release
evidence:

```sh
gh pr list --repo ta-061/noren --state open \
  --json number,title,isDraft,baseRefName,headRefName
gh issue list --repo ta-061/noren --state open --json number,title
for pr in 19 21 23 31 29 30 32 33; do
  gh pr view "$pr" --repo ta-061/noren \
    --json number,state,mergedAt,mergeCommit,headRefOid
done
git fetch origin main agent/terminal-parallel-docs
git rev-parse origin/main origin/agent/terminal-parallel-docs
```

The PR template uses `Closes #`, so the expected post-merge state is that Issues
#20, #22, and #24-#28 are closed. That expectation is not proof. Immediately
after #33 merges, repeat the PR and Issue queries. If any Issue remains open, do
not close it without authority and do not claim there are no open Issues; land
a status-only follow-up that lists its actual state before calling coordination
complete.

Replace the entire `docs/coordination/status.md` with the text below on #33.
Before committing, replace `LANDING_DATE` with the intended landing date; no
placeholder may land. If the live query finds unrelated open PRs or Issues,
replace the “none open” sentence with an explicit numbered list of them. The
text deliberately does not try to contain #33's eventual merge commit or its
own final head OID: neither value can be embedded in the commit that creates
it. Capture both in post-merge release evidence or a later status-only update.

```markdown
# Coordination status

Last updated: LANDING_DATE (Asia/Tokyo), for the completed terminal
compatibility stack. PR
[#19](https://github.com/ta-061/noren/pull/19) previously merged the
renderer-independent Terminal Core foundation as
`c695920d8bc99990447d0b451754ea96c91181fc`. PRs
[#21](https://github.com/ta-061/noren/pull/21),
[#23](https://github.com/ta-061/noren/pull/23),
[#31](https://github.com/ta-061/noren/pull/31),
[#29](https://github.com/ta-061/noren/pull/29),
[#30](https://github.com/ta-061/noren/pull/30),
[#32](https://github.com/ta-061/noren/pull/32), and
[#33](https://github.com/ta-061/noren/pull/33) then merged individually, in
that order. None of those PRs remains open or Draft.

## Current phase

Terminal foundation, bounded VT-compatibility baseline. The merged stack adds
scrolling regions, primary/alternate screen ownership, erase/insert/delete
operations, bounded SGR cell attributes, application cursor/keypad mode state
and input encoding, a bounded VT compatibility harness, and the documented
parallel-development process. See
[terminal core foundation](https://github.com/ta-061/noren/blob/main/docs/architecture/terminal-core-foundation.md).

This is not a claim of complete VT100/xterm, vim, tmux, or Zellij
compatibility. Non-ASCII glyph quality, full Unicode/IME/accessibility, Linux,
SSH, agent integration, tabs, panes, themes, persistence, and a remote daemon
remain behind their existing roadmap and risk gates.

## GitHub state

Verified after PR #33 merged:

- PR #19 and Issue [#18](https://github.com/ta-061/noren/issues/18) are closed
  and complete.
- PRs #21, #23, #31, #29, #30, #32, and #33 are merged. Their Issues
  [#20](https://github.com/ta-061/noren/issues/20),
  [#22](https://github.com/ta-061/noren/issues/22),
  [#24](https://github.com/ta-061/noren/issues/24),
  [#25](https://github.com/ta-061/noren/issues/25),
  [#26](https://github.com/ta-061/noren/issues/26),
  [#27](https://github.com/ta-061/noren/issues/27), and
  [#28](https://github.com/ta-061/noren/issues/28) are closed and complete.
- No PR or Issue is open.

## Terminal stack evidence

| Order | PR / Issue | Saved head | Delivered evidence | State |
| --- | --- | --- | --- | --- |
| Foundation | #19 / #18 | `05fb148` | Noren-owned terminal state, public-API tests, architecture review, exact-head CI; merged as `c695920` | Complete |
| 1 | #21 / #20 | `2543f18a06ed492f01e2336bcd42ca06a1cd6c6c` | Scrolling regions; core, eight public-API regressions, documentation, review, and both required checks | Merged |
| 2 | #23 / #22 | `c6a1e3fef469c09e243a0b5cc88c2bee2aedddb7` | Alternate screen and mode 1049; seven public-API regressions, documentation, review, and both required checks | Merged |
| 3 | #31 / #24 | `a630c93605e309c2fd23558c8807500ac12a684e` | Erase/insert/delete operations and regression coverage; both required checks | Merged |
| 4 | #29 / #25 | `0daa7d6aff2dbcdc547358288346a9804fa35011` | Bounded SGR and cell attributes, regression coverage, independent review, and both required checks | Merged |
| 5 | #30 / #26 | `fd1ea69584acbfdf2d0c08debbd148989f3f9f6b` | Application cursor/keypad modes and input encoders; 96 cumulative workspace tests, independent review, and both required checks | Merged |
| 6 | #32 / #27 | `c03e8b30ec82597b32b597b7b8961c30d61c6556` | Bounded VT compatibility harness, independent review, and both required checks | Merged |
| 7 | #33 / #28 | `d9abe6365fe81e04ef8d688c18becaab68445ef2` plus the final status commit in PR #33 | Parallel-development documentation and final status reconciliation | Merged |

The stack was landed with one merge commit per PR. The merge plan,
pre-landing merge-tree evidence, and mandatory post-merge gate are recorded in
[terminal stack merge plan](https://github.com/ta-061/noren/blob/main/docs/coordination/reviews/terminal-stack-merge-plan.md).

## Human decisions still required

No repository access control was changed by this stack. The owner still must
separately decide branch protection and required-CI policy, macOS
signing/notarization identity, and the public support/security contact before
Preview publication.

## Next steps

1. Use the bounded compatibility harness to drive evidence-backed vim checks,
   then tmux/Zellij checks, without advertising compatibility before their
   acceptance gates pass.
2. Keep SSH, agent integration, and remaining terminal behavior in separate,
   scoped Issues and PRs.
```

If the project wants `status.md` itself to record the post-merge command
results, add that result only after the commands below really pass on final
`main`, through a separately reviewed status-only PR. Never record anticipated
evidence as a result.

## Post-merge verification gate

Run from a clean checkout of the final `origin/main`. All commands must exit 0
before the stack or the status record is called complete:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 scripts/check_docs.py
python3 -m unittest scripts/test_check_docs.py
```

Record the final `git rev-parse HEAD`, tool versions, test count, and exact
output in the release evidence. The 96-test result belongs to the pre-merge #30
tip and is not a substitute for this final-main gate.
