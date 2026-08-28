# Noren UX plan — synthesis and ranking

Date: 2026-08-29 (JST)
Status: adjudication and planning only. No UI change is implemented here.

## Inputs and their status

| Input | Branch | Status |
| --- | --- | --- |
| Independent evaluation, position B (codex) | `docs/ux-evaluation-b` | Read in full (`docs/design/ux-evaluation.md`). Built and ran the release binary, captured the production render stream, measured ink ratio 27.5% and `states_distinguishable=2/4`. |
| Independent evaluation, position A (GLM) | `docs/ux-evaluation` | **Nothing landed.** The branch has zero commits beyond `main`; its worktree (`w-ux`) holds no uncommitted evaluation file. The lane is treated as produced-nothing, not waited on. Every "agreement" below is therefore agreement across **two** independent evaluations, cross-checked by a third read of the product done for this plan. |
| Comparative study (codex) | `docs/ux-comparative` | Read in full (`docs/design/ux-comparative-study.md`). 18 findings (F-01..F-18), peer evidence all first-party-documented, Noren side observed/source-verified. |
| This plan's own product read | — | `cargo build --release` (clean, 27s), run unconfigured with an isolated `HOME` (entered the event loop, spawned zsh, persisted `sessions.toml`, no output), run with a missing `NOREN_CONFIG` (clean typed error naming the file). Source checks cited inline below. |

## The judging constraint

`state/UX-PRINCIPLES.md` (owner-set, 2026-08-29) makes simplicity and freedom two
binding requirements. Every ranked item below answers both:

- **Q1 (simple):** does a user who reads nothing get a good result?
- **Q2 (free):** can a user who wants control get it without patching source?

Rejected resolutions (not negotiable here): configurable-instead-of-defaulted,
beginner modes that hide functionality, "it is documented", deleting advanced
capability to shrink surface.

## 1. Where the two evaluations agree

Agreement across independent evaluation methods (a frame-stream rasterization
vs. a source-verified peer-convention study) is the strongest signal available.

