# Contributing to Noren

Thank you for helping build Noren. Discovery and the first macOS local-zsh PTY
PoC are merged, and terminal foundation work is in progress; contributions
follow the [roadmap](ROADMAP.md) through Issue-backed, narrowly scoped Draft
PRs.

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

The Rust workspace and pinned toolchain exist, so Rust changes must pass:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Dependency audit and license policy checks remain intended additions; the
repository will not claim these pass before CI actually runs them.

Changes to `crates/noren-terminal/` additionally keep these conventions:

- Internal coordinates are zero-based; CSI parameters are one-based. Convert
  at the parse boundary and keep the two schemes out of each other's layers.
- Test through the public API with byte-driven fixtures: feed raw bytes into
  `TerminalState::feed_bytes` and assert on `snapshot()`, not on internals.
- Scrolling-region changes must preserve rows outside the active margins.
- Renderer and PTY concerns stay out of `noren-terminal` tests; the core
  neither reads from a PTY nor renders.

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
