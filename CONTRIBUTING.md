# Contributing to Noren

Thank you for helping build Noren. The project is in Discovery: documentation,
research, test design, fixtures, and narrowly approved experiments are welcome,
but production terminal implementation is gated on Milestone 1.

## Before starting

1. Read the [project principles](docs/project-principles.md), current
   [status](docs/coordination/status.md), and relevant requirement/RFC/ADR.
2. Find or open an Issue with background, objective, scope, forbidden scope,
   dependencies, acceptance criteria, security considerations, and required
   tests.
3. Agree on file ownership. Concurrent contributors must not edit the same file.
4. Create a branch or worktree from `main`; never commit directly to `main`.

## Change workflow

Use focused branches such as `docs/requirements`, `experiment/pty-comparison`,
`feature/split-layout`, or `security/osc52-policy`. Keep unrelated cleanup out
of the change.

Before opening a PR:

- run the tests named by the Issue;
- run `python3 scripts/check_docs.py` for documentation changes;
- run `git diff --check`;
- record behavior, limitations, security impact, and rollback steps;
- ensure no credentials, cookies, SSH keys, passphrases, or private data appear
  in the diff or logs.

Rust-specific commands will become mandatory once the workspace and pinned
toolchain exist. The intended baseline is formatting, Clippy with warnings
denied, all workspace tests/features, dependency audit, and license policy
checks; the repository will not claim these pass before CI actually runs them.

## Pull requests and review

Open a Draft PR early. Complete the PR template and link the Issue. The
implementer may respond to review but does not provide final approval. Reviewers
classify findings as BLOCKER, MAJOR, or MINOR and cite the file, impact,
reproduction/evidence, and proposed correction.

AI-assisted contributions follow the same rules. Generated code is not evidence
of correctness, and an agent summary does not replace inspection of the diff,
tests, and specification.

## Developer Certificate of Origin

Noren uses the [Developer Certificate of Origin 1.1](https://developercertificate.org/).
Sign each commit with:

```text
Signed-off-by: Your Name <your-email@example.com>
```

Git can add this line with `git commit -s`. By signing off, you certify that you
have the right to submit the contribution under the project license.
