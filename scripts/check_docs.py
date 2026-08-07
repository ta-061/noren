#!/usr/bin/env python3
"""Dependency-free checks for Noren's documentation baseline."""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote


ROOT = Path(__file__).resolve().parents[1]
REQUIRED = (
    "README.md",
    "ARCHITECTURE.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "GOVERNANCE.md",
    "ROADMAP.md",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "docs/project-principles.md",
    "docs/coordination/status.md",
    "docs/coordination/decisions.md",
    "docs/coordination/open-questions.md",
)
TEXT_SUFFIXES = {".md", ".txt", ".yml", ".yaml", ".toml", ".py"}
LINK_RE = re.compile(
    r"!?\[[^\]]*\]\((?P<target><[^>]+>|[^)\s]+)"
    r"(?:\s+(?:\"[^\"]*\"|'[^']*'))?\)"
)
SECRET_PATTERNS = (
    re.compile(r"gh[opusr]_[A-Za-z0-9]{20,}"),
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-[A-Za-z0-9_-]{20,}"),
    re.compile(r"(?:AKIA|ASIA)[0-9A-Z]{16}"),
    re.compile(r"AIza[0-9A-Za-z_-]{30,}"),
    re.compile(r"glpat-[A-Za-z0-9_-]{20,}"),
    re.compile(r"npm_[A-Za-z0-9]{20,}"),
    re.compile(r"xox[baprs]-[A-Za-z0-9-]{10,}"),
    re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----"),
)


def repository_files() -> list[Path]:
    completed = subprocess.run(
        [
            "git",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    return [
        ROOT / entry.decode("utf-8", errors="surrogateescape")
        for entry in completed.stdout.split(b"\0")
        if entry
    ]


def contains_secret(text: str) -> bool:
    return any(pattern.search(text) for pattern in SECRET_PATTERNS)


def check_local_links(path: Path, text: str, failures: list[str]) -> None:
    if path.suffix != ".md":
        return
    for match in LINK_RE.finditer(text):
        target = match.group("target").strip("<>")
        if target.startswith(("#", "http://", "https://", "mailto:")):
            continue
        relative = unquote(target.split("#", 1)[0].split("?", 1)[0])
        if not relative:
            continue
        destination = (
            ROOT / relative.lstrip("/")
            if relative.startswith("/")
            else path.parent / relative
        )
        resolved = destination.resolve()
        try:
            resolved.relative_to(ROOT)
        except ValueError:
            line = text.count("\n", 0, match.start()) + 1
            failures.append(
                f"{path.relative_to(ROOT)}:{line}: local link escapes repository {target!r}"
            )
            continue
        if not resolved.exists():
            line = text.count("\n", 0, match.start()) + 1
            failures.append(
                f"{path.relative_to(ROOT)}:{line}: missing local link target {target!r}"
            )


def main() -> int:
    failures: list[str] = []

    for relative in REQUIRED:
        path = ROOT / relative
        if not path.is_file() or path.stat().st_size == 0:
            failures.append(f"{relative}: required non-empty file is missing")

    for path in repository_files():
        if not path.is_file():
            continue
        relative_path = path.relative_to(ROOT)
        raw_cli_evidence = relative_path.parts[:3] == (
            "docs",
            "coordination",
        )
        try:
            text = path.read_text(encoding="utf-8")
        except UnicodeDecodeError:
            if path.suffix in TEXT_SUFFIXES:
                failures.append(f"{relative_path}: not valid UTF-8")
            continue

        if contains_secret(text):
            failures.append(
                f"{relative_path}: possible credential/private key material"
            )
        if path.suffix not in TEXT_SUFFIXES:
            continue
        if text and not text.endswith("\n"):
            failures.append(f"{relative_path}: missing final newline")
        if not raw_cli_evidence:
            for number, line in enumerate(text.splitlines(), start=1):
                if line.endswith((" ", "\t")):
                    failures.append(
                        f"{relative_path}:{number}: trailing whitespace"
                    )
        check_local_links(path, text, failures)

    if failures:
        for failure in failures:
            print(f"ERROR: {failure}", file=sys.stderr)
        return 1

    print("Documentation structure, local links, whitespace, UTF-8, and secret patterns: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
