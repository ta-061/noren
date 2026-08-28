#!/usr/bin/env python3
"""Generate Noren release notes from the merged git history.

The notes cannot drift from what shipped because they are derived from
`git log` output, not written by hand: every listed line is a verbatim commit
subject with its short SHA and, where derivable, the pull request that landed
it. Nothing here invents prose about the product; the fixed banner at the top
carries the D-M8-001 framing instead.

Grouping is by what a user would notice (terminal emulation, PTY, SSH,
workspace app, rendering, configuration), never by commit order.

Determinism: running this twice at the same `--head` produces a byte-identical
file (the printed date is the head commit's committer date, not the clock).

    generate_notes.py [--head <rev>] [--output <path>] [--version <label>]

Default output: docs/release/notes/<version>.md relative to the repository
root, regardless of the current working directory.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]

# Commits whose type alone says "a user notices this".
USER_VISIBLE_TYPES = {"feat", "fix", "perf"}
# Scopes that describe internal quality work even when the type is fix/feat;
# those land in the counted "not user-visible" bucket instead of the lists.
INTERNAL_SCOPES = {"test", "bench", "ci", "fleet"}
# Legacy one-off subjects from merged lanes that wrote the scope in the type
# position (`theme: built-in palettes ...`); mapped to the visible type their
# content actually is so the theme work is listed, not counted away.
TYPE_ALIASES = {"theme": "feat", "config": "feat", "renderer": "feat"}

# Scope -> the area a reader looks under. Every scope observed in this
# repository's history is mapped; an unmapped scope is a loud error, not a
# silent "Other", so the grouping cannot quietly rot as scopes are added.
AREA_BY_SCOPE = {
    "app": "Workspace app and window",
    "ui": "Workspace app and window",
    "workspace": "Workspace app and window",
    "sidebar": "Workspace app and window",
    "session": "Workspace app and window",
    "palette": "Workspace app and window",
    "persistence": "Workspace app and window",
    "terminal": "Terminal emulation core",
    "snapshot": "Terminal emulation core",
    "pty": "PTY and process layer",
    "ssh": "SSH sidebar",
    "ssh_config": "SSH sidebar",
    "renderer": "Rendering and themes",
    "theme": "Rendering and themes",
    "config": "Configuration and diagnostics",
    "diagnostics": "Configuration and diagnostics",
}

# Area display order (areas with no entries are omitted).
AREA_ORDER = [
    "Workspace app and window",
    "Terminal emulation core",
    "PTY and process layer",
    "SSH sidebar",
    "Rendering and themes",
    "Configuration and diagnostics",
]

SUBJECT_RE = re.compile(r"^(?P<type>[a-z]+)(?:\((?P<scope>[a-z0-9_-]+)\))?: (?P<title>.+)$")
INLINE_PR_RE = re.compile(r"\(#(?P<pr>\d+)\)$")
MERGE_PR_RE = re.compile(r"^Merge pull request #(?P<pr>\d+) from ")


def git(*args: str) -> str:
    completed = subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    )
    return completed.stdout


def parse_subject(subject: str) -> tuple[str | None, str | None, int | None]:
    """Split a commit subject into (type, scope, inline_pr).

    Non-conforming subjects (the repository's early discovery phase) return
    (None, None, None) and are counted, not listed.
    """
    inline_pr = None
    inline = INLINE_PR_RE.search(subject)
    if inline:
        inline_pr = int(inline.group("pr"))
    match = SUBJECT_RE.match(subject)
    if not match:
        return None, None, inline_pr
    return match.group("type"), match.group("scope"), inline_pr


def load_commits(head: str) -> list[dict]:
    """Every non-merge commit reachable from head, newest first."""
    raw = git("log", "--no-merges", f"--format=%H%x00%s", head)
    commits = []
    for line in raw.splitlines():
        if not line:
            continue
        sha, subject = line.split("\x00", 1)
        commits.append({"sha": sha, "subject": subject})
    return commits


def build_pr_map(head: str) -> dict[str, int]:
    """Map commit SHA -> pull-request number for PR-merged commits.

    Walks the first-parent spine of head. A spine commit whose subject is
    `Merge pull request #N from ...` credits N for every commit reachable
    from its second parent but not its first (the commits the PR landed).
    Commits brought in before PR workflow started, or landed outside a PR
    merge, are simply absent from the map.
    """
    pr_map: dict[str, int] = {}
    spine = git(
        "log", "--first-parent", "--merges", f"--format=%H%x00%P%x00%s", head
    )
    for line in spine.splitlines():
        if not line:
            continue
        sha, parents, subject = line.split("\x00", 2)
        merge = MERGE_PR_RE.match(subject)
        if not merge:
            continue
        parent_shas = parents.split(" ")
        if len(parent_shas) < 2:
            continue
        brought = git("rev-list", f"{parent_shas[1]}", f"--not", f"{parent_shas[0]}")
        for brought_sha in brought.split():
            pr_map.setdefault(brought_sha, int(merge.group("pr")))
    return pr_map


def classify(commits: list[dict], pr_map: dict[str, int]) -> dict:
    """Bucket commits into the note's sections. Raises on an unmapped scope."""
    result = {
        "listed": [],  # dicts: subject, sha, short, pr, type, area
        "counted": {},  # label -> count
        "pr_numbers": set(),
    }
    for commit in commits:
        subject = commit["subject"]
        commit_type, scope, inline_pr = parse_subject(subject)
        pr = inline_pr or pr_map.get(commit["sha"])
        if pr:
            result["pr_numbers"].add(pr)
        if commit_type is None:
            result["counted"]["early project history (pre-convention subjects)"] = (
                result["counted"].get(
                    "early project history (pre-convention subjects)", 0
                )
                + 1
            )
            continue
        commit_type = TYPE_ALIASES.get(commit_type, commit_type)
        if commit_type in USER_VISIBLE_TYPES and scope not in INTERNAL_SCOPES:
            if scope is not None and scope not in AREA_BY_SCOPE:
                raise SystemExit(
                    f"generate_notes.py: unmapped scope {scope!r} in subject {subject!r}; "
                    "add it to AREA_BY_SCOPE so the grouping stays complete"
                )
            result["listed"].append(
                {
                    "subject": subject,
                    "sha": commit["sha"],
                    "short": commit["sha"][:7],
                    "pr": pr,
                    "type": commit_type,
                    "area": AREA_BY_SCOPE.get(scope, "Unscoped"),
                }
            )
        else:
            label = f"{commit_type}({scope})" if scope else commit_type
            result["counted"][label] = result["counted"].get(label, 0) + 1
    return result


def safe_subject(subject: str) -> str:
    """Keep verbatim subjects from being parsed as markdown links by
    check_docs.py (no commit subject today contains the pattern; this guard
    keeps that true forever). A space after the bracket breaks the link
    grammar without hiding the text."""
    if "](" in subject:
        return subject.replace("](", "] (")
    return subject


def render(
    version: str,
    head: str,
    commits: list[dict],
    pr_map: dict[str, int],
    head_subject: str,
    head_date: str,
    root_sha: str,
) -> str:
    classified = classify(commits, pr_map)
    total = len(commits)
    lines: list[str] = []
    lines.append(f"# Noren {version} — release notes")
    lines.append("")
    lines.append("> **Read this first.** This is an explicitly dated developer")
    lines.append("> preview, not a finished terminal. Decision"
                 " [D-M8-001](../../coordination/decisions/D-M8-001-preview-scope.md)")
    lines.append("> scoped it honestly: a bitmap font with bounded coverage (no CJK")
    lines.append("> or emoji glyphs), no IME input, no accessibility surface, macOS")
    lines.append("> (Apple Silicon) only, a workspace sidebar that is a first vertical")
    lines.append("> slice rather than the full product, and fixed built-in themes. What")
    lines.append("> does not work is enumerated clause by clause in"
                 " [known limitations](../../known-limitations.md),")
    lines.append("> and that page outranks anything below: features listed here are the")
    lines.append("> merged history, stated with their limits, not a product claim.")
    lines.append("")
    lines.append("Generated from the merged git history; do not edit by hand.")
    lines.append(f"Regenerate with: `python3 scripts/release/generate_notes.py --head {head}`")
    lines.append("")
    lines.append(f"- Head: `{head}` — {head_subject} ({head_date})")
    lines.append(
        f"- History covered: {root_sha[:7]}..{head[:7]} — {total} non-merge commits"
    )
    lines.append(
        f"- Landed through {len(classified['pr_numbers'])} distinct pull requests"
    )
    lines.append("")

    def area_sections(commit_type: str, heading: str) -> None:
        entries = [item for item in classified["listed"] if item["type"] == commit_type]
        if not entries:
            return
        lines.append(f"## {heading}")
        lines.append("")
        by_area: dict[str, list[dict]] = {}
        for item in entries:
            by_area.setdefault(item["area"], []).append(item)
        for area in AREA_ORDER + [a for a in by_area if a not in AREA_ORDER]:
            if area not in by_area:
                continue
            lines.append(f"### {area}")
            lines.append("")
            for item in by_area[area]:
                ref = f"({item['short']}"
                if item["pr"]:
                    ref += f", #{item['pr']}"
                ref += ")"
                lines.append(f"- {safe_subject(item['subject'])} {ref}")
            lines.append("")

    area_sections("feat", "New in this preview")
    area_sections("fix", "Fixed")
    area_sections("perf", "Performance")

    if classified["counted"]:
        lines.append("## Also merged (counted, not user-visible)")
        lines.append("")
        for label, count in sorted(classified["counted"].items(), key=lambda kv: -kv[1]):
            lines.append(f"- {label}: {count} commits")
        lines.append("")

    lines.append("The bug tracker remains the authority on open defects; in")
    lines.append("particular the issues named in"
                 " [known limitations](../../known-limitations.md) describe")
    lines.append("behaviour that this preview ships with on purpose.")
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--head", default="HEAD", help="commit to document")
    parser.add_argument("--version", default="0.1.0-preview")
    parser.add_argument("--output", type=Path, default=None)
    args = parser.parse_args()

    head = git("rev-parse", args.head).strip()
    head_subject = git("log", "-1", "--format=%s", head).strip()
    head_date = git("log", "-1", "--format=%cI", head).strip()[:10]
    root_sha = git("rev-list", "--max-parents=0", head).splitlines()[-1]
    commits = load_commits(head)
    pr_map = build_pr_map(head)
    text = render(args.version, head, commits, pr_map, head_subject, head_date, root_sha)

    output = args.output
    if output is None:
        output = ROOT / "docs" / "release" / "notes" / f"{args.version}.md"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(text, encoding="utf-8")
    listed = sum(1 for _ in classify(commits, pr_map)["listed"])
    print(f"wrote {output.relative_to(ROOT)} ({listed} listed commits, {len(commits)} total)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
