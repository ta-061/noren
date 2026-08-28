# Release review — checklist definition

This document defines the Noren release review: the last Milestone 8
deliverable before the owner-only steps (tag, publication, signing,
notarization). It is a checklist, not a narrative: every item names the
command to run or the file to read, and every verdict is one of
`pass`, `fail`, or `unverified` — `unverified` means "could not be
checked on this machine", never "probably fine".

A second reviewer following these items in order on the same commit must
reach the same verdicts. If an item's outcome depends on judgment rather
than observation, the item is wrong; fix the item.

The review's subject is the **release candidate**, decided by
[D-M8-001](../coordination/decisions/D-M8-001-preview-scope.md): an
explicitly dated developer preview, not "`0.1.0-preview` of the Noren
terminal". The review stops at a recommendation. Tagging, publishing,
signing, and notarizing are owner-only actions and are never performed by
the review.

## How to run it

Work from the exact commit under review. Record, before anything else:

- the commit hash (`git rev-parse HEAD`) and its branch;
- the machine: `uname -m`, `sw_vers` on macOS;
- the toolchain: `rustc --version --verbose`, `cargo --version --verbose`,
  `rustup target list --installed` — these must match
  `rust-toolchain.toml`, which is what makes the build reproducible in
  the NFR-008 sense (recorded provenance, not bit-identical promise).

Then execute the sections below in order. Verdicts go in a run record
named `release-review-<artifact>.md` in this directory, following the
shape of the executed items: claim, command, observed output, verdict.

Items marked **[#183]** can only fully pass once the release machinery
(`scripts/release/build.sh`, checksum publication, release notes) has
landed; until then they are `unverified` with the dependency named, not
`pass`.

## A. Build and provenance

1. **Pinned toolchain honored.** `rustc --version` reports exactly the
   channel pinned in `rust-toolchain.toml`. Any mismatch is `fail`.
2. **Release build succeeds.** `cargo build --release` exits 0 from the
   commit under review with a clean checkout.
3. **Provenance recorded** (NFR-008): the run record lists
   `rustc --version --verbose`, `cargo --version --verbose`, installed
   targets, macOS version, architecture, confirms `Cargo.lock` is
   committed and unmodified (`git status --porcelain Cargo.lock` empty),
   and confirms every direct dependency in `Cargo.toml` is pinned with
   `=`.
4. **Rebuild determinism.** Compute `shasum -a 256
   target/release/noren-app`, then force a recompile (`touch
   crates/noren-app/src/lib.rs && cargo build --release`) and compare.
   Identical digest is `pass`. (Same-machine determinism; cross-machine
   is claimed only as recorded provenance.)
5. **Checksum published with the artifact.** **[#183]** The release
   script generates the artifact digest and it is published beside the
   artifact with the release notes. On a tree without the release
   machinery this is `unverified`; a manually computed digest does not
   satisfy publication.
6. **Artifact identity and framing.** `file` reports the expected
   Mach-O arm64 binary; the artifact name, the Cargo version string, the
   window title, and the release notes all carry the preview framing
   decided by D-M8-001 (a dated developer preview) and none imply the
   product. **[#183]** for the notes half.

## B. The binary launches and operates

7. **Launch.** The binary runs in the foreground as a GUI application
   and creates one window (`osascript -e 'tell application "System
   Events" to tell process "noren-app" to get {position, size} of
   window 1'`).
8. **PTY chain agrees.** The process owns exactly one direct `/bin/zsh`
   child (`pgrep -lP <pid>`), and the child's tty size equals the window
   grid minus the sidebar columns (`SIDEBAR_COLS`, 16) and the status
   row (1): for the default 900x600 window at 10x20 cells that is
   **29 rows x 74 columns** (`stty -a -f /dev/<tty>`). Any other numbers
   mean the window-to-grid-to-PTY chain disagrees: `fail`.
9. **Persistence.** With a fresh `HOME`, first launch creates
   `~/Library/Application Support/Noren/sessions.toml`; a normal run
   writes nothing to stderr; sidebar state survives a relaunch.
10. **Config failure fails closed.** An invalid `config.toml` produces a
    typed message on stderr, no window, and a nonzero exit — never a
    partial startup.
11. **Clean shutdown.** Closing the window exits the app, reaps the
    child (no zombie), removes the pty device (`ls /dev/<tty>` fails),
    and stderr stays empty.
12. **Signature state matches the documented gap.** `codesign -dvvv
    target/release/noren-app` reports `Signature=adhoc` and
    `TeamIdentifier=not set`, exactly as
    [known-limitations](../known-limitations.md) states. A real signing
    identity here without owner action would be `fail` (undocumented
    state).

## C. Gates

13. `cargo fmt --all -- --check` exits 0.
14. `cargo clippy --workspace --all-targets -- -D warnings` exits 0.
15. `cargo test --workspace` — zero failures; every `ignored` test has a
    stated, justified reason; the passing count is recorded.
16. **FR-005 frame oracle gathered evidence.** `cargo test --test
    frame_oracle --manifest-path crates/noren-app/Cargo.toml` runs with
    zero `SKIP [` notices (requires a Metal adapter). A skip is
    `unverified`, never `pass`.
17. **Zellij live pass-through gathered evidence.** With `zellij
    --version` reporting the pinned 0.44.x on `PATH`, `cargo test
    --test zellij_live --manifest-path crates/noren-app/Cargo.toml`
    runs with zero `SKIP [` notices. Record the caveat of issue #153:
    no gating machine runs this suite; the review machine is the
    evidence.
18. `cargo deny check` — advisories, bans, licenses, sources all ok.
19. `python3 scripts/check_docs.py` — OK.

## D. Documentation honesty

Read `README.md`, `ROADMAP.md`, and
[known-limitations](../known-limitations.md) as a prospective user who
has never seen the repository, then check every claim about *current*
behavior against the binary and the code. The direction of an error
matters and must be recorded: overstating a capability is the dangerous
direction; overstating a limitation is stale but still erodes trust.

20. **README status block** (everything above "Everything below the
    status block describes intent") verified claim by claim.
21. **known-limitations** internally consistent and matching observed
    behavior — no clause contradicting another clause.
22. **ROADMAP** claims about current behavior match the binary.
23. **The signing/notarization gap is stated where a reader first meets
    claims**, not in a footnote: README status block plus the
    "What this preview is not" section of known-limitations.

## E. Known-limitations completeness

24. **First-ten-minutes walkthrough.** Launch the release binary and use
    it for ten minutes: open it, look at the sidebar, run a command,
    resize, relaunch. Record everything surprising, broken, or
    undocumented. Then check each against the three documents; anything
    a user hits that no document states is a finding, and this list is
    the review's main finding.

## F. Framing

25. **D-M8-001 framing holds end to end**: the artifact, its title, its
    version, and its notes present a dated developer preview; nothing
    implies the workspace, colour theming beyond the built-ins, CJK
    rendering, or the full product exist.

## Recording the verdict

The run record states, per item: the command run, the observed output
(quoted, not paraphrased), and the verdict. It ends with a
**recommendation** — `go`, `go-with-caveats`, or `no-go` — separating:
what is ready; what is missing but not blocking; what genuinely blocks
the release. The review recommends; the owner decides. A review that
concludes "ready" does not tag, publish, or sign anything itself.
