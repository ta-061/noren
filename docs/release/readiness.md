# Milestone 8 release readiness — evidence, not a verdict

Status terms come from the roadmap: only evidence-backed work is marked
complete. This page reports **done / missing / owner-required** against
Milestone 8's own scope line and [D-M8-001](../coordination/decisions/D-M8-001-preview-scope.md)
and **declares nothing**. The gate — the owner's release review — decides.

The release candidate this page describes was built and verified on one
machine (macOS 26.4.1, arm64, rustc 1.88.0) at the commits named below. Every
claim below was either run on that machine or is marked **unverified**.

## The candidate

Built by `scripts/release/build.sh` (landed in this branch):

| Fact | Value |
| --- | --- |
| Artifact | `dist/noren-0.1.0-preview-<sha>-aarch64-apple-darwin.tar.gz` (one file inside: the `noren-app` binary, ~6.9 MB) |
| Checksums | `dist/SHA256SUMS`, generated and self-verified (`shasum -c`) by the script itself |
| Provenance | `dist/BUILD-PROVENANCE.txt` — the NFR-008 record (see below) |
| Reproducibility | Verified **three-way**: the working tree plus two throwaway clean worktrees of the same commit all produced the identical tarball digest (`build.sh --check-repro`) |
| Launch | **Verified**: binary starts, opens a window, owns a live `zsh` child, exits cleanly on close/`SIGTERM` with the child reaped |
| Not done | No git tag, no GitHub Release, no publication, no signing identity, no notarization, no uploads of any kind — the hard stop was honored |

Why builds are reproducible at all: without `--remap-path-prefix`, rustc
embeds absolute source paths in panic-location strings — measured, two clean
checkouts in different directories differed in exactly those bytes plus the
content-derived `LC_UUID` (17 bytes total). The script remaps the workspace
root and cargo home, which closed it. **Unverified and unclaimed:**
cross-machine, cross-SDK, and cross-toolchain reproducibility (the linker and
SDK participate in the build and are not pinned by this repository);
`--check-repro` proves same-machine same-toolchain only.

## Milestone 8 scope line, item by item

The roadmap's scope: *"Honest docs/site, binaries, checksums, release review,
known limitations and `0.1.0-preview`"* — status **Not started** at the time
D-M8-001 was written.

| Scope item | State | Evidence |
| --- | --- | --- |
| Honest docs | Done as a candidate | [known limitations](../known-limitations.md) (re-verified against the tree 2026-08-27 per its header) and the [install checklist](install-verification-checklist.md), whose first section is the unsigned-binary Gatekeeper warning |
| Site | **Missing** | No website exists; README only. Whether M8 requires a site is an owner scoping decision, not an engineering gap this branch can close |
| Binaries | Done as a release candidate | The candidate above; reproducible, checksummed, launch-verified. Not published anywhere |
| Checksums | Done | Script-generated, never by hand; `shasum -c` self-verified in the build |
| Release review | **Missing** — owner gate | This page is input to that review, not the review |
| Known limitations | Done | [known limitations](../known-limitations.md), front-loaded per D-M8-001 precondition 3 |
| `0.1.0-preview` | **Owner-only, deliberately not done** | Tagging and any public artifact are reserved owner decisions under D-M8-001 |

## D-M8-001 blocking preconditions, item by item

| Precondition | State | Evidence |
| --- | --- | --- |
| 1. FR-005 rendered-frame oracle exists — findings not hidden | Done | `cargo test -p noren-app --test frame_oracle` run at this branch's head: **58 passed, 0 failed, 0 ignored**, including both former defect tests (`lowercase_distinct_from_uppercase`, `non_ascii_glyph_is_not_the_question_mark`), which now guard the PR #141 font fixes. What the font still cannot do stays documented in known limitations (bounded coverage, seven-pair collision allowlist) — the gaps are named, not hidden |
| 2. Release-integrity and reproducibility gates | Partly done; the rest is owner-only | Checksums: done (above). NFR-008 provenance: done (below). NFR-009's signing, notarization, and packaging gates: **unmet and unmeetable without owner credentials** — no signing identity, no Apple certificate, no notarization exists in this branch by design |
| 3. Front-loaded honest documentation | Done as a candidate | The limits are where a reader meets them first: known limitations for the product, the checklist's top section for the binary, the notes banner for the release |
| 4. Framing that does not imply what does not exist | Done as a candidate | The notes open with the dated-developer-preview banner and defer to known limitations; the artifact is named `0.1.0-preview-<sha>` with no product claim. Final framing of anything published is part of the owner's release review |

## NFR-008 provenance, requirement by requirement

