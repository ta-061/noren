# Design council process

Production implementation is gated on the three rounds below. Technical spikes
before the gate must be disposable and confined to `experiments/`.

## Round 1: independent proposals

Codex, Claude Code, codex-lab, GLM, Qwen, and Fugu receive the same product brief
without access to one another's responses. Each proposal must cover the product
value, users, non-goals, Preview scope, architecture, crate/repository structure,
library candidates, rendering, PTY, SSH and remote sessions, Zellij/keybindings,
agent integration, themes, plugins, security/performance/portability risks, ten
top risks, tests, sequencing, release gates, disposable choices, and avoided
overengineering.

Outputs are stored under `docs/coordination/proposals/` with command/model
provenance. Missing or timed-out contributors remain visible; another model's
answer is never relabeled as theirs.

## Round 2: cross-critique

Every available reviewer receives the proposal set and evaluates claims and
evidence—not model identities. Reviews cover feasibility, missing requirements,
security, performance, Zellij conflicts, SSH failure modes, license risk,
testability, repository boundaries, and smaller alternatives. Outputs live under
`docs/coordination/reviews/`.

## Round 3: integration

The integrator resolves proposals and critiques in this order: correctness,
data-loss prevention, security, compatibility, verifiability, maintainability,
feasibility, performance, extensibility, appearance, and fashion. Material
choices receive ADRs; disputed or novel mechanisms receive RFCs. Dissent and
uncertainty remain recorded.

Required integrated artifacts are product, functional, and non-functional
requirements; architecture and repository/API contracts; threat model and
security requirements; test strategy; release plan; and risk register. Claude
performs a final security/maintainability review, and codex-lab independently
checks that requirements are executable and measurable.

## Gate to implementation

The gate opens only when all required artifacts exist, every Preview requirement
maps to a test/evidence owner, no unresolved blocker invalidates the architecture,
and the independent reviews are addressed or explicitly accepted in an ADR.
