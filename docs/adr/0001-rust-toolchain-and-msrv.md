# ADR 0001: Pin Rust 1.88.0 for the first PoC

- Status: Accepted
- Date: 2026-08-03
- Decision owners: Codex integration for Issue #6; human owner approves by merge
- Related Issue/RFC: [#6](https://github.com/ta-061/noren/issues/6)

## Context

Noren has no installed Rust toolchain or Cargo manifest. R-PORT-01 requires an
approved, reproducible toolchain/MSRV and recorded targets before compilation
can supply evidence. The first implementation target is the owner's Apple
Silicon Mac; production portability remains unapproved.

A narrow crates.io metadata check on 2026-08-03 reports declared MSRVs of 1.87
for `wgpu` 30.0.0, 1.82 for `avt` 0.18.0, 1.70 for `winit` 0.30.13, and 1.66 for
`unicode-width` 0.2.2. `portable-pty` 0.9.0 and `swash` 0.2.10 do not declare
`rust_version`; only a locked compile can validate them.

## Decision drivers

- Exact, reproducible local/CI behavior rather than an unbounded `stable` pin.
- Rust 2024 edition and workspace resolver 3.
- Coverage of the highest declared MSRV in the scoped candidate set.
- A modest rollback path if an undeclared dependency MSRV or compiler issue is
  found.

## Options considered

1. Floating `stable`: current, but not reproducible and fails R-PORT-01.
2. Rust 1.88.0: one stable release above the highest declared candidate MSRV.
3. The oldest apparent candidate MSRV: smaller promise, but forces avoidable
   cross-version testing before the first compile.

## Decision

The first implementation creates `rust-toolchain.toml` with:

```toml
[toolchain]
channel = "1.88.0"
profile = "minimal"
components = ["clippy", "rustfmt"]
targets = ["aarch64-apple-darwin"]
```

Workspace packages use Rust edition 2024, resolver 3, and `rust-version =
"1.88"`. Rust 1.88.0 is both the build pin and preview MSRV until a later ADR
changes it. The implementation Issue may install this toolchain now; no other
toolchain or target is silently substituted.

The first local and CI compile checkpoint records verbatim:

- `rustc --version --verbose` and `cargo --version --verbose`;
- `rustup toolchain list` and `rustup target list --installed`;
- `uname -m`, macOS version, CI image label, and exact lockfile;
- success/failure of `cargo check --workspace --all-targets` and the required
  test/lint commands.

If CI's host differs from arm64, its host target is recorded in addition to the
required installed `aarch64-apple-darwin` target. A successful compile is still
required; this ADR alone does not pass the executable part of R-PORT-01.

## Consequences

Implementers can create `Cargo.toml` and CI without choosing a moving compiler.
Upgrading Rust requires a reviewed ADR change plus old/new CI evidence. Packages
whose MSRV exceeds 1.88 are unavailable unless replaced or the ADR is revised.

## Security and reliability impact

Pinning reduces toolchain drift and makes compiler behavior auditable. It does
not audit the compiler, registry, dependency graph, or native SDK. Toolchain
installation uses official rustup distribution metadata; no credential is
stored in repository configuration.

## Validation evidence

Design evidence is the merged library comparison plus the narrow versioned
metadata check above. Executable evidence is intentionally pending the first
implementation PR because Rust is absent at decision time.

## Reversal or replacement plan

Change the pinned channel and workspace `rust-version` together in one Issue;
record candidate dependency MSRVs, lockfile diff, and full old/new CI results.
Rollback restores the prior toolchain file and lockfile.

## Dissent and unresolved questions

GLM proposed 1.88.0. Qwen did not contest the toolchain. The undeclared MSRVs of
`portable-pty` and `swash`, CI host architecture, signing/notarization identity,
and future Linux targets remain executable or human-decision gates.
