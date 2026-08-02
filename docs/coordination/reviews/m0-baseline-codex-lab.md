# Milestone 0 baseline independent QA

- Issue: [#1](https://github.com/ta-061/noren/issues/1)
- Branch reviewed: `docs/discovery-baseline`
- Reviewer: codex-lab, `gpt-5.6-sol`
- Initial command: `codex-lab review --uncommitted`
- Follow-up: same recorded session via `codex-lab exec resume`
- Review mode: read-only

## Initial findings

| Priority | Finding | Resolution |
| --- | --- | --- |
| P1 | Secret checks ran only after a documentation-suffix filter, so credentials in `.env.example`, JSON, shell, Rust, and other text could pass. | Secret detection now runs for every UTF-8-decodable repository file; suffix filtering applies only to documentation-specific rules. A subprocess regression test proves a fake fine-grained PAT in a non-document suffix fails the checker. |
| P1 | Current fine-grained GitHub, OpenAI project, and Anthropic token forms were not matched. | Added current GitHub, OpenAI/Anthropic-style, AWS temporary, Google, GitLab, npm, Slack, and private-key patterns with constructed-token regression tests. |
| P2 | Newline-delimited `git ls-files` could C-quote and silently skip non-ASCII filenames. | Repository enumeration uses `git ls-files -z`; a Japanese untracked filename test proves enumeration. |
| P2 | A Markdown link resolving outside the repository could be accepted when that host path existed. | Resolved local targets must remain beneath the repository root; an escape regression test covers the case. |
| P2 | Global EditorConfig trimming could mutate trailing whitespace in raw CLI help evidence. | Added a scoped `trim_trailing_whitespace = false` override for `docs/coordination/cli-help/*.txt`. |
| P2 | Python bytecode was not ignored and a generated `.pyc` was present. | Added `__pycache__/` and `*.py[cod]` ignores; moved the generated local artifact out of the worktree. |

## Follow-up verdict

> All prior findings resolved

The reviewer independently verified:

- the full documentation checker passes with bytecode generation disabled;
- runtime probes for secrets in non-document files, modern token formats,
  NUL/non-ASCII paths, and repository-escaping links pass;
- checker regression tests pass;
- raw-evidence EditorConfig and bytecode ignore rules apply;
- no generated Python bytecode remains in the worktree; and
- CI runs both the checker and its regression tests.

## Integrator verification

```text
PYTHONDONTWRITEBYTECODE=1 python3 scripts/check_docs.py
Documentation structure, local links, whitespace, UTF-8, and secret patterns: OK

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest scripts/test_check_docs.py
Ran 7 tests
OK

git diff --check
(exit 0)
```

YAML parsing for all Issue forms, Dependabot configuration, and the documentation
workflow also completed successfully with Ruby's standard YAML parser.
