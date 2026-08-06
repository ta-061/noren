# Development model

Noren is developed with parallel AI coding lanes coordinated by a human owner,
alongside ordinary human contribution. This file records only the rules that a
contributor or reviewer needs; the tooling that runs the lanes is personal
infrastructure and lives outside this repository.

## Rules that apply to every change

- **Every change lands through a pull request.** No direct pushes to `main`.
- **CI is required.** The Rust build/lint/test job, the documentation validator,
  the dependency audit, and MSRV verification must all pass.
- **Review must cover the current head.** A review of an earlier commit says
  nothing about code pushed after it, and checks for a new head can finish before
  its review arrives. `scripts/ci/merge_gate.py` enforces this and can be run by
  anyone.
- **Zero unresolved review threads** before merge.
- **The implementer is not the verifier.** Whoever wrote a change does not
  approve it, and self-review alone is never sufficient.
- **No completion claim without evidence.** Only evidence-backed work is marked
  complete: quote the command that was run and its real output. A summary is not
  evidence, and green CI is not review.

## Parallel work

Concurrent changes are given non-overlapping file ownership. Two changes that
must edit the same file are sequenced instead of run together, because a clean
automatic merge of two independent edits can still be semantically wrong.

Files that concentrate conflicts — crate-root export modules, the application
entry point, `Cargo.toml`, and `Cargo.lock` — are edited by one change at a time,
with export wiring deferred to a separate integration commit.

A change based on an older `main` must be rebased before merge, and its diff
checked for deletions it did not intend. A branch cut before a sibling's tests
existed will otherwise propose removing them, and `mergeable=CLEAN` plus green CI
does **not** catch that: a clean delete is still a clean merge.

## Review expectations

A review is not a read-through. It should include a reproduction attempt and a
search for counter-examples. Findings are classified BLOCKER, MAJOR, or MINOR and
cite the file, the impact, the reproduction, and a proposed correction.

Generated code is not evidence of correctness, and an agent summary does not
replace inspecting the diff, the tests, and the specification.
