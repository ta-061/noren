#!/usr/bin/env python3
"""Check whether a PR is genuinely ready to merge.

`gh pr view` reporting CLEAN is not enough: automated reviewers post *after* the
checks finish, so a PR can look mergeable while findings are still in flight.
Three PRs in one session needed follow-up Issues because of exactly that.

Exit 0 only when every required check passed, a review has actually been
submitted, and no review thread is unresolved.

    merge_gate.py <pr-number>
"""
import json
import subprocess
import sys

QUERY = """
{repository(owner:"ta-061",name:"noren"){pullRequest(number:%d){
  mergeStateStatus
  reviews(first:20){nodes{submittedAt}}
  reviewThreads(first:50){nodes{isResolved}}
  statusCheckRollup: commits(last:1){nodes{commit{statusCheckRollup{
    contexts(first:20){nodes{__typename
      ... on CheckRun{name conclusion status}
      ... on StatusContext{context state}}}}}}}
}}}
"""


def main():
    if len(sys.argv) != 2:
        print("usage: merge_gate.py <pr-number>", file=sys.stderr)
        return 2
    pr = int(sys.argv[1])
    out = subprocess.run(
        ["gh", "api", "graphql", "-f", "query=" + QUERY % pr],
        capture_output=True, text=True, check=True).stdout
    p = json.loads(out)["data"]["repository"]["pullRequest"]

    reviews = len(p["reviews"]["nodes"])
    unresolved = sum(1 for t in p["reviewThreads"]["nodes"] if not t["isResolved"])
    state = p["mergeStateStatus"]

    checks = []
    for node in p["statusCheckRollup"]["nodes"]:
        rollup = node["commit"]["statusCheckRollup"]
        if rollup:
            checks = rollup["contexts"]["nodes"]
    pending = [c for c in checks
               if c.get("status") in ("IN_PROGRESS", "QUEUED")]
    failed = [c for c in checks
              if c.get("conclusion") not in (None, "SUCCESS", "NEUTRAL", "SKIPPED")
              or c.get("state") in ("FAILURE", "ERROR")]

    problems = []
    if pending:
        problems.append("%d check(s) still running" % len(pending))
    if failed:
        problems.append("%d check(s) not successful" % len(failed))
    if reviews == 0:
        problems.append("NO REVIEW SUBMITTED YET — wait for it")
    if unresolved:
        problems.append("%d unresolved review thread(s)" % unresolved)
    if state not in ("CLEAN", "UNSTABLE", "HAS_HOOKS"):
        problems.append("mergeStateStatus=%s" % state)

    if problems:
        print("NOT READY (PR #%d): %s" % (pr, "; ".join(problems)))
        return 1
    print("READY (PR #%d): checks green, %d review(s) submitted, 0 unresolved"
          % (pr, reviews))
    return 0


if __name__ == "__main__":
    sys.exit(main())
