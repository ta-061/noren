# Decision ledger

This ledger records coordination decisions before formal ADRs exist. Architecture
decisions move to numbered ADRs during Milestone 1.

| ID | Date | Decision | Evidence/rationale | Status |
| --- | --- | --- | --- | --- |
| D-0001 | 2026-08-03 | Keep production implementation closed until the Milestone 1 gate passes. | The project requires requirements, architecture, threat model, and independent review before implementation. | Active |
| D-0002 | 2026-08-03 | Begin as one repository; do not create remote, conformance, site, or extension repositories yet. | None currently has an independently proven release/security boundary. | Active; revisit by ADR |
| D-0003 | 2026-08-03 | Treat all feature and library choices as candidates during Discovery. | The repository has no PoCs, measurements, or verified library comparison. | Active |
| D-0004 | 2026-08-03 | Use only model IDs and CLI workflows demonstrated by local help/config/model-list commands. | Prevents guessed commands and unavailable-model assignments. | Active |
| D-0005 | 2026-08-03 | Keep generated calibration responses verbatim and score them separately. | Preserves provenance and makes role assignment auditable. | Active |
| D-0006 | 2026-08-03 | Use DCO 1.1 sign-off rather than a CLA for initial contributions. | It is lightweight, auditable per commit, and compatible with the intended dual license; governance can revisit by RFC. | Active |
| D-0007 | 2026-08-03 | Enable GitHub private vulnerability reporting and retain public Issues for non-sensitive work. | Provides a private security channel before a support email exists. | Active |
