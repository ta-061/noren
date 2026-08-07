# Decision ledger

This ledger records coordination decisions before formal ADRs exist. Architecture
decisions move to numbered ADRs during Milestone 1.

| ID | Date | Decision | Evidence/rationale | Status |
| --- | --- | --- | --- | --- |
| D-0001 | 2026-08-03 | Keep production implementation closed until the Milestone 1 gate passes. | The project requires requirements, architecture, threat model, and independent review before implementation. | Satisfied only for the scoped local-PTY PoC by ADR 0001/0002 when Issue #6 merges; active for every deferred feature |
| D-0002 | 2026-08-03 | Begin as one repository; do not create remote, conformance, site, or extension repositories yet. | None currently has an independently proven release/security boundary. | Active; revisit by ADR |
| D-0003 | 2026-08-03 | Treat all feature and library choices as candidates during Discovery. | The repository has no PoCs, measurements, or verified library comparison. | Active |
| D-0004 | 2026-08-03 | Use only model IDs and CLI workflows demonstrated by local help/config/model-list commands. | Prevents guessed commands and unavailable-model assignments. | Operational rule; evidence is kept outside the public product repository |
| D-0005 | 2026-08-03 | Keep generated calibration responses verbatim and score them separately. | Preserves provenance and makes role assignment auditable. | Operational rule; artifacts are kept outside the public product repository |
| D-0006 | 2026-08-03 | Use DCO 1.1 sign-off rather than a CLA for initial contributions. | It is lightweight, auditable per commit, and compatible with the intended dual license; governance can revisit by RFC. | Active |
| D-0007 | 2026-08-03 | Enable GitHub private vulnerability reporting and retain public Issues for non-sensitive work. | Provides a private security channel before a support email exists. | Active |
| D-0008 | 2026-08-03 | Open the implementation gate only for FR-001 through FR-007: a single-window macOS local-zsh PTY PoC. | GLM and Qwen supplied non-overlapping proposals; Codex integrated requirements, architecture, threats, tests, and ADRs; Claude/codex-lab corrections are clean at `2bdd7bea`. | Approved; becomes Active when PR #15 merges |
| D-0009 | 2026-08-03 | Pin Rust/MSRV 1.88.0, edition 2024, resolver 3, and installed `aarch64-apple-darwin` target for the first PoC. | Covers the scoped candidates' declared MSRVs; exact compile/toolchain evidence remains the first implementation gate. | Accepted by ADR 0001; executable evidence pending |
| D-0010 | 2026-08-05 | Develop through parallel AI coding lanes with independent verification, coordinated by the owner. | A single-integrator model concentrated all work in one place and stalled when that one place was unavailable. Lanes are scoped so no two edit the same file, and the implementer never verifies their own change. | Active; see [development model](development-model.md) |
| D-0011 | 2026-08-05 | Land the Terminal Core stack by merging cumulative tip PR #30 first, then #32 and #33 — not the seven bottom-up per-PR merges originally planned. | GLM found, and the coordinator independently reproduced, a BLOCKER present in #21–#29: `ESC ( B` leaked a printable `B` and HT was dropped. The fix lands only on #30, so bottom-up merging would have put four knowingly output-corrupting commits on `main`. codex-lab argued for per-PR auditability (A) and Qwen for green-main (C); the coordinator chose C and records the subsumed PR numbers in the merge commit to retain the trail. | Active |
