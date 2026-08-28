# Release review — `0.1.0-preview` release candidate, run 1

Executed 2026-08-28 against `main` at `c0d451d9bdfa831cf012f7946f8fab8ddff7e56f`
("Merge pull request #181"), on branch `docs/m8-release-review`, using the
checklist defined in [release-review.md](release-review.md).

Review machine: macOS 26.4.1 (25E253), arm64 (Apple Silicon, Metal
adapter present), `rustc 1.88.0 (6b00bc388 2025-06-23)`, `cargo 1.88.0
(873a06493 2025-05-10)`, installed target `aarch64-apple-darwin` — both
match the `rust-toolchain.toml` pin of 1.88.0.

Scope note: PR #183 (release machinery: `scripts/release/build.sh`,
checksum publication, `docs/release/notes/`, `readiness.md`,
`install-verification-checklist.md`) was **open and in rework** during
this review. This review runs against `main`, which does not contain
#183; the items it gates are marked `unverified [#183]` below.

Verdicts: **19 pass, 6 fail, 1 unverified** of 25 items.

## A. Build and provenance

| # | Item | Verdict | Evidence |
| --- | --- | --- | --- |
| 1 | Pinned toolchain | **pass** | `rustc --version` reports `rustc 1.88.0 (6b00bc388 2025-06-23)`; `rust-toolchain.toml` pins `channel = "1.88.0"`. |
| 2 | Release build | **pass** | `cargo build --release` → `Finished 'release' profile [optimized] target(s) in 36.19s`. |
| 3 | Provenance recorded | **pass** | Header above records rustc/cargo versions, target, macOS, arch. `git status --porcelain Cargo.lock` empty; every workspace dependency in `Cargo.toml` is `=`-pinned (libc 0.2.189, portable-pty 0.9.0, toml_edit 0.25.13, unicode-width 0.2.2, winit 0.30.13, wgpu 30.0.0, criterion 0.8.2). |
| 4 | Rebuild determinism | **pass** | Three compilations (one initial, two forced via `touch crates/noren-app/src/{main,lib}.rs`) produced the identical digest `c11ec1c7e7bd91f4dbd8c3c8e5d9b803a2ef514663cfc87dc51fc8af32abd710` (`shasum -a 256`, `cmp` reported 0 differing bytes). Same-machine determinism only; cross-machine is claimed as recorded provenance (NFR-008), not verified here. |
| 5 | Checksum published with artifact | **unverified [#183]** | The digest was computed manually (above), but publication beside an artifact via the release script arrives with #183 (`scripts/release/build.sh`, `docs/release/notes/0.1.0-preview.md`). Re-run item 5 after #183 lands. |
| 6 | Artifact identity and framing | **fail** | `file`: `Mach-O 64-bit executable arm64`, 6.6 MB — identity correct. Framing is not: the window title is **"Noren PoC"** (`with_title("Noren PoC")`, `crates/noren-app/src/main.rs:2177`; also the status strings "Noren PoC starting/ready"), the Cargo version is `0.1.0` with no `-preview` pre-release suffix, and no release notes exist on `main` (notes are #183). A dated developer preview introducing itself as "PoC" with an unlabelled version is inconsistent with D-M8-001 Option C. |

## B. The binary launches and operates

| # | Item | Verdict | Evidence |
| --- | --- | --- | --- |
| 7 | Launch | **pass** | `./target/release/noren-app` ran in the foreground; System Events reported process `noren-app` with window 1 at `{630, 220}`, size `{450, 332}` points (= 900x600 physical at 2x, 332pt including title bar). Window title: "Noren PoC". |
| 8 | PTY chain agrees | **pass** | One direct child: `pgrep -lP 12226` → `12246 zsh` (`/bin/zsh`). Child tty `ttys014` reported **`29 rows; 74 columns`** (`stty -a -f /dev/ttys014`): 900/10 − 16 sidebar columns = 74, 600/20 − 1 status row = 29. The chain agrees; note the ROADMAP's manual-gate numbers are stale (see finding O8). |
| 9 | Persistence | **pass** | Fresh-`HOME` run created `~/Library/Application Support/Noren/sessions.toml` (`version = 1`, one `[[sessions]] kind = "local"`, `selected = 0`) with empty stderr. A second launch in the real HOME restored 13 prior rows and appended a 14th (file rewritten, `selected = 13`). |
| 10 | Config failure fails closed | **pass** | Invalid `config.toml` (`this is not valid toml [[[`) → process exits, no window, stderr: `Noren configuration is unusable: configuration is not valid TOML at line 1, column 6` / `see docs/configuration.md; fix or remove the file (or unset NOREN_CONFIG) to continue`. Typed, content-free, actionable. |
| 11 | Clean shutdown | **pass** | Clicking the window close button (System Events) → app exited, child 12246 reaped (no zombie), `/dev/ttys014` no longer exists, stderr log 0 bytes after a full run, `sessions.toml` persisted. |
| 12 | Signature state matches docs | **pass** | `codesign -dvvv`: `Identifier=noren_app-…`, `Signature=adhoc`, `TeamIdentifier=not set` — exactly what known-limitations "What this preview is not" states. The signing/notarization gap is real and documented (see item 23). |

## C. Gates

| # | Item | Verdict | Evidence |
| --- | --- | --- | --- |
| 13 | fmt | **pass** | `cargo fmt --all -- --check` → exit 0. |
| 14 | clippy | **pass** | `cargo clippy --workspace --all-targets -- -D warnings` → `Finished 'dev' profile [unoptimized + debuginfo] target(s) in 19.74s`, no warnings. |
| 15 | tests | **pass** | `cargo test --workspace` → **1111 passed, 0 failed, 3 ignored**. The ignored three are each justified: `system_clipboard_round_trips_user_text` (touches the real macOS clipboard), `baseline_mixed_quadratic_measurement` (temporary baseline), `fifo_subprocess_helper` (invoked only by parent tests). |
| 16 | FR-005 frame oracle | **pass** | `tests/frame_oracle.rs`: **59 passed, 0 failed, 0 skipped** — rendered-frame evidence was gathered (Metal adapter present; zero `SKIP [` notices in the full log). Includes the two former defect tests (`lowercase_distinct_from_uppercase`, `non_ascii_glyph_is_not_the_question_mark`) now guarding the PR #141 font fixes. |
| 17 | Zellij live pass-through | **pass** (caveat) | `zellij --version` → `zellij 0.44.3` (pinned corpus version); `tests/zellij_live.rs`: **14 passed, 0 skipped**, suite ran in 3.36s with zero `SKIP [` notices. Caveat recorded per issue #153: no gating machine runs this suite — this review machine is the only evidence. |
| 18 | cargo deny | **pass** | `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`. |
| 19 | docs validator | **pass** | `python3 scripts/check_docs.py` → `Documentation structure, local links, whitespace, UTF-8, and secret patterns: OK`. |

## D. Documentation honesty

| # | Item | Verdict | Evidence |
| --- | --- | --- | --- |
| 20 | README status block | **fail** | Four false claims, all stale in the overstates-a-limitation direction (details O1–O4 below). No claim was found that overstates a capability. |
| 21 | known-limitations consistent | **fail** | Two internal contradictions (O5, O6 below) — the document disagrees with itself about the SSH sidebar cap and about session switching. |
| 22 | ROADMAP matches binary | **fail** | One internal contradiction (O7) and one stale evidence record (O8) below. |
| 23 | Signing gap stated up front | **pass** | README status block: "a local build carries only macOS's automatic ad-hoc signature — no signing identity, no notarization"; known-limitations closes with "What this preview is not" stating NFR-009 gates are unmet. Observed signature state matches (item 12). |

### The eight overstatements (O1–O8)

Every false claim found overstates a **limitation** (docs lagging behind
shipped capability); none overstates a capability. That is the less
dangerous direction for a preview audience, but D-M8-001 precondition 3
makes front-loaded honest documentation a blocking precondition, and a
document contradicting itself is not honest-by-construction.

- **O1 — README:21** "(at most 24 rows)" for SSH aliases. Code:
  `MAX_SSH_SIDEBAR_HOSTS: usize = 64` (`crates/noren-app/src/main.rs:84`);
  pinned by tests asserting `showing first 64 of 70; 6 past sidebar
  bound`. Truth: **64**.
- **O2 — README:13–14** "The default palette and theme are not
  user-configurable." Code: `[theme] name` in `config.toml` selects
  `dark`/`light`/`high-contrast` (`crates/noren-app/src/theme.rs`,
  `config.rs`; `configured_theme_reaches_the_app_renderer_input`;
  documented in `docs/configuration.md` §`[theme]`). Truth: **three
  built-in themes are selectable**.
- **O3 — README:33** "project rows and agents remain modelled but
  unreachable." Code and tests: `[[projects]]` rows launch real
  directory-rooted PTY sessions; `[[agents]]` rows launch shell-free
  argv PTYs (ROADMAP M3 status records both; `main/tests.rs` exercises
  launch, failure, and persistence for both kinds). Truth: **both are
  reachable**.
- **O4 — README:73–74** "while reachable project rows and agent
  launching have not." Same staleness as O3.
- **O5 — known-limitations:303** "It shows only the first 24 literal
  aliases" contradicts known-limitations:98–100 in the same file
  ("retains at most `MAX_SSH_SIDEBAR_HOSTS` (64)"). Truth: **64**.
- **O6 — known-limitations:120–122** "one live PTY at a time … there is
  still no multi-session switching" contradicts the same document's
  "Session switching exists, within one viewport" clause (and ROADMAP M3
  status): multiple live PTYs exist simultaneously (parked sessions keep
  draining) and switching between them is real. Truth: **multi-session
  switching within one viewport exists**.
- **O7 — ROADMAP:182–183** ("What blocks a public preview") "at most 24
  positive literal aliases" contradicts ROADMAP:119 in the same file
  ("At most 64 … `showing first 64 of 70`"). Truth: **64**.
- **O8 — ROADMAP:63–65** manual gate: "that child's tty reported
  `30 90` — the 900x600 window divided by the 10x20 cell". Re-run at
  `c0d451d`: the child reports **29 rows × 74 columns**, because the
  sidebar (16 columns) and status row (1) now reserve space — the
  mechanism claim (window→grid→PTY agreement) holds, the recorded
  numbers are pre-sidebar and stale.

## E. Known-limitations completeness

| # | Item | Verdict |
| --- | --- | --- |
| 24 | First-ten-minutes walkthrough | **fail** — nine findings; six undocumented (below). |

### First ten minutes with the release binary

What a user actually hits, in order. "Documented" means one of README /
known-limitations / ROADMAP states it.

1. **No visible cursor** — you type and nothing marks the insertion
   point. *Documented* (known-limitations leads with it) — still the
   first thing anyone notices, and the honest docs are doing their job
   here.
2. **The window introduces itself as "Noren PoC"** (title bar, and the
   status line while starting: "Noren PoC starting" → "Noren PoC
   ready"). *Undocumented* — no doc mentions the title; inconsistent
   with a "0.1.0-preview" artifact (finding for item 6).
3. **The terminal is 29 rows × 74 columns** — narrower and shorter than
   any familiar default (80×24), because 16 of 90 columns are the
   sidebar and 1 of 30 rows is the status line. *Undocumented* as a
   concrete size; the sidebar reservation is documented but the
   resulting default grid size is not.
4. **Dead session rows accumulate across launches.** Every launch
   appends a new local-session row and restores every previous row as a
   dead "Restored" entry (observed: 13 restored + 1 new = 14 after two
   runs). Nothing prunes them; the sidebar fills with rows that cannot
   take the live view. *Partially documented* — "A restored session's
   shell is not running" is stated; unbounded accumulation is not.
5. **No keyboard quit.** `Super+Escape` is the exit-to-workspace
   leader, not quit; `Cmd+Q` does nothing for a bare binary; the only
   quit is the window close button. *Undocumented.*
6. **A config typo makes the app refuse to start, stderr-only.** The
   failure mode is typed and honest, but the message goes only to
   stderr; launched from a terminal that is fine, from Finder it would
   look like a silent bounce-and-die. *Undocumented* failure mode (the
   config schema itself is documented).
7. **No in-UI hint that `Super+p` exists.** The palette is the primary
   interaction surface and nothing on screen mentions it; a user
   without the README has no way to discover session creation,
   switching, or closing. *Undocumented UX gap.*
8. **Glyphs are visually tiny.** The 5x7 bitmap is centered in a 10x20
   cell, so text occupies roughly half the cell — the first visual
   impression is "very small characters", distinct from the documented
   "bounded coverage" claim. *Undocumented as an appearance fact*
   (geometry is code-evident: `glyph_rows` 5x7 in 10x20 cells).
9. **The binary is named `noren-app`**, not `noren`. Cosmetic, but the
   artifact name should be decided before publication (the #183 build
   script names it). *Undocumented.*

## F. Framing

| # | Item | Verdict |
| --- | --- | --- |
| 25 | D-M8-001 framing end to end | **fail** — the README status block and known-limitations frame the preview honestly (dated developer preview, "not nearly done", ad-hoc signature stated), but the artifact itself says "PoC" (finding 2) and `0.1.0` (no `-preview`), and no release notes exist on `main` [#183]. The framing survives in the docs and fails in the artifact's own surface. |

## Recommendation: **go-with-caveats**

Not a declaration of readiness — a recommendation with evidence
attached, for the owner to act on.

**Ready now** (no caveats): the terminal foundation and workspace slice
are real and verified — 1111 tests green on this machine including the
rendered-frame oracle (59/59, evidence gathered, not skipped) and a live
Zellij 0.44.3 pass-through run (14/14); the release binary builds
deterministically on the pinned toolchain, launches, runs the PTY chain
correctly (29×74 = window minus sidebar minus status row), persists
state, fails closed on bad config, and shuts down cleanly with children
reaped. The known-limitations core (cursor, IME, accessibility, font
coverage, macOS-only) is honest where it is not self-contradicting, and
the signing gap is stated exactly where a reader first looks.

**Missing but not blocking:**

- Signing and notarization unmet (NFR-009) — reserved to the owner by
  D-M8-001; the gap is documented; the review stops before it by
  design.
- Zellij live evidence gates no machine (issue #153) — evidence was
  gathered on this review machine.
- No keyboard quit, no in-UI palette hint, tiny glyphs, `noren-app`
  binary name — preview-acceptable rough edges; file issues, do not
  hold the release.

**Genuinely blocking a `0.1.0-preview` tag:**

1. **The eight documentation overstatements (O1–O8).** D-M8-001
   precondition 3 makes front-loaded honest documentation a gate, and
   four README claims are false plus two documents self-contradict.
   All eight are doc-only fixes (no code changes), cheap, and must land
   before a tag: a preview reader who catches the docs contradicting
   themselves will not file a bug — they will leave.
2. **Release machinery from #183 (checksum publication, release notes,
   readiness/install-verification docs) must land and this review must
   re-run items 5, 6, and 25 against it.** On `main` today the checksum
   is a manually computed number with no publication path, and there
   are no release notes to review for honesty.

**Bottom line:** do not tag `main` at `c0d451d` as-is. Land the O1–O8
doc fixes and #183, re-run this checklist (expect items 5, 6, 25 to
move), and the candidate is in shape for the owner's go/no-go on tag,
publication, and signing.

*This review created no tag, published nothing, and signed nothing; all
three remain owner-only actions.*
