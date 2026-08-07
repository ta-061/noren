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

The brief's errors, corrected by both and confirmed against the tree:

- The brief claimed 388 tests. `ROADMAP.md:44` says **353**.
- The brief claimed Milestone 3 was "largely verified." `ROADMAP.md:11` says
  **Not started**, and the tree agrees: of the M3 modules, only `session.rs` is
  on `main`. `sidebar.rs`, `palette.rs`, `passthrough.rs`,
  `session_supervisor.rs`, and `session_persistence.rs` exist **only in unmerged
  PRs**. "Verified" described lane state, not `main`.

That second correction is the one that decides the question. It was also the
falsifier one session named for its own recommendation: *if M3 were merged,
option A would become defensible.* It is not merged, so A is not.

## Decision

**Option C.** The first public artifact is an explicitly dated developer preview,
not "0.1.0-preview of the Noren terminal."

The reasoning that survives both sessions:

**Why not B.** The accepted requirements deliberately defer CJK, IME, and
accessibility to "Later" (`docs/requirements/v0.1.md:46`, FR-012). Blocking the
preview on a full font stack re-litigates a decision already made and holds the
artifact hostage to Milestone 6, which has not started.

**Why not A.** What `main` contains today is a terminal *foundation*, not Noren.
Noren's defining feature is the external workspace sidebar (ADR 0003;
`v0.1.md:25` FR-009) — and it is not on `main`. Beyond that, the renderer is
monochrome: `renderer.rs:35` returns a constant colour and the vertex format at
`renderer.rs:117-125` carries no colour channel. A terminal where `ls --color`,
`vim`, and Zellij's own status bar are all one shade of green, and where CJK and
any non-ASCII path render as `?` (`renderer.rs:353`, asserted at `:458`), is a
technology demo. Filing that under "known limitations" is dishonest by omission.

The font is worse than "ASCII-only" implies: `renderer.rs:457` asserts
`glyph_rows('a') == glyph_rows('A')`. The bitmap does not distinguish case.

## Blocking preconditions

These are **not** M8 polish items; they gate the artifact reaching strangers.

1. **A rendered-frame oracle, or an explicit written admission that none
   exists.** FR-005 (`v0.1.md:21`) requires that "a macOS rendered-frame capture
   match their oracles." It does not exist — `ROADMAP.md:68-70` concedes glyph
   correctness is unverified by automation. Silently waiving the project's own
   PoC gate is the one path that would make the release claim dishonest, which
   is precisely what this project's evidence rule forbids.
2. **Release-integrity gates.** NFR-009 (`v0.1.md:46`) states signing,
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
- **Merge the verified M3 work**, which would materially change what a preview
  can honestly claim, and would reopen option A on its own stated falsifier.

## What would change this decision

If a rendered-frame oracle is built and the M3 workspace lands on `main`, the
premises behind C no longer hold and option A should be re-argued on the evidence
at that time.
