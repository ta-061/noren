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

Three subtleties this deliberately handles:

* A review of an older head does not count. If the author pushes more commits,
  earlier reviews say nothing about the new code, and the checks for the new head
  can finish before its review arrives.
* Review threads are paged. A partial page can hide an unresolved finding, so
  every page is fetched before deciding.
* The aggregate status rollup is checked as well as the returned contexts. A
  context page or a less-common waiting state must not let unfinished CI pass.

    merge_gate.py <pr-number>      exit 0 only if the PR is ready to merge
    merge_gate.py --self-test      verify the gate's own policy offline
"""
from __future__ import annotations

import json
import subprocess
import sys

HEAD_QUERY = """
{
  repository(owner:"%s",name:"%s") {
    pullRequest(number:%d) {
      mergeStateStatus
      headRefOid
      reviews(last:100) {
        nodes { submittedAt commit { oid } }
      }
      commits(last:1) {
        nodes {
          commit {
            statusCheckRollup {
              state
              contexts(first:100) {
                nodes {
                  __typename
                  ... on CheckRun { name conclusion status }
                  ... on StatusContext { context state }
                }
              }
            }
          }
        }
      }
    }
  }
}
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


PENDING_CHECK_STATUSES = {"IN_PROGRESS", "PENDING", "QUEUED", "REQUESTED", "WAITING"}
PENDING_CONTEXT_STATES = {"EXPECTED", "PENDING"}
SUCCESSFUL_CONCLUSIONS = {"SUCCESS", "NEUTRAL", "SKIPPED"}


def check_result(context: dict) -> str:
    """Classify one rollup context, failing closed on unknown values."""
    if context.get("__typename") == "StatusContext" or "state" in context:
        state = context.get("state")
        if state in PENDING_CONTEXT_STATES:
            return "pending"
        return "success" if state == "SUCCESS" else "failed"

    status = context.get("status")
    if status in PENDING_CHECK_STATUSES:
        return "pending"
    if status != "COMPLETED":
        return "failed"
    return (
        "success"
        if context.get("conclusion") in SUCCESSFUL_CONCLUSIONS
        else "failed"
    )


def evaluate(head, reviews, threads, checks, rollup_state, merge_state):
    """Pure decision function, so the policy is testable without GitHub."""
    on_head = [r for r in reviews if (r.get("commit") or {}).get("oid") == head]
    stale = len(reviews) - len(on_head)
    unresolved = sum(1 for t in threads if not t["isResolved"])
    results = [check_result(context) for context in checks]
    pending = results.count("pending")
    failed = results.count("failed")

    problems = []
    if not checks or rollup_state is None:
        problems.append("no checks reported yet")
    elif pending or rollup_state in PENDING_CONTEXT_STATES:
        problems.append("%d check(s) still running" % max(pending, 1))
    if checks and (failed or rollup_state not in ({"SUCCESS"} | PENDING_CONTEXT_STATES)):
        problems.append("%d check(s) not successful" % max(failed, 1))
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
    ok = {"__typename": "CheckRun", "status": "COMPLETED", "conclusion": "SUCCESS"}
    on_head = [{"commit": {"oid": "abc123"}}]
    cases = [
        ("ready to merge", on_head, [], [ok], "SUCCESS", "CLEAN", True),
        ("no review at all", [], [], [ok], "SUCCESS", "CLEAN", False),
        ("review only on an older head", [{"commit": {"oid": "old999"}}], [],
         [ok], "SUCCESS", "CLEAN", False),
        ("unresolved review thread", on_head, [{"isResolved": False}], [ok],
         "SUCCESS", "CLEAN", False),
        ("check still running", on_head, [],
         [{"__typename": "CheckRun", "status": "IN_PROGRESS"}],
         "PENDING", "CLEAN", False),
        ("check waiting", on_head, [],
         [{"__typename": "CheckRun", "status": "WAITING"}],
         "PENDING", "CLEAN", False),
        ("check requested", on_head, [],
         [{"__typename": "CheckRun", "status": "REQUESTED"}],
         "PENDING", "CLEAN", False),
        ("legacy status pending", on_head, [],
         [{"__typename": "StatusContext", "state": "PENDING"}],
         "PENDING", "CLEAN", False),
        ("check failed", on_head, [],
         [{"__typename": "CheckRun", "status": "COMPLETED",
           "conclusion": "FAILURE"}], "FAILURE", "CLEAN", False),
        ("rollup catches omitted failure", on_head, [], [ok], "FAILURE",
         "CLEAN", False),
        ("completed check lacks conclusion", on_head, [],
         [{"__typename": "CheckRun", "status": "COMPLETED",
           "conclusion": None}], "SUCCESS", "CLEAN", False),
        ("no checks reported yet", on_head, [], [], None, "CLEAN", False),
        ("dirty merge state", on_head, [], [ok], "SUCCESS", "DIRTY", False),
        ("resolved thread does not block", on_head, [{"isResolved": True}],
         [ok], "SUCCESS", "CLEAN", True),
    ]
    failures = 0
    for name, reviews, threads, checks, rollup, state, want_ready in cases:
        problems, _, _ = evaluate(
            "abc123", reviews, threads, checks, rollup, state
        )
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
    rollup_state = None
    for node in p["commits"]["nodes"]:
        rollup = node["commit"]["statusCheckRollup"]
        if rollup:
            rollup_state = rollup["state"]
            checks = rollup["contexts"]["nodes"]

    problems, on_head_n, thread_n = evaluate(
        head, p["reviews"]["nodes"], threads, checks, rollup_state,
        p["mergeStateStatus"])

    if problems:
        print("NOT READY (PR #%d): %s" % (pr, "; ".join(problems)))
        return 1
    print("READY (PR #%d): checks green, %d review(s) on head %s, 0 unresolved of %d"
          % (pr, on_head_n, head[:7], thread_n))
    return 0


if __name__ == "__main__":
    sys.exit(main())
