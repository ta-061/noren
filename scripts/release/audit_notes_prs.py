#!/usr/bin/env python3
"""Audit the release notes' pull-request accounting against GitHub.

`gh pr list --state merged` is the authority on how many pull requests
landed. generate_notes.py works offline from git alone and therefore cannot
know, for example, that a trailing "(#59)" is an issue rather than a pull
request, or which of a stack's "(subsumes #N, ...)" PRs GitHub marks MERGED.
Those two cases live in the generator's INLINE_ISSUE_REFS and
SUBSUMED_MERGED tables; this script is what keeps those tables honest.

It recomputes, at a head, exactly the PR set the notes would print, then
diffs it against every gh-known merged pull request (any base) whose
mergeCommit is reachable from that head — reachability, not a base-branch
filter, is what "landed in this history" means, and it correctly excludes a
PR merged into main after the branch point.

Exit status is 0 only when the two sets are identical. A difference is
either a generator bug (a merge shape the pattern still misses — the class
of bug that once dropped 37 coordinator merges) or a drifted table; the fix
is to correct the pattern or table, never to widen this audit. Network and
`gh` authentication are required: this is an on-demand audit, deliberately
not part of the offline build or CI.

    audit_notes_prs.py [--head <rev>]
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from scripts.release import generate_notes as gn  # noqa: E402


def offline_pr_set(head: str) -> set[int]:
    """The PR set the notes print at head: merges + inline refs + subsumed."""
    pr_map, subsumed = gn.build_pr_map(head)
    inline: set[int] = set()
    for commit in gn.load_commits(head):
        _commit_type, _scope, ref = gn.parse_subject(commit["subject"])
        if ref is not None and ref not in gn.INLINE_ISSUE_REFS:
            inline.add(ref)
    return set(pr_map.values()) | inline | subsumed


def gh_merged_prs() -> dict[int, str | None]:
    """Every gh-known merged pull request (any base) -> mergeCommit oid."""
    completed = subprocess.run(
        [
            "gh",
            "pr",
            "list",
            "--state",
            "merged",
            "--limit",
            "300",
            "--json",
            "number,mergeCommit",
        ],
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise SystemExit(
            "audit_notes_prs.py: gh pr list failed "
            f"(rc={completed.returncode}): {completed.stderr.strip()}"
        )
    return {
        entry["number"]: (entry.get("mergeCommit") or {}).get("oid")
        for entry in json.loads(completed.stdout)
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--head", default="HEAD", help="commit to audit")
    args = parser.parse_args()

    head = gn.git("rev-parse", args.head).strip()
    notes_prs = offline_pr_set(head)

    merged = gh_merged_prs()
    reachable: set[int] = set()
    unreachable: dict[int, str | None] = {}
    for number, merge_commit in merged.items():
        if not merge_commit:
            unreachable[number] = "no mergeCommit recorded"
            continue
        is_ancestor = subprocess.run(
            ["git", "merge-base", "--is-ancestor", merge_commit, head],
            cwd=gn.ROOT,
        ).returncode
        if is_ancestor == 0:
            reachable.add(number)
        else:
            unreachable[number] = "mergeCommit not reachable from head"

    missing = sorted(reachable - notes_prs)
    phantom = sorted(notes_prs - reachable)
    print(f"head={head[:7]} notes_prs={len(notes_prs)} gh_merged={len(merged)} "
          f"gh_merged_reachable={len(reachable)}")
    print(f"not_reachable_at_head (merged elsewhere/later): "
          f"{sorted(unreachable)}")
    if missing:
        print(f"MISSING from the notes (gh-merged and reachable but not "
              f"attributed): {missing}")
    if phantom:
        print(f"PHANTOM in the notes (attributed but not gh-merged/reachable): "
              f"{phantom}")
    if missing or phantom:
        print("audit_notes_prs.py: FAIL — fix the generator pattern or the "
              "INLINE_ISSUE_REFS/SUBSUMED_MERGED tables; never widen the audit")
        return 1
    print("audit_notes_prs.py: OK — notes PR set equals gh-merged-and-reachable")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
