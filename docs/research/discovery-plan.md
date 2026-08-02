# Discovery research plan

## Objective

Produce source-backed, dated evidence before choosing product libraries or
presenting compatibility claims.

## Required reports

| Artifact | Required evidence | Completion rule |
| --- | --- | --- |
| Landscape report | Official project docs/source/licenses and current release/maintenance evidence for relevant terminals | Major alternatives, transferable lessons, and legal boundaries are explicit |
| cmux feature matrix | Public documentation and reproducible observation; no copied code/assets/marks | Every row has status, source, test plan, and honest Noren state |
| Zellij compatibility matrix | Official Zellij docs/source for supported versions plus local/remote fixtures | Key/protocol behavior has executable tests or an explicit unknown |
| Library comparison | Official docs/source, licenses, releases/commits, platform support, unsafe/dependency/security notes | Every required category has at least two viable candidates or a documented reason it does not |
| Agent integration report | Official hooks/plugins/structured output/help for Codex, Claude Code, and OpenCode | State claims map to trusted signals; unsupported state remains `Unknown` |
| SSH architecture report | OpenSSH manuals/source and candidate-library official evidence | Config/host-key/agent/proxy/reconnect/failure semantics are testable |
| Risk register | Evidence-linked likelihood, impact, owner, mitigation, trigger, and release gate | Top risks cover input, data loss, security, portability, performance, dependencies, and release integrity |
| Agent calibration | Identical task, preserved outputs, scored rubric, role decision | Every available role is completed, timed out, failed, or explicitly unavailable |

## Source policy

Use primary sources for technical and license claims: official documentation,
source repositories, release metadata, standards, and upstream issue/advisory
records. Record retrieval date and version/commit. Secondary sources may provide
context but cannot be the sole basis for an API, license, security, or
compatibility decision.

## Research boundaries

- Do not copy terminal implementations, product assets, or trademarks.
- Do not infer undocumented API or keybinding behavior.
- Do not hard-code mutable upstream defaults without version/provenance.
- Keep comparison evidence separate from an adoption decision; PoCs and ADRs make
  the decision later.
- Sanitize commands and logs before committing evidence.

## Execution order

1. Inventory current target versions and authoritative source locations.
2. Establish product/compatibility feature vocabularies.
3. Collect evidence in parallel without overlapping artifact ownership.
4. Cross-review citations, licenses, and testability.
5. Convert unresolved behavior into experiments or open questions.
6. Feed verified findings—not model preference—into Round 1 and ADRs.