| # | Agreed finding | Why it survived two separate looks |
| --- | --- | --- |
| A1 | The command palette has no visible opener and `No sessions` is a dead end (`EMPTY_SIDEBAR_MESSAGE`, `sidebar.rs:190`) | Verifiable in one minute of unaided use; both built and ran the binary. Every Noren-differentiating action hides behind an undisclosed chord. Issue #191 covers the hint half. |
| A2 | No terminal cursor is rendered | Proven twice by different methods: byte-identical frames on cursor move (B), and `renderer.rs` contains no cursor drawing at all (B's evidence, re-verified for this plan: the word "cursor" does not occur in `renderer.rs`). |
| A3 | Selection works but is invisible — a real selection changes no pixel | Same double method: identical frames around selection (B), no selection styling in the renderer (source-verified here). |
| A4 | The 16-column sidebar destroys identity and lifecycle (`SIDEBAR_COLS = 16`, `renderer.rs:50`, hardcoded) | Measured by B (visible slices), structurally analyzed by the comparative (F-02: the 8-char `TYPE-STATE` prefix spends the identity budget). Filed as #196. |
| A5 | The 5×7 bitmap font is below terminal-grade legibility: 27.5% mean ink, hard 2×2 blocks, CJK/emoji render as a replacement glyph, and cell-size config does not scale glyph ink | Measured by B, generation-gap-framed by the comparative (F-17), confirmed here: `FontConfig` scales the grid, not the glyph raster. Filed as #192. |
| A6 | Scrollback is retained but un navigable and unindicated | B proved frame-identical output with and without history; the comparative (F-14) established the deeper half: there is no navigation at all (only `sidebar_scroll_offset` exists in `main.rs` — no terminal viewport offset). |
| A7 | Themes are palette-complete but interface-incomplete: no semantic color for selection/error/state, sidebar and terminal share one undivided plane | B measured readability of the three palettes; the comparative showed peers make state spatial and redundant (F-10). The fix is layout+semantics, not palette substitution. |

## 2. Where they disagree, and who is right

**D1 — Which gap is first: palette discoverability or the cursor?**
B ranks the palette hint #1; the comparative's time-to-notice puts the cursor #1
(noticed in seconds) and the hidden palette #3.
*Adjudication: B, for this plan's ranking criterion.* The two order different
metrics — first-*notice* (comparative) vs. first-*strand* (B). A missing cursor
degrades every keystroke but the shell still works; an undisclosed palette
permanently strands the new user and hides the entire product thesis. With
impact-per-effort as the rank rule the dispute nearly disappears: the hint is a
few cells of copy, the cursor is a renderer primitive. Both stay in the top
three; the hint wins on effort asymmetry, not because the comparative was wrong
about notice order.

**D2 — Is the gap "no scrollbar" or "no navigation"?**
B's proposal 4 asks for a "scroll position/live-tail indicator"; the
comparative corrects the premise: WezTerm ships its scrollbar off by default
and Alacritty shows a position indicator only in search/vi mode. The real
convention is *leave-the-bottom navigation with orienting feedback*, which
Noren lacks entirely.
*Adjudication: the comparative is right* — it examined peer evidence B never
gathered, and B applied a standard (persistent scrollbar) the product does not
owe its users. B's measurement (identical frames) stands as the defect proof.
The plan item is therefore "navigation first, indicator only while scrolled",
not "draw a scrollbar".

**D3 — How severe is the font?**
B ranks the font #5 of 6, below the feedback repairs; the comparative's
notice-rank puts it #2, immediately after the cursor.
*Adjudication: the comparative is right on severity, B is right on sequencing.*
B's rasterization was ASCII-only, so it could not weigh what the comparative
saw: prompts containing CJK, emoji, or Nerd Font symbols collapse to one
replacement glyph — functional unreadability, not discomfort (and the owner
works in Japanese). But the effort is the largest in the list (a text-stack
replacement), so it ranks below the cheap feedback repairs here while being
explicitly *not* deferred behind semantic styling.

**D4 — What shape is the sidebar fix?**
B proposes an adaptive minimum width, a divider, and stable type/state columns.
The comparative argues (F-04, F-07) the peer answer is progressive disclosure
and resizing — and warns (F-08, F-09, F-11) that a permanently narrow rail is
the wrong shape for management tasks.
*Adjudication: both, in the two halves the owner's constraint demands.* B's
layout fix is the Q1 half — the *default* must preserve kind, identity, and
state with zero configuration. The comparative's resizability is the Q2 half —
the width must then be user-controllable. Neither evaluation stated the Q2
half; `SIDEBAR_COLS` is a hardcoded constant today.

Findings only one evaluation has are not disagreements: the Zellij-handoff
invisibility (F-06, comparative only — B's scope was the closed first-launch
surface) is kept as a ranked item on the comparative's evidence.

## 3. What all evaluations missed

Found by reading and running the product for this plan, judged against the
owner's constraint:

- **M1 — IME input is dropped entirely.** `main.rs:3388`: `WindowEvent::Ime(_)`
  records a drop and discards the composed text ("IME support itself is
  deferred"). A Japanese IME user cannot type Japanese into Noren *at all*;
  dead-key composition is likewise dropped (`KeyDropReason::ImeOrDeadKey`).
  B's raster tests were ASCII-only; the comparative documented CJK *rendering*
  absence (F-17) but neither ranked CJK *input*. It is also absent from
  `docs/known-limitations.md`. For a product whose owner sets principles in
  Japanese, this is the largest miss.
- **M2 — Themes are not customisable.** `[theme]` accepts exactly one string
  from a closed three-name vocabulary (`config.rs` `parse_theme`; `theme.rs`).
  A user who wants any other palette must patch source. Principle 6 names
  *themes* explicitly as a default that must be overridable. Both evaluations
  graded theme legibility and never noticed the freedom gap. The correct shape
  exists already in-product: validate user palettes against the same measured
  4.5:1 floor the built-ins are pinned to (principle 5's model).
- **M3 — Sidebar width is neither adaptive nor user-controllable.**
  `SIDEBAR_COLS = 16` is a `pub(crate) const`. #196 tracks the clipping as an
  information defect; the configurability axis (resizable window rail, or a
  configured width) is unnamed by either evaluation.
- **M4 — The shell is hard-fixed at `/bin/zsh`** by threat-model TM-01, while
  principle 6 names "launch policy" as something that must be overridable.
  This is a genuine conflict between two owner artifacts. The plan does not
  resolve it silently; it flags it for an explicit owner decision (e.g. a
  validated allowlist of absolute shell paths preserves the no-`PATH`-lookup
  security property while restoring freedom).
- Minor, folded into ranked items: the palette's `F` (sidebar focus) command
  is a visible no-op (comparative mentions it as evidence, ranks nothing); no
  config file or pointer to one is surfaced on first run (#190 covers the
  related silent Finder-launch death).

## 4. The ranked plan

Ranking rule: user impact per unit of effort, with both owner questions
answered per item. Effort: S ≤ a day-ish, M = a focused lane, L = multi-lane.

| Rank | Item | Found by | Effort | Q1 (reads nothing) | Q2 (wants control) |
| --- | --- | --- | --- | --- | --- |
| 1 | **Palette affordance + actionable empty state.** A persistent, visible opener hint — e.g. a `⌘P Commands` affordance rendered from the *configured* `palette_open` chord — and an empty state that says how to recover (`No sessions — <configured chord>, then C`). | B #1; comparative F-16 (#191 covers the hint half) | S | Yes — the closed surface finally discloses the product's actions; the empty state stops being a dead end. | Yes — the hint derives from `[keys] palette_open`, so a user who rebinds sees their chord, never a stale hardcoded one. |
| 2 | **Render the terminal cursor.** Theme-aware block/bar at the snapshot cursor, distinct focused/unfocused treatment. | B #2; comparative F-13 | M | Yes — restores the baseline feedback every terminal owes, by default, no config. | Yes — colors/shape come from the theme and inherit theme overrides; a later `[cursor]` style key is additive, not required. |
| 3 | **Sidebar geometry that preserves kind, identity, state.** Wider default (or reflowed columns) so the lifecycle word survives, one-cell divider/gutter from the terminal, ellipsis on identity only, never on state. Then make the width user-resizable. | B #3/#4/#5/#7; comparative F-02/F-04 (#196 filed) | M | Yes — the *default* width is chosen so a two-digit session shows name + state; the divider ends sidebar/terminal concatenation. | Yes — resizable rail (VS Code-style drag) once the default is right; configurability is the second step, never the substitute. |
| 4 | **Selection highlight.** Theme-owned selection background over the exact cell range about to be copied. | B #3 (proposal 4); comparative F-15 | S–M | Yes — visible by default; copying stops being blind. | Yes — selection colors are theme entries, overridable the moment user palettes land (rank 8). |
| 5 | **IME input.** Handle `WindowEvent::Ime` — commit composed text to the PTY; preedit display can follow. | **Missed by all (M1)** | M | Yes — Japanese/dead-key users can type their own language with zero setup. | Yes — no freedom removed; nothing new to configure for correct behavior. |
| 6 | **Scrollback navigation + orientation.** Wheel/page input can leave the live tail; an indicator (thumb/counter/mark) appears only while scrolled. | B #9; comparative F-14 (adjudicated D2) | M | Yes — retained history becomes reachable and oriented by default. | Yes — the indicator is chrome, not a preference dial; a later retention-cap key is additive. |
| 7 | **Terminal-grade text stack.** Replace the 5×7-only path with an antialiased monospace raster at a normal default size; keep the bitmap path selectable as a fallback; scale glyph ink with configured cell size. | B #5/#6; comparative F-17 (#192 filed) | L | Yes — the *default* rendering becomes comfortable and covers CJK/emoji instead of substituting them. | Yes — family/size become configurable on top of a sane default; the bitmap path stays for those who want it. |
| 8 | **User-defined palettes, validated.** Extend `[theme]` with color entries; reject any palette that fails the built-ins' measured 4.5:1 floor, with a typed error naming the failing slot. | **Missed by all (M2)** | M | Yes — nothing changes for the unconfigured user; the three built-ins remain the defaults. | Yes — restores the theme freedom principle 6 names, honestly: validated, typed, never a silent fallback to dark. |
| 9 | **Semantic state styling + Zellij handoff.** Theme-owned selected/error/starting/connected treatments once state survives layout (rank 3); one status-line hint that tabs/panes belong to Zellij inside a session. | B #6/#8; comparative F-05/F-10/F-06 | M | Yes — errors and ownership become scannable without reading anything. | Yes — all semantic colors are theme entries; the hint is honest copy, not a mode. |

Ordering notes: rank 4 (selection) lands before rank 6 (scrollback) because it
is cheaper and every copy attempt hits it; rank 5 (IME) sits between them
because it blocks an entire language of user, but the items above it block
*everyone including that user*. For a Japanese-first audience, 4 and 5 swap.

## 5. Deliberately ranked LOW

- **Attention routing** (unread badges, progress, agent output signals —
  comparative F-05's cmux parity list): real product value, but it presupposes
  ranks 3 and 7 (state must survive layout; a signal needs legible glyphs).
  High effort, low yield until then.
- **Sidebar grouping/collapse/drag-reorder/multi-select** (comparative F-03/F-04
  inventory): power surface before basics; no defect is filed against it.
- **Palette modal separation** (B severity 10): recurring friction, not task
  failure; the palette is legible once open. Cheap to piggyback on rank 3's
  divider work, not worth its own lane.
- **Animations, icons, rounded panels, theme proliferation**: explicitly
  omitted by B; this plan concurs — none repairs an observed task failure.
- **Window title (#185), keyboard quit (#189), dead-row accumulation (#188)**:
  already filed; real, but none is a UX-plan driver.

## 6. Principle traps — do not implement these ways

1. **Adding `[ui] show_palette_hint = true` instead of showing the hint by
   default.** Configurable-instead-of-defaulted is a named rejected resolution.
   The hint ships visible; a key, if ever added, can only turn it *off*.
2. **Hardcoding `⌘P` in the hint or empty-state copy.** `palette_open` is
   configurable (`config.rs`); the rendered hint must be generated from the
   active `KeymapConfig`, or a rebound user is shown a dead chord — dishonest
   customisation.
3. **Making sidebar width *only* configurable (or *only* fixed-wider).** Either
   half alone fails one of the two questions: the default must preserve state
   words unconfigured, and the width must then be user-controllable.
4. **A "simple mode" that hides agent/SSH/worktree rows.** Hiding functionality
   behind a flag is a named rejected resolution; the catalog is the product.
5. **A font-size key that only grows cell spacing.** This trap is *live today*:
   `[font] cell_width/height` scale the grid while glyph ink stays 2×2 —
   configuration that cannot obtain the result it promises. The text-stack item
   must fix ink scaling, not document the knob.
6. **Custom palettes without the contrast validation.** Principle 5 (typed,
   validated, honest) plus principle 4 (a default that hides text is a defect)
   require rejecting a user palette that fails the 4.5:1 floor, with an error
   naming the failing slot.
7. **Deleting the bitmap font path (or the `F` palette command) when replacing
   it.** Removing capability to shrink surface is a named rejected resolution;
   the bitmap path stays selectable, and `F` gets behavior or an honest
   removal decision — not silent deletion.

## 7. Issue mapping

Already filed: #196 (sidebar clipping → rank 3), #191 (palette hint → rank 1,
hint half), #192 (glyph size → rank 7), #190/#189/#188/#185 (related, low).
Filed from this plan: cursor (rank 2), actionable empty state (rank 1, recovery
half), selection highlight (rank 4), scrollback navigation/orientation
(rank 6), IME input (rank 5) — numbers recorded in the plan's commit.
