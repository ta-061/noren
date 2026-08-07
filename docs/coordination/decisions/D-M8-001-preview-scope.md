# D-M8-001 — The honest scope of the first public preview

Status: **Decided** (coordinator, 2026-08-07). Supersedes nothing; constrains
Milestone 8 planning.

## Question

What is the minimum honest scope of a `0.1.0-preview`, given the real state of
the code rather than the state the roadmap aspires to?

Three options were weighed:

- **A** — ship `0.1.0-preview` with the current bitmap font, documenting
  "ASCII-only rendering" as a known limitation.
- **B** — block the preview on a real font stack (rasterization, atlas, shaping,
  fallback).
- **C** — ship a narrower, explicitly-labelled preview that does not claim to be
  the product.

## How this was decided

Two independent sessions were consulted, each without sight of the other's
answer, and each instructed to reject false premises rather than agree with the
brief. Both were given the same facts to verify rather than to trust.

Both returned **C**. Both independently corrected two errors in the brief they
were given, and both raised the same blocker that the brief had not asked about.
The convergence is worth more than either answer alone because the reasoning
differed: one grounded the objection in the missing workspace feature, the other
in monochrome rendering being disqualifying for a terminal.

The brief's errors, corrected by both and re-confirmed against the current tree:

- The brief claimed 388 tests. The Milestone 2 completion evidence in
  `ROADMAP.md` records **353**.
- The brief claimed Milestone 3 was "largely verified." The milestone table in
  `ROADMAP.md` marks Milestone 3 **Not started**, and the tree bears this out
  more sharply than the brief allowed. The six M3 modules — `session`,
  `sidebar`, `palette`, `passthrough`, `session_supervisor`, and
  `session_persistence` — plus `mouse` (an M2 input deliverable, PR #79) are
  now files on `main` (PRs #77, #78, #79, #81, #82, and #84 merged after the
  brief was written), and PR #92 declared all seven in `lib.rs`, so they are
  compiled into the library and covered by CI's clippy and dead-code checks.
  But `main.rs` imports none of them, the linked binary contains no symbols
  from them, and nothing presents a sidebar to a user (Issue #88). "Verified"
  described lane state, not the application a user runs: launching the build
  still presents no workspace sidebar.

The distinction that carries the decision is neither *merged versus unmerged*
nor *declared versus undeclared in the library* — the first line moved with
PRs #77–#84, the second with PR #92, and either may move again — but *present
in the running application versus absent from it*. And the reason A fails does
not turn on any of these: it turns on the renderer, which no M3 merge touches.

## Decision

**Option C.** The first public artifact is an explicitly dated developer preview,
not "0.1.0-preview of the Noren terminal."

The reasoning that survives both sessions:

**Why not B.** The accepted requirements deliberately defer themes,
IME/CJK/HiDPI, and keyboard/accessibility work to "Later" (FR-012 in
`docs/requirements/v0.1.md`). Blocking the preview on a full font
stack re-litigates a decision already made and holds the artifact hostage to
Milestone 6, which has not started.

**Why not A.** The renderer is the reason, and no M3 merge touches it. It is
monochrome: the fragment shader `fs_main` in `renderer.rs` returns a constant
colour and the vertex format produced by `glyph_vertices` carries no colour
channel, so `ls --color`, `vim`, and Zellij's own status bar all draw in one
shade of green. The bitmap is worse than "ASCII-only": the `renderer.rs` test
`ascii_glyphs_are_distinct_and_unknown_is_question_mark` asserts
`glyph_rows('a') == glyph_rows('A')`, so the font cannot distinguish case, and
asserts a non-ASCII glyph collapses to `?`. This is now mechanically
documented, not merely asserted: the FR-005 rendered-frame oracle landed on
`main` with PR #89
(`a2271a9`), drives the real `wgpu` pipeline offscreen, and its defect tests —
written to pass only under a correct renderer and `#[ignore]`d today — record
exactly these two failures (`frame_oracle.rs`,
`lowercase_distinct_from_uppercase` and `non_ascii_glyph_is_not_the_question_mark`).
An oracle that exists, runs, and on its face documents the renderer's defects is
evidence against shipping as "the Noren terminal," not for it.

Noren's defining feature is the external workspace sidebar (ADR 0003; FR-009 in
`docs/requirements/v0.1.md`). Its modules are on `main` and declared in
`lib.rs`, but they are not wired into the application (Issue #88): `main.rs`
imports none of them, the linked binary contains none of their code, and
nothing presents a sidebar to a user. The merges and the library declarations
are real progress; what a user sees is unchanged. Filing monochrome rendering
and an invisible workspace under "known limitations" is dishonest by omission.

## Blocking preconditions

These are **not** M8 polish items; they gate the artifact reaching strangers.

1. **The FR-005 rendered-frame oracle exists — its findings must not be hidden.**
   FR-005 (`docs/requirements/v0.1.md`) requires that "state snapshots and a
   macOS rendered-frame
   capture match their oracles." PR #89 (`a2271a9`) supplied the rendered-frame
   half, driving the real pipeline offscreen. Its passing tests verify glyph
   geometry and grid mapping; its two ignored tests record the font's case-fold
   and non-ASCII-`?` defects. The gate is no longer missing — it is satisfied on
   structure and honest about its gaps. Hiding those gaps would be the dishonest
   path, and the project's evidence rule forbids it.
2. **Release-integrity gates.** NFR-009 (`docs/requirements/v0.1.md`) states
   signing,
   notarization, packaging, and release-integrity gates "must pass before Preview
   claims." Reproducible binaries, checksums, and toolchain provenance are
   required by NFR-008.
3. **Front-loaded honest documentation** — monochrome, ASCII-only and
   case-insensitive glyphs, no IME, no accessibility surface, macOS-only, and no
   workspace sidebar — stated where a reader meets them first, not in a footnote.
4. **Framing that does not imply** the workspace, colour, or CJK support exist.

## Consequences and owner decisions required

Precondition 2 collides with a reserved owner decision: signing and notarization
were deferred until immediately before distribution, and that moment *is* this
one. Signing keys, Apple certificates, and any public release are owner-only
actions. **Milestone 8 therefore stops at a release candidate.**

Two cheap items are worth doing regardless of scope, and both were raised
unprompted:

- **Wire colour to drawing.** Truecolor is already modelled in terminal state and
  simply never reaches the renderer. This is far cheaper than a font stack and
  removes the single most disqualifying visual defect.
- **Wire the merged M3 modules into the application.** They are on `main` and
  declared in `lib.rs` (PR #92), but `main.rs` consumes none of them and they
  do not reach the linked binary (Issue #88); the workspace is still invisible
  to a user. Wiring it — together with wiring colour to the renderer — is what
  would materially change what a preview can honestly claim.

## What would change this decision

Option C stands until a user launching the build sees a wired, visible workspace
sidebar **and** the renderer draws colour and real glyphs — distinct upper and
lower case, and real shapes for non-ASCII. Modules appearing in the repository,
or in the library's module list, does not change this: they must reach the
running application, and the renderer's defects must be fixed. Short of both,
re-arguing A would be re-arguing against the evidence.
