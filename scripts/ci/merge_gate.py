#!/usr/bin/env python3
"""Check whether a pull request is genuinely ready to merge.

Usable by any contributor on any GitHub repository: the owner and name are
discovered with `gh repo view`, and nothing here depends on a particular
reviewer, CI provider detail, or local environment.

`gh pr view` reporting CLEAN is not enough. Reviews are often posted *after* the
checks finish, so a pull request can look mergeable while findings are still in
flight. Several merged changes in this project needed follow-up Issues for
exactly that reason.

Exit 0 only when every check has finished successfully, a review covering the
**current head** has been submitted, and no review thread is unresolved.

Two subtleties this deliberately handles:

* A review of an older head does not count. If the author pushes more commits,
  earlier reviews say nothing about the new code, and the checks for the new head
  can finish before its review arrives.
* Review threads are paged. A partial page can hide an unresolved finding, so
  every page is fetched before deciding.

    merge_gate.py <pr-number>      exit 0 only if the PR is ready to merge
    merge_gate.py --self-test      verify the gate's own policy offline
"""
from __future__ import annotations

import json
import subprocess
import sys

HEAD_QUERY = """
{repository(owner:"%s",name:"%s"){pullRequest(number:%d){
  mergeStateStatus
  headRefOid
  reviews(first:50){nodes{submittedAt commit{oid}}}
  commits(last:1){nodes{commit{statusCheckRollup{contexts(first:50){nodes{
    __typename
    ... on CheckRun{name conclusion status}
    ... on StatusContext{context state}}}}}}}
}}}
"""

THREADS_QUERY = """
{repository(owner:"%s",name:"%s"){pullRequest(number:%d){
  reviewThreads(first:100%s){
    pageInfo{hasNextPage endCursor}
    nodes{isResolved}
}}}}
"""


def repo_slug() -> tuple[str, str]:
    """Discover owner/name from gh so nothing is hard-coded."""
    out = subprocess.run(
        ["gh", "repo", "view", "--json", "nameWithOwner", "-q", ".nameWithOwner"],
        capture_output=True, text=True, check=True).stdout.strip()
    owner, _, name = out.partition("/")
    return owner, name


def graphql(query: str) -> dict:
    out = subprocess.run(["gh", "api", "graphql", "-f", "query=" + query],
                         capture_output=True, text=True, check=True).stdout
    return json.loads(out)["data"]["repository"]["pullRequest"]


def all_threads(owner: str, name: str, pr: int) -> list:
    """Every review thread, following pagination to the end."""
    threads, cursor = [], ""
    while True:
        page = graphql(THREADS_QUERY % (owner, name, pr, cursor))["reviewThreads"]
        threads.extend(page["nodes"])
        info = page["pageInfo"]
        if not info["hasNextPage"]:
            return threads
        cursor = ', after: "%s"' % info["endCursor"]


def evaluate(head, reviews, threads, checks, merge_state):
    """Pure decision function, so the policy is testable without GitHub."""
    on_head = [r for r in reviews if (r.get("commit") or {}).get("oid") == head]
    stale = len(reviews) - len(on_head)
    unresolved = sum(1 for t in threads if not t["isResolved"])
    pending = [c for c in checks if c.get("status") in ("IN_PROGRESS", "QUEUED")]
    failed = [c for c in checks
              if c.get("conclusion") not in (None, "SUCCESS", "NEUTRAL", "SKIPPED")
              or c.get("state") in ("FAILURE", "ERROR")]

    problems = []
    if not checks:
        problems.append("no checks reported yet")
    if pending:
        problems.append("%d check(s) still running" % len(pending))
    if failed:
        problems.append("%d check(s) not successful" % len(failed))
    if not on_head:
        problems.append(
            "NO REVIEW OF THE CURRENT HEAD (%s)%s — wait for it"
            % (head[:7],
               "; %d review(s) cover older heads" % stale if stale else ""))
    if unresolved:
        problems.append("%d unresolved review thread(s) of %d"
                        % (unresolved, len(threads)))
    if merge_state not in ("CLEAN", "UNSTABLE", "HAS_HOOKS"):
        problems.append("mergeStateStatus=%s" % merge_state)
    return problems, len(on_head), len(threads)


def self_test() -> int:
    """Exercise the policy offline. Each case names what it protects against."""
    ok = {"conclusion": "SUCCESS"}
    on_head = [{"commit": {"oid": "abc123"}}]
    cases = [
        ("ready to merge", on_head, [], [ok], "CLEAN", True),
        ("no review at all", [], [], [ok], "CLEAN", False),
        ("review only on an older head", [{"commit": {"oid": "old999"}}], [],
         [ok], "CLEAN", False),
        ("unresolved review thread", on_head, [{"isResolved": False}], [ok],
         "CLEAN", False),
        ("check still running", on_head, [], [{"status": "IN_PROGRESS"}],
         "CLEAN", False),
        ("check failed", on_head, [], [{"conclusion": "FAILURE"}], "CLEAN", False),
        ("no checks reported yet", on_head, [], [], "CLEAN", False),
        ("dirty merge state", on_head, [], [ok], "DIRTY", False),
        ("resolved thread does not block", on_head, [{"isResolved": True}],
         [ok], "CLEAN", True),
    ]
    failures = 0
    for name, reviews, threads, checks, state, want_ready in cases:
        problems, _, _ = evaluate("abc123", reviews, threads, checks, state)
        got_ready = not problems
        if got_ready != want_ready:
            failures += 1
        print("%-4s %-34s ready=%-5s %s"
              % ("ok" if got_ready == want_ready else "FAIL", name, got_ready,
                 "" if got_ready else "(%s)" % problems[0]))
    print("self-test: %d case(s), %d failure(s)" % (len(cases), failures))
    return 1 if failures else 0


def main() -> int:
    if len(sys.argv) == 2 and sys.argv[1] == "--self-test":
        return self_test()
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2

    pr = int(sys.argv[1])
    owner, name = repo_slug()
    p = graphql(HEAD_QUERY % (owner, name, pr))
    head = p["headRefOid"]
    threads = all_threads(owner, name, pr)

    checks = []
    for node in p["commits"]["nodes"]:
        rollup = node["commit"]["statusCheckRollup"]
        if rollup:
            checks = rollup["contexts"]["nodes"]

    problems, on_head_n, thread_n = evaluate(
        head, p["reviews"]["nodes"], threads, checks, p["mergeStateStatus"])

    if problems:
        print("NOT READY (PR #%d): %s" % (pr, "; ".join(problems)))
        return 1
    print("READY (PR #%d): checks green, %d review(s) on head %s, 0 unresolved of %d"
          % (pr, on_head_n, head[:7], thread_n))
    return 0


if __name__ == "__main__":
    sys.exit(main())
