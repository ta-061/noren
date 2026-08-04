#!/usr/bin/env python3
"""Check whether a PR is genuinely ready to merge.

`gh pr view` reporting CLEAN is not enough: automated reviewers post *after* the
checks finish, so a PR can look mergeable while findings are still in flight.
Three PRs in one session needed follow-up Issues because of exactly that.

Exit 0 only when every required check passed, a review covering the CURRENT head
has been submitted, and no review thread is unresolved.

Two subtleties this deliberately handles:

* A review of an older head does not count. If the author pushes more commits,
  earlier reviews say nothing about the new code, and the checks for the new head
  can finish before its review arrives — the exact race this script exists to
  prevent.
* Review threads are paged. A partial page can hide an unresolved finding, so
  every page is fetched before deciding.

    merge_gate.py <pr-number>
"""
import json
import subprocess
import sys

HEAD_QUERY = """
{repository(owner:"ta-061",name:"noren"){pullRequest(number:%d){
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
{repository(owner:"ta-061",name:"noren"){pullRequest(number:%d){
  reviewThreads(first:100%s){
    pageInfo{hasNextPage endCursor}
    nodes{isResolved}
}}}}
"""


def graphql(query):
    out = subprocess.run(["gh", "api", "graphql", "-f", "query=" + query],
                         capture_output=True, text=True, check=True).stdout
    return json.loads(out)["data"]["repository"]["pullRequest"]


def all_threads(pr):
    """Every review thread, following pagination to the end."""
    threads, cursor = [], ""
    while True:
        page = graphql(THREADS_QUERY % (pr, cursor))["reviewThreads"]
        threads.extend(page["nodes"])
        info = page["pageInfo"]
        if not info["hasNextPage"]:
            return threads
        cursor = ', after: "%s"' % info["endCursor"]


def main():
    if len(sys.argv) != 2:
        print("usage: merge_gate.py <pr-number>", file=sys.stderr)
        return 2
    pr = int(sys.argv[1])
    p = graphql(HEAD_QUERY % pr)

    head = p["headRefOid"]
    # Only reviews of the current head say anything about the code being merged.
    on_head = [r for r in p["reviews"]["nodes"]
               if (r.get("commit") or {}).get("oid") == head]
    stale = len(p["reviews"]["nodes"]) - len(on_head)

    threads = all_threads(pr)
    unresolved = sum(1 for t in threads if not t["isResolved"])

    checks = []
    for node in p["commits"]["nodes"]:
        rollup = node["commit"]["statusCheckRollup"]
        if rollup:
            checks = rollup["contexts"]["nodes"]
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
        problems.append("NO REVIEW OF THE CURRENT HEAD (%s)%s — wait for it"
                        % (head[:7],
                           "; %d review(s) cover older heads" % stale if stale else ""))
    if unresolved:
        problems.append("%d unresolved review thread(s) of %d"
                        % (unresolved, len(threads)))
    if p["mergeStateStatus"] not in ("CLEAN", "UNSTABLE", "HAS_HOOKS"):
        problems.append("mergeStateStatus=%s" % p["mergeStateStatus"])

    if problems:
        print("NOT READY (PR #%d): %s" % (pr, "; ".join(problems)))
        return 1
    print("READY (PR #%d): checks green, %d review(s) on head %s, 0 unresolved of %d"
          % (pr, len(on_head), head[:7], len(threads)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
