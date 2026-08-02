# Project principles

These constraints apply to Noren design, code, documentation, reviews, and
releases.

## Product integrity

1. Preserve terminal input before adding convenience shortcuts.
2. Do not claim that a planned, stubbed, partial, or untested feature works.
3. Separate UI themes from terminal ANSI palettes and test both light and dark
   presentation.
4. Treat unknown agent state as `Unknown`; process names alone do not prove an
   agent state.
5. Publish implementation status, known limitations, and release evidence
   honestly. Avoid unsupported superlatives or compatibility claims.

## Engineering gates

1. Requirements, non-functional requirements, architecture, and a threat model
   precede production implementation. Only bounded experiments may live under
   `experiments/` before that gate.
2. Important choices require independent proposals, cross-review, an integration
   decision, and an ADR or RFC when appropriate.
3. Terminal parsers, PTYs, SSH, cryptography, and font shaping are not casual
   greenfield implementations. Library claims require evidence from official
   documentation/source, license terms, maintenance state, and a PoC.
4. Every functional and non-functional requirement must state how it will be
   measured and what passes.
5. Tests, CI, review, and release evidence—not generated code volume—determine
   completion.

## Security and reliability

1. Never store or log API keys, access tokens, cookies, SSH private keys, or SSH
   passphrases.
2. Never concatenate external input into a shell command. Keep process arguments
   structured.
3. Bound OSC payloads and untrusted configuration; protect IPC and filesystem
   paths; minimize `unsafe`; require a `SAFETY` explanation at every unsafe use.
4. Preserve existing valid state on failed configuration reloads, use atomic
   writes, and version persisted workspace data.
5. A remote or child-process failure must not crash or block the local UI.

## Collaboration

1. Every change starts from an Issue and uses a non-`main` branch or worktree.
2. The implementer does not provide final approval. Reviewers inspect the diff,
   code, tests, and requirements rather than trusting a summary.
3. Agents receive non-overlapping file ownership. Decisions are recorded in
   Issues, PRs, Markdown, commits, RFCs, and ADRs rather than assumed shared
   memory.
4. Discussion normally stops after independent proposals, cross-critique, and an
   integration decision. Residual disagreement remains visible in the ADR.
5. Changes to credentials, repository deletion, history, visibility, or access
   controls require explicit human confirmation.
