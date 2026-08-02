# Independent cross-critique task

Status: **Draft — do not execute until all six proposal outcomes are recorded and
the immutable review-pack commit below is filled in.**

## Provenance

- Issue: #6
- Shared-input commit: `TBD`
- Review-pack commit: `TBD`
- Prompt revision: 1
- Execution date: `TBD` (Asia/Tokyo)

Every reviewer receives this file, the same Discovery evidence, and the five
other Round 1 proposals labeled Proposal A through Proposal E. A reviewer never
receives its own proposal. The per-reviewer label mappings are hidden during
review and published after every review is captured. Responses are stored
verbatim; command, tool version, model identifier, outcome, duration, and word
count are recorded outside the response.

## Assignment

Review the proposals as claims and designs, not as model output. Do not guess
authorship or reward writing style. Do not browse, modify files, run project
commands, or inspect another review. Check upstream claims only against the
provided Discovery evidence. Label absent evidence `Unknown`; propose a bounded
experiment when it can resolve a decision.

Produce one decision-oriented review of no more than 3,500 words using exactly
these headings:

1. **Gate verdict** — `Proceed`, `Proceed after listed changes`, or `Do not
   proceed`, with the reasons that determine the verdict.
2. **Proposal comparison** — a compact table of the strongest, weakest, and
   uniquely useful parts of A–E.
3. **Requirement and scope gaps** — missing user behavior, failure semantics,
   non-goals, and Preview cut-line problems.
4. **Feasibility and dependencies** — unsupported APIs, maintenance/license
   risks, platform constraints, and experiments required before selection.
5. **Security and data-loss risks** — trust boundaries, secrets, OSC/config/IPC,
   plugins/adapters, child processes, SSH, persistence, rollback, and recovery.
6. **Input and compatibility risks** — key capture, pass-through liveness,
   Zellij/tmux/full-screen applications, keyboard protocols, and versioned
   fixtures.
7. **Performance and portability risks** — unmeasured claims, resource budgets,
   target environments, macOS/Linux differences, fonts, IME, and accessibility.
8. **Testability and release evidence** — measurable requirements, oracles,
   fault injection, conformance, CI, packaging, rollback, and known limitations.
9. **Repository and API boundaries** — cohesion, dependency direction, stable
   contracts, replaceable libraries, unsafe ownership, and unjustified services
   or repositories.
10. **Smaller alternatives and deferrals** — the least complex design that still
    delivers a credible Preview.
11. **Ranked findings** — up to fifteen findings labeled `Blocker`, `High`,
    `Medium`, or `Low`; each names affected proposals, evidence, consequence, and
    a concrete correction or experiment.
12. **Recommended synthesis** — decisions to adopt, reject, keep reversible, and
    record as ADRs/RFCs, plus any material dissent that must remain visible.

## Review rules

- Correctness, data-loss prevention, security, compatibility, verifiability,
  maintainability, feasibility, performance, extensibility, appearance, then
  fashion is the decision order.
- A feature is not Preview-ready unless its user-visible behavior, failure
  behavior, test oracle, owner, and target environment are identifiable.
- A dependency is not selected merely because proposals agree. Require official
  evidence, compatible license, maintained status, platform fit, security notes,
  and a PoC where the boundary is high-risk.
- A daemon, plugin runtime, custom protocol, separate repository, or compatibility
  promise carries the burden of proof.
- Process-name-only agent detection is not trustworthy state.
- Invalid keybinding configuration must be diagnosable while terminal input is
  preserved. Pass-through requires a reachable configurable exit and a
  non-keyboard fallback.
- Do not silently resolve disagreement. State the competing choices and the
  evidence or experiment that should decide them.

This is a critique, not the integration decision. The integrator must answer all
Blocker and High findings or explicitly retain the risk in an ADR.
