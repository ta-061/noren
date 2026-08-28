# UI/UX evaluation of the Noren preview — first pass

- Date: 2026-08-29. Evaluator: automated session (glm-ux), operating the
  running application, not reading intentions off the code.
- Binary: `cargo build --release` at `7654bb2` ("Merge pull request #183"),
  run as a real windowed process on macOS (arm64, 2× Retina).
- Scope: the six areas requested — first launch, sidebar, legibility, state
  communication, the three themes, and missing terminal expectations — plus a
  ranked proposal list. This document records what was observed on screen;
  anything not observed is marked `unverified`.

## Method

The release binary was launched repeatedly under controlled `$HOME`
directories (fresh, rich-config, and one per theme), driven through
synthesized Cmd+P, key, click, wheel, and drag events, and observed through
window screenshots decoded to pixels:

- text read back by reconstructing each rendered 5×7 glyph cell and matching
  it against the shipped glyph table (OCR against the app's own font);
- colors measured by full-frame pixel census per theme;
- ink coverage measured per cell on real rendered rows, with a reference
  measurement of Menlo (the macOS terminal monospace) rendered into
  terminal-sized cells through CoreText at the same 2× scale.

Environment notes: the evaluation machine ran other Noren instances owned by
parallel work sessions; every interaction was verified against this
instance's own window (by pid and window id) before being counted, and the
one externally-caused event that produced observable state (a burst of
sidebar row selections) is labeled as such where cited. The light wedge seen
at the bottom-left of window captures is the macOS rounded window corner
compositing the desktop behind it — a capture artifact, not application
content.

## Severity scale

- **S1 — violates what a terminal user expects.** Ordinary use is blocked or
  a basic universal affordance is absent. Not a preference.
- **S2 — significant degradation visible in normal use**, but the task is
  still completable.
- **S3 — polish.** Would be nicer; deferrable without harming ordinary use.

## Findings

| # | Finding | Severity | Issue |
| --- | --- | --- | --- |
| F1 | No visible cursor — the caret is never drawn | S1 | #197 |
| F2 | The primary interaction surface is undiscoverable: nothing on screen says `Super+p` opens the palette | S1 | #191 |
| F3 | Session lifecycle states are invisible: `running`/`exited`/`failed` are clipped off every session row | S1 | #196 |
| F4 | The viewport is frozen: no key, wheel, or indicator reaches the 10,000-line scrollback | S1 | #199 |
| F5 | Selection is tracked but produces no visible highlight | S2 | #57 (comment) |
| F6 | Dead session rows accumulate across launches; duplicates allowed, no cap | S2 | #188 |
| F7 | The sidebar is a flat monochrome list: five row kinds, three prefix styles, inconsistent truncation, no hierarchy | S2 | #198 |
| F8 | Glyph quality: unantialiased 5×7 grid — bold but blocky; measurable against a real terminal font | S2 | #192 (comment) |
| F9 | First launch buries the workspace: a wall of truncated worktree rows is the opening screen | S2 | #198 |
| F10 | State/errors surface as one sticky status line that truncates at the window edge and does not distinguish severity | S3 | — |
| F11 | The palette has no chrome: plain text rows, labels truncated at 16 columns | S3 | #191 (comment) |
| F12 | The window titles itself "Noren PoC" | S3 | #185 |
| F13 | `Cmd+Q` quits the running binary — refines the recorded "no keyboard quit" finding | informational | #189 (comment) |
| F14 | All three themes render coherently; `light` is usable, not merely compliant | positive | — |

## 1. First launch

Observed with a fresh `$HOME` (no config, no saved state), launched from a
git worktree:

- The window titles itself **"Noren PoC"** (#185) — the first thing a new
  user reads contradicts the release version.
- The sidebar opens with one live session row and **24 worktree rows**
  (`noren main`, `fix-117 ssh-pa`, `pool-a11y a11y`, …), truncated at the
  16-column wall. The status row reports `Noren worktrees: showing first
  24; 78 omitted`.
- The terminal shows a zsh prompt (`user@Mac fresh-home %`) — the shell
  starts in `$HOME` (the documented spawn policy).
- **Nothing on screen mentions `Super+p`, the palette, or any key.** No
  hint, no help line, no menu. Without the README the product's only
  interaction surface does not exist for the user: no session creation, no
  switching, no closing. This is F2 (S1): the empty state communicates a
  wall of truncated paths and says nothing about how to use the app.
- What a user *can* do unaided: type in the shell, scroll the sidebar with
  the wheel. Both were verified live.

First impression recorded during the session: tiny blocky text, a sidebar
full of cut-off names, and no caret — the product reads as a proof of
concept, which the title then confirms.

## 2. The sidebar

Observed with projects, agents, SSH aliases, worktrees, and multiple live,
exited, and failed sessions present:

- **Five row kinds, one look.** Everything renders in the theme's single
  default foreground — same color, size, and weight for sessions, projects,
  worktrees, SSH hosts, and agents, in all three themes (pixel census
  confirms one ink color across the whole sidebar). No icons, headers,
  separators, or backgrounds. The only selection cue anywhere is a `>`
  marker character.
- **Prefixes are legible but inconsistent.** `PRJ-OFF`/`PRJ-ERR`,
  `SSH-OFF`/`SSH-ON`/`SSH-ERR`, `AGT-OFF`/`AGT-ERR` — three different
  abbreviation styles for parallel concepts — while sessions and worktrees
  carry **no prefix at all**. At a glance a bare `fix-117 ssh-pa` row and a
  bare `session-4 loca` row are the same kind of thing.
- **16 columns is not enough, and truncation behaves three different
  ways.** Project `payroll` renders as `PRJ-OFF pay...` (ellipsis visible);
  worktree rows hard-cut mid-word (`fix-117 ssh-pa`, `rv-120 (detach`) with
  any ellipsis clipped off the sidebar's edge; session rows hard-cut inside
  their own detail (`session-1 loca`) so the lifecycle word is never shown
  (F3, #196). The long SSH alias truncates with a visible ellipsis
  (`SSH-OFF ali...`).
- **Scrolling works but is invisible.** The wheel over the sidebar scrolls
  the list (verified down and back up); there is no scrollbar, no
  position indicator, and no affordance suggesting more rows exist below
  the fold — the agent rows were unreachable until scrolled.
- **Accumulation.** Across one restart the sidebar gained a dead
  `session-1` beside the new `session-2` (the killed instance had persisted
  its state on exit — #188 confirmed live). During the session an external
  burst of row selections (labeled in Method) produced one new session row
  per worktree click — duplicates included (`pool-render` twice) — with no
  cap on session rows and no visual difference between live and dead rows.

Verdict: **partly distinguishable** — only by reading text, never at a
glance.

## 3. Legibility, measured

Measured on the real rendered pixels (not the source bitmaps):

- Geometry: the glyph box is 10×14 px inside the 10×20 px cell — 70 % of
  the cell height, 100 % of its width (3 px top and bottom insets).
- **Ink fraction per non-blank cell**: dark theme prompt row mean **0.162**
  (median 0.160, min 0.055, max 0.255); light theme text row mean **0.200**;
  high-contrast mean **0.166**. Reference: Menlo rendered into
  terminal-sized cells at the same 2× scale measures **0.114–0.116**.
  Noren's glyphs are ~1.5× *bolder* relative to the cell than a normal
  terminal font — the text is not faint.
- Strokes are 2 px of the 10 px advance (20 %); Menlo at terminal sizes is
  ~2 px of 26 px (~8 %).

Judgment: text is comfortably readable at normal viewing distance in all
three themes (contrast is high; see §5), but the rendering reads as
"calculator/LCD": a 5×7 design grid scaled 2× with hard edges, no
antialiasing, and the seven documented identical glyph pairs. Against
Terminal.app or iTerm — vector, antialiased SF Mono/Menlo at a comparable
absolute glyph height but with ~2.6× the horizontal resolution per
character — long reading sessions fatigue from the blockiness and
smearing of dense glyphs (`m`, digits) at 5 columns, not from size. F8
(S2); the measured numbers are recorded on #192.

## 4. State communication

Of the four session lifecycle states, **one of four is distinguishable on
screen, and only as text**:

- **Starting** — `unverified` (too transient to capture; a fresh session
  row was never caught showing it).
- **Running vs exited — observed identical.** A live session's `/bin/zsh`
  was killed from outside while watching the sidebar: the row stayed
  pixel-identical, because the lifecycle word lives in the clipped detail
  (F3, #196). Restored rows behave the same — `restored (not running)` is
  clipped too.
- **Failed — the only observable state**, and it is carried entirely by
  text: a missing agent command flips its row to `AGT-ERR broken` and sets
  the status line `Noren agent launch failed`; selecting the configured
  alias `box1` (ssh cannot resolve it) flips the row to `> SSH-ERR box1 c`,
  prints ssh's own error in the terminal, and sets `Noren ssh connection
  failed`; a deleted project root shows `PRJ-ERR ghost` (observed after an
  externally-caused selection). A failed session row is inserted into the
  list — with its `failed` state clipped off like every other session row.
- **SSH-ON** — `unverified` (no real connection was attempted).
- The status row is a **single sticky line** that truncates at the window
  edge (`Noren SSH: partial literal aliases; select one for source;` — cut)
  and keeps showing the last message after later, unrelated actions. No
  severity differentiation: launch failure and informational notices look
  identical (F10, S3).

Everything is monochrome text; no state anywhere is distinguished by
color, weight, or marker.

## 5. The three themes

Rendered and measured (display-space values from pixel census):

- **dark** (default): background ≈ 50,60,56; default foreground ≈
  227,247,233; ANSI red/green/yellow/white all render and resolve through
  the palette (verified on live SGR output). Coherent.
- **light**: background ≈ 251,251,248; foreground ≈ 97,104,111; the
  darkened ANSI set renders as saturated darks on off-white — no washed-out
  slot appeared in the rendered sample. The full ANSI test row was legible
  in one pass. **Usable, not merely compliant.**
- **high-contrast**: pure white on pure black with pastel ANSI entries —
  the strongest of the three; everything legible including the lowest
  slots.
- The three agree on structure: identical layout, identical single-color
  sidebar, identical chrome. A theme changes only the palette. **The set is
  coherent.**

`unverified`: the full 256-colour cube and truecolor on `light`;
program-paired colors are invisible by design when a program pairs a slot
against itself (observed: `SGR 31;41` words vanish into their background
blocks — the documented, out-of-contract case).

## 6. Missing versus what a terminal user expects

- **Cursor — confirmed absent** (F1, #197): typed text appears with no
  block, bar, or underline anywhere; frames captured mid-composition show
  the line ending in background pixels. Impact: every "where am I typing?"
  moment — after output scrolls, after switching sessions, mid-command —
  is a guess. This is the single most disorienting absence for a terminal
  user.
- **Selection highlight — confirmed absent** (F5): a Shift-drag over
  terminal text produced no visual change; the full-frame color census
  contains no selection color. Impact: users cannot see what they are
  about to copy. (The copy path itself is `unverified`.)
- **Scrollback viewport — confirmed absent** (F4, #199): after 60 lines of
  output, Shift+PageUp and the mouse wheel both leave the view unchanged,
  and no indicator hints that history exists. Impact: any output longer
  than one screen is permanently unreachable — builds, logs, test runs.
- Paste behavior and IME: `unverified` (documented as gated/absent).

## 7. What is already fine

- `Cmd+Q` quits the running binary (F13) — the recorded "no keyboard quit"
  finding needs rewording to "no *discoverable/documented* quit": the
  close button and an undocumented chord exist, and Cmd+Q works via the
  default application menu (observed live; recorded on #189).
- Window resize reflows the grid correctly (observed: the window was
  resized mid-session from 90 to 126 columns and content re-wrapped).
- Sidebar wheel-scrolling, session switching by click, palette command
  dispatch (`c` created a real session), and per-theme color resolution
  all work as documented.
- The three themes are coherent and `light` is genuinely usable (F14).

## 8. Ranked proposals — smallest changes, largest effect

Not implemented. Ranked by effect per unit of effort; each names the
finding it addresses.

1. **Draw the cursor** (F1, #197). One filled cell rectangle at the already
   tracked position, steady block to start — blink and shape options later.
   Effort: a single rect emission beside the existing glyph loop. Ranked
   first because the effort is trivial, every terminal user looks for the
   caret first, and the project's own limitations doc calls it "the first
   thing most people notice".
2. **Put a persistent palette hint on screen** (F2, #191). One drawn line —
   e.g. `⌘P PALETTE` — pinned in the status row or at the bottom of the
   sidebar, visible in every state. Effort: one string in an existing drawn
   row. Ranked second because without it the product's only interaction
   surface does not exist for an unaided user; no other fix matters if the
   user cannot find the palette.
3. **Make the session lifecycle word visible** (F3, #196). Smallest
   version: drop the `local · ` prefix so `running`/`exited`/`failed` fits
   within 16 columns. Better version: color the word (running green,
   exited/failed red) using the theme palette that already resolves
   colors. Effort: string formatting plus at most one color decision.
   Ranked third because "is this row alive?" is the sidebar's core
   question, and today the answer is always clipped away.
4. **Open a scrollback viewport** (F4, #199). Wire Shift+PageUp/PageDown
   (and the wheel when the program has not enabled mouse modes) to a
   scroll offset over the existing bounded buffer, with a one-line
   position indicator. Effort: days — a real seam (offset input to the
   frame layout), but no new data structures. Ranked fourth: ordinary
   daily use repeatedly needs history, but the user can still rerun a
   command today, unlike F1–F3.
5. **Differentiate the sidebar** (F7/F9, #198). Per-kind color or marker,
   consistent truncation with an in-view ellipsis, sessions-first ordering,
   and a header or cap for worktree floods. Effort: days and a real design
   decision (the row model already carries `kind`). Ranked fifth: high
   visible payoff but the largest judgement surface so far.
6. **Adopt a real font atlas** (F8, #192). Replace the hand-built bitmap
   with a prerendered vector monospace atlas. Deliberately ranked last and
   kept out of "smallest changes": it is milestone-scale (the known
   limitations already scope it as its own decision), while the
   measurements above show the current text is *readable* — bold and
   blocky, not invisible.

Explicit non-priorities read off the evidence: theme redesign (the three
themes are coherent), palette chrome before discoverability (F11 is S3
behind F2), and any new row-prefix vocabulary (F7 needs *fewer* styles, not
more).

## Issue trail

Filed from this evaluation: #197, #198, #199. Comments with observed
evidence: #191, #189, #192, #196. Pre-existing issues confirmed live:
#185, #188, #196.
