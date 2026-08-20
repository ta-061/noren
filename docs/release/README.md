# Release candidate process (Milestone 8 machinery)

This page documents how a Noren release candidate is built, checksummed, and
described — and, just as deliberately, what that process **never** does.
Decision [D-M8-001](../coordination/decisions/D-M8-001-preview-scope.md)
settled that Milestone 8 stops at a release candidate: signing, notarization,
tagging, and publication are owner decisions, taken by a human, never by a
script.

## What is NOT covered — read this first

**The artifact this process produces is unsigned, not notarized, and passes
through no Gatekeeper handling.** Concretely:

- **No code signing.** No Developer ID Application certificate is used,
  requested, or assumed. The binary carries no signature.
- **No notarization.** Apple's notarization service is never contacted.
- **Gatekeeper will react.** An unsigned macOS binary that reaches another
  machine — by browser download, AirDrop, email, or anything other than
  building it locally — acquires a quarantine attribute, and macOS will show
  a security dialog ("cannot be opened because the developer cannot be
  verified") on first launch. Depending on the macOS version and local
  policy it may be openable via System Settings, or **refused entirely**.
  This is expected behavior for an unsigned binary, not a broken download.
  First-launch expectations, including this warning, are spelled out in the
  [install verification checklist](install-verification.md).
- **A checksum is not a signature.** SHA-256 in `SHA256SUMS` proves the bytes
  are the bytes the builder staged; it says nothing about who built them.
  Only signing and notarization address that, and they are out of scope here.
- **No publication.** Nothing in this process creates a git tag, a GitHub
  release, or uploads any artifact anywhere. Those actions do not exist in
  the scripts; they are performed by the owner or not at all. If a step
  cannot be done without a certificate or credential, the gap is documented
  and the process stops — that is the intended stopping point.

## Building the release candidate

One command, from the repository root, on macOS:

```
python3 scripts/release/build.py --smoke
```

It runs, in order (the exact commands are echoed live and recorded in
`dist/BUILD-PROVENANCE.txt`):

| Step | Exact command |
| --- | --- |
| Toolchain record | `rustc -vV`, `cargo --version --verbose`, `rustup target list --installed` |
| Host record | `uname -a`, `sw_vers --productVersion`, `git rev-parse HEAD`, `git status --porcelain` |
| Version record | `cargo metadata --no-deps --format-version 1` |
| Build | `cargo build --release --locked -p noren-app` |
| Stage | copy `target/release/noren-app` to `dist/noren-<version>-<host-triple>` |
| Notes | `python3 scripts/release/notes.py --output dist/release-notes.md` |
| Checksums | SHA-256 of every staged file, written by the script to `dist/SHA256SUMS` |
| Launch check | `--smoke`: run the staged binary with `NOREN_CONFIG` naming a missing file; it must exit 1 with `Noren configuration is unusable` on stderr (the documented `AppConfig::load` failure path in `crates/noren-app/src/config.rs`) |

`--dry-run` prints the commands and artifact paths without building.
`--smoke-gui` additionally launches the real windowed binary against a
scratch `HOME`, verifies it stays alive and owns a direct `zsh` child, then
terminates it; it needs a logged-in GUI session, so CI never runs it.

The build refuses a dirty working tree (a release candidate must correspond
to a commit); `--allow-dirty` overrides and records the fact in provenance.

### Artifacts

| Path | Contents |
| --- | --- |
| `dist/noren-<version>-<host-triple>` | the executable (e.g. `noren-0.1.0-aarch64-apple-darwin`) |
| `dist/SHA256SUMS` | `sha256sum`-format checksum of every artifact except itself |
| `dist/BUILD-PROVENANCE.txt` | NFR-008 toolchain and host record |
| `dist/release-notes.md` | generated changelog template |

`dist/` is gitignored; artifacts are never committed.

### Reproducibility, honestly

The script records the toolchain (`rustc`/`cargo` versions, installed
targets), the host (macOS version, architecture), the exact build command
with `--locked`, the `Cargo.lock` SHA-256, and the commit. **It does not
claim bit-for-bit reproducibility** of the binary; no such claim has been
verified. The provenance file says the same thing.

## Release notes from real history

```
python3 scripts/release/notes.py            # to stdout
python3 scripts/release/notes.py --output dist/release-notes.md
```

The changelog body is `git log` subjects since the baseline, verbatim,
grouped by conventional-commit type; merge commits are elided and counted,
never silently dropped (unknown subject shapes land in an explicit
"Unclassified" section). The baseline is resolved as: `--since` ref if
given, else the newest git tag, else the documented fallback `1d329a5` —
the Milestone 2 close named in [ROADMAP.md](../../ROADMAP.md); the
repository has no release tags. Because the list is generated from history,
it cannot drift from it; human decisions live only in the unchecked
"Owner to complete" boxes (including the signing gap).

## Verifying an install

[install-verification.md](install-verification.md) is the executable
checklist: fresh-machine assumptions, checksum verification, first-launch
expectations (including the Gatekeeper warning), and clean uninstall.

## Test coverage and mutation evidence

`scripts/test_release_tools.py` pins the tooling's behaviour; CI runs it
alongside the documentation checker tests. Each pinned behaviour was
mutation-tested — the mutation applied, the named test observed to fail, the
original restored:

| Behaviour | Mutation applied | Test that failed |
| --- | --- | --- |
| Merge commits are elided from notes | collector stops filtering `Merge …` subjects | `test_collected_history_has_no_merge_subjects` |
| Fixed group order (Features before Fixes) | swap `feat`/`fix` in `GROUP_ORDER` | `test_fixed_group_order_features_before_fixes` |
| Unknown subjects kept, verbatim, in "other" | file unknown subject as `chore` | `test_unknown_subject_lands_in_other_verbatim` |
| Owner checklist includes the signing gap | drop the `SIGNING GAP` checklist block | `test_render_contains_owner_checklist_and_signing_gap` |
| `--since` override wins over tag/fallback | remove the override branch | `test_override_wins_over_tag_and_fallback` |
| Every subject renders once | drop the entry-rendering line | `test_render_lists_every_subject_and_counts` |
| Build uses `--locked` | remove `--locked` from the cargo command | `test_release_build_is_locked_and_package_scoped` |
| Checksum is SHA-256 | swap in `hashlib.sha1` | `test_sha256_file_against_known_digest` |
| Manifest uses `sha256sum` two-space format | single-space separator | `test_manifest_entry_uses_sha256sum_two_space_format` |
| Manifest covers every artifact except itself | manifest only the binary | `test_write_manifest_covers_every_artifact_except_itself` |
| Version read from `noren-app`, not another package | return the first package's version | `test_version_comes_from_the_noren_app_package` |
| Build refuses non-macOS platforms | guard condition replaced with `False` | `test_non_macos_platform_is_refused` |
| Staged artifact is executable | drop the `chmod` | `test_stage_binary_copies_content_and_sets_exec_bit` |

No Rust behaviour was added by this machinery, so there is nothing new that
needs reaching from the `noren-app` binary; the scripts are themselves the
reachable surface — invoked above, exercised by their tests, and run by CI.