NFR-008: *CI and local handoff record `rustc --version --verbose`,
`cargo --version --verbose`, installed targets, macOS version, architecture,
lockfile, and exact direct dependencies.*

| Requirement | Where recorded |
| --- | --- |
| rustc version, verbose | `BUILD-PROVENANCE.txt` `[toolchain]` |
| cargo version | `[toolchain]` |
| Installed targets | `[toolchain]` (`rustup target list --installed`) |
| macOS version | `[host]` (`sw_vers`, `uname -a`) |
| Architecture | `[host]` (`uname -m`) |
| Lockfile | Full `Cargo.lock` SHA-256 (`[inputs]`) |
| Exact direct dependencies | `cargo tree --workspace --depth 1 --edges normal --locked` output (`[direct-dependencies]`); the workspace pins every direct dep with `=` |

Also recorded: the commit, its subject and date, the toolchain channel
(`rust-toolchain.toml` pins 1.88.0), the macOS SDK and linker versions, the
exact `RUSTFLAGS` remap used, and the built binary's own SHA-256.

## The two "cheap items" D-M8-001 raised

Both landed on `main` before this branch:

- **Colour is wired to drawing.** SGR foregrounds and backgrounds resolve
  through built-in palettes to per-vertex colour (merge history: PRs #112,
  #121, and the theme-palettes commits, listed in the generated notes), with
  oracle-tested colour assertions and measured WCAG contrast per theme.
- **The M3 modules are wired into the application.** Launching the build
  draws the sidebar, local sessions spawn and switch for real, and state
  persists (ROADMAP Milestone 3 status, plus the launch verification above).

D-M8-001 remains Option C until the owner re-runs the scope decision; the
renderer's remaining disqualifiers (real non-ASCII glyphs — CJK/emoji still
draw replacement boxes, no IME, no accessibility surface) are Milestone 6
scope that has not started.

## What requires the owner

1. **Signing and notarization** — Developer ID certificate, `codesign` with a
   real identity, notarization submission. Requires credentials that do not
   exist on any lane machine. Until then NFR-009's gates are unmet and the
   Gatekeeper warning stays.
2. **Packaging format** — a bare tarball is a candidate answer; a `.dmg` or
   notarized `.app` bundle is the usual macOS answer. Owner decision.
3. **The tag `0.1.0-preview` (or whatever label), the GitHub Release, and
   publication of any artifact** — explicitly out of bounds for every lane.
4. **The release review itself**, including the framing of anything public.
5. **ROADMAP status for Milestone 8** — this branch does not move it.
6. **Coordination:** a stale lane `agent/release-packaging` (last commit
   2026-08-20, no PR ever opened, based on pre-#131 `main`) contains a
   parallel `scripts/release/build.py` + `notes.py` + its own install page.
   This branch's files are disjoint by name; which machinery (if either, or a
   merge) ships is an owner decision.

## Verification log (commands actually run on this machine)

| Command | Result |
| --- | --- |
| `scripts/release/build.sh` | Artifact + `SHA256SUMS` + `BUILD-PROVENANCE.txt` produced; `shasum -c` OK |
| `scripts/release/build.sh --check-repro` | `REPRODUCIBLE`: working tree + two clean worktrees, identical digests |
| Manual determinism experiment (two clean checkouts, no remap) | 17 differing bytes: embedded absolute paths + `LC_UUID` — the documented reason remapping is mandatory |
| `./noren-app` (scratch `HOME`) | Alive with live `zsh` child; `sessions.toml` written under `~/Library/Application Support/Noren/`; clean exit on `SIGTERM`, child reaped |
| `cargo fmt --all -- --check` | Pass (each commit) |
| `cargo clippy --workspace --all-targets -- -D warnings` | Pass (each commit) |
| `cargo test --workspace` | 1099 passed, 0 failed |
| `cargo test -p noren-app --test frame_oracle` | 58 passed, 0 failed, 0 ignored |
| `python3 -m unittest scripts/test_generate_notes.py` | 13 passed |
| `python3 scripts/check_docs.py` | OK |
| `python3 scripts/release/audit_notes_prs.py` (new in this branch) | `OK` — notes PR set 108 = gh-merged-and-reachable 108 at the audited head; only #181 excluded (merged into main after this branch's base) |

**Marked unverified, on purpose:** Gatekeeper's first-launch dialogs were
documented from macOS behavior, not reproduced — a locally built binary
carries no quarantine attribute, so the block cannot fire on the build
machine; someone must exercise the checklist's Gatekeeper section on a second
machine before publication. Likewise macOS versions other than 26.4.1,
machines other than Apple Silicon (unsupported), and cross-machine
reproducibility.
