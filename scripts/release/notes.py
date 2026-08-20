#!/usr/bin/env python3
"""Generate the Noren release-notes template from the actual merged history.

The changelog body is `git log` output, verbatim: every non-merge commit
subject since the baseline, grouped by conventional-commit type. Nothing in
the generated list is hand-written, so the notes cannot drift from the
history; anything a human must still decide is an explicit unchecked box in
the owner section, never invented prose.

The baseline is resolved as: an explicit `--since` ref if given, else the
most recent git tag (none exists today), else the documented fallback — the
Milestone 2 close `1d329a5` named in ROADMAP.md, the last milestone that
reached a recorded completion state.

Usage:
    python3 scripts/release/notes.py [--since <ref>] [--output <path>]

Without `--output` the notes are written to stdout.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FALLBACK_BASELINE = "1d329a5"
FALLBACK_BASELINE_WHY = (
    "documented Milestone 2 close (ROADMAP.md); the repository has no release tags"
)
RECORD_SEP = "\x1e"
# A unit-separator control character, not %x00: an embedded NUL cannot be
# passed inside an argv string, and the separator must be safe both in argv
# and inside commit subjects' rendering (subjects cannot contain it).
FIELD_SEP = "\x1f"
MERGE_SUBJECT_RE = re.compile(
    r"^Merge (PR #\d+: |pull request #\d+ |remote-tracking branch |branch ')"
)
CONVENTIONAL_RE = re.compile(
    r"^(?P<type>feat|fix|perf|refactor|test|docs|chore|ci|style|build)"
    r"(?:\((?P<scope>[A-Za-z0-9_-]+)\))?(?P<breaking>!)?: (?P<description>.+)$"
)
GROUP_ORDER = (
    ("feat", "Features"),
    ("fix", "Fixes"),
    ("perf", "Performance"),
    ("refactor", "Refactors"),
    ("test", "Tests"),
    ("docs", "Documentation"),
    ("chore", "Chores"),
    ("ci", "CI"),
    ("style", "Style"),
    ("build", "Build"),
)
OTHER_TITLE = "Unclassified subjects (listed verbatim)"


def is_merge_subject(subject: str) -> bool:
    return bool(MERGE_SUBJECT_RE.match(subject))


def classify_subject(subject: str) -> tuple[str, str]:
    """Map one commit subject to `(group_key, rendered entry)`.

    Conventional subjects render as `scope: description` (scope omitted when
    absent). Anything else lands in the `other` group, verbatim — a subject
    the classifier does not understand must be surfaced, never dropped or
    silently re-worded.
    """
    match = CONVENTIONAL_RE.match(subject)
    if match is None:
        return "other", subject
    scope = match.group("scope")
    description = match.group("description")
    bang = "!" if match.group("breaking") else ""
    if scope:
        return match.group("type"), f"{scope}{bang}: {description}"
    return match.group("type"), f"{bang}{description}" if bang else description


def resolve_baseline(override: str | None, tags: list[str]) -> tuple[str, str]:
    """Pick the changelog baseline: override wins, then the newest tag, then
    the documented fallback. Returns `(ref, human-readable source)`."""
    if override is not None:
        return override, f"--since {override}"
    if tags:
        return tags[0], f"git tag {tags[0]}"
    return FALLBACK_BASELINE, FALLBACK_BASELINE_WHY


def list_tags(repo: Path) -> list[str]:
    completed = subprocess.run(
        ["git", "for-each-ref", "--sort=-creatordate", "--format=%(refname:short)",
         "refs/tags"],
        cwd=repo, check=True, capture_output=True, text=True,
    )
    return completed.stdout.split()


def collect_subjects(repo: Path, baseline: str) -> tuple[list[str], str, str, int]:
    """Return `(subjects, head_sha, baseline_sha, merge_commit_count)` for
    every commit in `baseline..HEAD`."""
    completed = subprocess.run(
        ["git", "rev-parse", baseline],
        cwd=repo, check=True, capture_output=True, text=True,
    )
    baseline_sha = completed.stdout.strip()
    completed = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        cwd=repo, check=True, capture_output=True, text=True,
    )
    head_sha = completed.stdout.strip()
    completed = subprocess.run(
        ["git", "log", f"{baseline_sha}..HEAD", f"--format=%H{FIELD_SEP}%s{RECORD_SEP}"],
        cwd=repo, check=True, capture_output=True, text=True,
    )
    subjects: list[str] = []
    merges = 0
    for record in completed.stdout.split(RECORD_SEP):
        if not record.strip():
            continue
        _hash, _, subject = record.strip().partition(FIELD_SEP)
        if is_merge_subject(subject):
            merges += 1
        else:
            subjects.append(subject)
    return subjects, head_sha, baseline_sha, merges


def group_subjects(subjects: list[str]) -> list[tuple[str, str, list[str]]]:
    """Group classified subjects into `(group_key, title, entries)` in the
    fixed GROUP_ORDER, `other` last."""
    buckets: dict[str, list[str]] = {}
    for subject in subjects:
        group, rendered = classify_subject(subject)
        buckets.setdefault(group, []).append(rendered)
    sections = [
        (key, title, buckets.pop(key))
        for key, title in GROUP_ORDER
        if buckets.get(key)
    ]
    if buckets.get("other"):
        sections.append(("other", OTHER_TITLE, buckets["other"]))
    return sections


def render(head_sha: str, baseline_sha: str, baseline_source: str,
           sections: list[tuple[str, str, list[str]]], merge_count: int,
           total_commits: int, generated_utc: str) -> str:
    listed = sum(len(entries) for _key, _title, entries in sections)
    lines = [
        "# Noren release notes — GENERATED TEMPLATE",
        "",
        f"Generated {generated_utc} from merged history; the lists below are",
        "`git log` subjects, verbatim. Do not hand-edit them — regenerate with:",
        "",
        "    python3 scripts/release/notes.py",
        "",
        f"- Baseline: `{baseline_sha}` (chosen by: {baseline_source})",
        f"- Head: `{head_sha}`",
        f"- Commits since baseline: {total_commits}"
        f" ({listed} listed, {merge_count} merge commits elided)",
        "",
    ]
    for _key, title, entries in sections:
        lines.append(f"## {title}")
        lines.append("")
        lines.extend(f"- {entry}" for entry in entries)
        lines.append("")
    lines.extend([
        "## Owner to complete before any publication",
        "",
        "Unchecked boxes are decisions and verifications only the owner can",
        "make. Nothing below is done by the build script.",
        "",
        "- [ ] Run `python3 scripts/release/build.py --smoke` and keep the",
        "      printed artifact path, size, checksum, and launch results with",
        "      the release records.",
        "- [ ] Execute docs/release/install-verification.md on a machine that",
        "      did not build the artifact, and record the outcome.",
        "- [ ] Decide the artifact label and date. D-M8-001 settled that the",
        "      first artifact is an explicitly dated developer preview, NOT a",
        "      `0.1.0-preview` of the product.",
        "- [ ] Link docs/known-limitations.md unchanged; do not soften it.",
        "- [ ] SIGNING GAP (mandatory reading, docs/release/README.md): the",
        "      artifact is unsigned and not notarized. macOS Gatekeeper will",
        "      warn on first launch and may refuse to open it at all. The",
        "      checksum does not substitute for signing. Deciding whether and",
        "      how to sign and notarize — with which certificate — is an owner",
        "      decision that must be recorded before publication.",
        "- [ ] Creating any git tag, GitHub release, or upload remains an",
        "      owner action; this template and the build script never do it.",
        "",
    ])
    return "\n".join(lines)


def build_notes(repo: Path, override: str | None) -> str:
    baseline, baseline_source = resolve_baseline(override, list_tags(repo))
    subjects, head_sha, baseline_sha, merges = collect_subjects(repo, baseline)
    sections = group_subjects(subjects)
    generated = datetime.now(timezone.utc).isoformat(timespec="seconds")
    return render(head_sha, baseline_sha, baseline_source, sections, merges,
                  len(subjects) + merges, generated)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--since", default=None, metavar="REF",
                        help="baseline ref for the changelog")
    parser.add_argument("--output", default=None, metavar="PATH",
                        help="write notes here instead of stdout")
    args = parser.parse_args(argv)

    notes = build_notes(ROOT, args.since)
    if args.output is None:
        sys.stdout.write(notes)
        return 0
    Path(args.output).write_text(notes, encoding="utf-8")
    print(f"notes written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
