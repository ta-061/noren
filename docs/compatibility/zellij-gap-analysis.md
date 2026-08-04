# Zellij capability gap analysis

Snapshot: 2026-08-05, at `main` (`b3391cc`). This is a research lane deliverable
for the [Zellij compatibility matrix](zellij.md): an evidence-based list of what
Zellij v0.44.3 emits that the Noren terminal core does not yet interpret, ranked
by user-visible impact, with a proposed sequencing. It changes no code and
records no new compatibility claim; every `Planned` state in the parent matrix
stays `Planned`.

## Evidence discipline

- "Noren today" claims cite `file:line` in this tree. Several were additionally
  confirmed by feeding the exact byte sequences through
  `crates/noren-terminal/src/parser.rs` at this head (reproduction recipe at the
  end of this document).
- Upstream claims marked *verified* were read directly from the pinned Zellij
  v0.44.3 source (commit `55a2121`, as pinned in
  [zellij.md](zellij.md#versioned-upstream-fixture)) on 2026-08-05. Upstream
  claims taken from the parent matrix without re-fetching cite that matrix.
- Anything inferred from general multiplexer behavior, or not located in the
  pinned source, is labeled **inference** or **open** and must not be read as a
  Zellij contract.

## What Zellij sends on top of what Noren already handles

Before the gaps, the verified startup/teardown byte surface so the gap list is
not padded. Zellij's client writes, on attach: alternate screen
`CSI ?1049h`, a terminal-state reset blob
(`CSI ?1l`, `ESC =`, `CSI r`, `CSI ?1000l … ?1006l`, `CSI ?12l`), Kitty
keyboard push `CSI >1u`, theme subscription/query (`CSI ?2031h`, `CSI ?996n`),
and bracketed paste `CSI ?2004h`; on exit it writes `CSI <1u`, `CSI ?2031l`,
`CSI ?1049l`, SGR reset `CSI m`, `CSI ?25h`, and a final CUP
(*verified*: `zellij-client/src/lib.rs`, pinned commit). Noren already
interprets the load-bearing subset: mode 1049 and mode 1
(`crates/noren-terminal/src/parser.rs:310-330`), `ESC =` / `ESC >` keypad
modes (`parser.rs:139-146`), DECSTBM (`parser.rs:296-299`), CUP and the full
cursor/erase/edit set (`parser.rs:272-308`), and SGR reset (`state.rs:747-793`).
`CSI >1u`, `CSI <1u`, DECRQM `CSI ?2026$p`, and the Primary-DA barrier
`CSI c` are consumed without side effects (probe-verified), which is the
correct behavior for an unsupported terminal.

Zellij also launches against `TERM=xterm-256color` because that is what Noren
sets (`crates/noren-pty/src/lib.rs:34`), so 256-color/RGB SGR is Zellij's
expected path here, not an edge case.

## Ranked gap table

Blocking means: a default-keymap Zellij v0.44.3 session under Noren is
visibly broken or unusable without it. Cost is S (parser/state only),
M (new input path or renderer-visible attribute work), L (cell-model
architecture).

| # | Capability | What Zellij emits | Noren today | Impact if missing | Cost |
| --- | --- | --- | --- | --- | --- |
| 1 | UTF-8 output and character width | Every frame: pane frames are rounded box-drawing characters and the status bar uses non-ASCII glyphs (theme and frame config are RGB/Unicode-based, *verified*: `zellij-utils/src/input/theme.rs`, pinned); all CJK/emoji pane content inside | ASCII-only: `Action::Print(u8)` for `0x20..=0x7e` and every other byte dropped (`parser.rs:129-130`); write path is `Cell::from_ascii` (`state.rs:63-69`, `state.rs:617-632`). A `unicode-width` seed exists but is not wired to the write path (`lib.rs:21,64-66`). Probe: U+2500 yields zero actions | Zellij's visible chrome disappears or misaligns; CJK and emoji content vanishes; column addressing drifts once any wide cell exists. This is the single largest visible breakage | L |
| 2 | Mouse input generation and mouse-mode tracking | Client writes `CSI ?1000h CSI ?1002h CSI ?1003h CSI ?1015h CSI ?1006h` on enable, reverse on disable (*verified*: `ENABLE_MOUSE_SUPPORT`/`DISABLE_MOUSE_SUPPORT`, `zellij-client/src/os_input_output.rs`, pinned; also [zellij.md](zellij.md) mouse row); default config enables mouse mode | Not tracked: only private modes 1 and 1049 exist (`parser.rs:314-317`); probe: all five DECSETs yield zero actions. No pointer events reach the PTY: the window handler covers close/resize/modifiers/keyboard/IME only (`crates/noren-app/src/main.rs:317-327`); no mouse encoder exists anywhere in `crates/` | Pointer-first interaction is dead: no pane click/focus, no drag-resize, no wheel scrolling of normal-screen children | M |
| 3 | Correct consumption of mouse reports (`CSI <`, X10 `CSI M`) | SGR-1006 reports `CSI < Cb;Cx;Cy M/m` and legacy X10 `CSI M cb cx cy` flow wherever an enabled mouse mode routes them ([inner encoder](zellij.md) cited in the parent matrix) | Buggy: `<` is not recognized as a CSI private marker (only `?` and `>` are, `parser.rs:229-236`; `<` falls into the catch-all at `parser.rs:249`). Probe results: `CSI <0;5;7m` (left release) parses as SGR `[0,5,7]` and **resets all attributes** (`state.rs:752`); `CSI <2;5;7m` sets reverse video; legacy `CSI M` + 3 data bytes parses as `DeleteLines(1)` and prints the coordinate bytes as text (`parser.rs:241-248`, `parser.rs:295`, `state.rs:732-745`) | Once any mouse bytes appear in the output stream, the screen corrupts: attribute resets/flicker, spurious line deletes, garbage glyphs. This is the blocking defect behind any #2 work | S |
| 4 | 256-color and truecolor SGR | Zellij chrome is styled from themes whose colors are RGB hex values (*verified*: `HexColor`/`Theme`, `zellij-utils/src/input/theme.rs`, pinned); TERM is `xterm-256color`, so indexed (`38;5;N`) and RGB (`38;2;R;G;B`) forms are emitted (exact byte form is **inference**, pending the `Z-PROTO` byte trace owned by the parent matrix) | Consumed but discarded: 38/48/58 swallow their arguments as one unsupported group (`state.rs:785-788`, `state.rs:844-850`); `Color` models only Default and 16 ANSI colors (`crates/noren-terminal/src/attributes.rs:58-63`) | Status bar, tab bar, and pane frames lose all theme colors and render with default fg/bg; light/dark themes are indistinguishable | M |
| 5 | Bracketed paste input path (mode 2004) | Client enables 2004 on the host (*verified*: `ENABLE_BRACKETED_PASTE`, `zellij-client/src/lib.rs`, pinned) and wraps pastes in begin/end markers, stripping them for inner panes that did not enable 2004 ([zellij.md](zellij.md) bracketed-paste row) | No paste path at all: multi-character text input is rejected (`main.rs:397-404`), mode 2004 is not tracked (`parser.rs:314-317`; probe: zero actions), no `WindowEvent::Paste` handling | Nothing can be pasted into Zellij or its panes; later, adding paste without 2004 tracking would also fail apps that rely on markers | M |
| 6 | Synchronized output (mode 2026) | Startup DECRQM `CSI ?2026$p`; on a 1/2/3 reply Zellij wraps every render in `CSI ?2026h … CSI ?2026l` (DCS variants exist for the alacritty heuristic) (*verified*: `build_startup_query_string`, `zellij-client/src/stdin_handler.rs`; `SyncOutput::start_seq/end_seq` and `SYNC_RE`, `zellij-client/src/stdin_ansi_parser.rs`, pinned). The query is fire-and-forget: "no deadline, no cache, no loading gate" | No reply is sent (no reply wiring at app level; `PtySession::send_reply` exists unused, `crates/noren-pty/src/lib.rs:348-351`); `CSI ?2026h/l` is ignored (`parser.rs:314-317`, probe-verified) | Zellij falls back safely (no hang) but repaints tear/flicker during full-screen redraws, which is most Zellij renders | S+M |
| 7 | Cursor visibility and shape (modes 25, 12; DECSCUSR) | Teardown emits `CSI ?25h`; the startup reset blob includes `CSI ?12l` (*verified*, lib.rs pinned); apps under Zellij toggle 25 and set DECSCUSR `CSI Ps SP q` | Ignored: neither 25 nor 12 is tracked (`parser.rs:314-317`); `CSI 3 SP q` yields no action (probe; SP sets the CSI ignore flag at `parser.rs:237-238`); snapshots carry no cursor-visibility/shape state (`state.rs:864-934`) | Apps that hide the cursor (nvim, fzf) show a stale second cursor; cursor-shape changes (block/beam) are cosmetic-only losses | S |
| 8 | Focus reporting (mode 1004) | Zellij tracks inner 1004 state and emits `CSI I`/`CSI O` to panes on focus change ([zellij.md](zellij.md) focus row, with pinned grid.rs citations) | Ignored: 1004 not tracked (`parser.rs:314-317`); `CSI I`/`CSI O` produce no actions (probe); no window-focus events are forwarded to the PTY (`main.rs:317-327`) | Focus-aware apps (editor autoread, dimming, `FocusGained/Lost` mappings) misbehave silently | S+M |
| 9 | OSC payloads: window title (OSC 0/2) and clipboard write (OSC 52) | Without a configured copy command Zellij emits OSC 52 on copy ([zellij.md](zellij.md) OSC 52 row, with pinned citations). Whether Zellij itself sets the host title via OSC 0/2 at the pinned commit is **open** (no title write in the fetched client files); re-emission of inner apps' titles is **inference** | All OSC payloads are swallowed without parsing (`parser.rs:100-115`); window title stays the app's own status string (`main.rs:62,103`). Per the parent matrix, any OSC 52 work must never answer read queries | Copy from Zellij never reaches the system clipboard; window title never reflects the session. Both are policy-carrying (clipboard security boundary), not just parsing | M |
| 10 | Terminal query/reply surface (OSC 10/11/4, CSI 14/16 t, DA) | At startup Zellij queries text-area and cell pixel sizes (`CSI 14t`, `CSI 16t`), fg/bg colors (`OSC 10;?`, `OSC 11;?`), sync support (`CSI ?2026$p`), and all 256 palette registers (`OSC 4;N;?`); inner apps' whitelisted queries are forwarded with a Primary-DA barrier and a 500 ms timeout (*verified*: stdin_handler.rs, stdin_ansi_parser.rs, lib.rs pinned) | No replies are ever produced; all queries are consumed safely (probe: `CSI ?2026$p` and `CSI 14t`-class finals yield no actions; OSC swallowed) | Graceful: Zellij times out forwards and uses cached defaults, but color-adaptive apps and pixel-size logic degrade | M |

## Verified safe today (kept off the gap list)

- Alternate screen 1049 round-trips, DECSC/DECRC, and parked-primary restore
  (`state.rs:820-841`).
- SGR 16-color subset plus bold/underline/reverse (`state.rs:747-793`).
- `ESC =` / `ESC >`, DECCKM (mode 1), DECSTBM, CUP/HVP, CNL/CPL/VPA, erase and
  edit operations (`parser.rs:272-330`).
- Kitty push/pop (`CSI >1u`, `CSI <1u`), DECRQM (`CSI ?2026$p`), unsupported
  DECSET/DECRST values, and DCS-free OSC payloads: consumed without side
  effects or crashes (probe), with adversarial coverage for long/unterminated
  streams (`crates/noren-terminal/tests/adversarial.rs:351-383`).

## Proposed sequencing

Cheap correctness before architectural change, with the one exception that
UTF-8 design should not wait: row 1 dominates user-visible breakage, so its
design work should start alongside phase 1 even though it lands later.

1. **Parser hardening (S, row 3 + mode tracking).** Treat `CSI <` as a mouse
   private marker and consume X10 `CSI M` trailing bytes; add DECSET/DECRST
   state bits for 1000/1002/1003/1005/1006/1015, 2004, 1004, 2026, 25, 12;
   consume DCS (`ESC P … ESC \`) safely instead of printing its payload
   (probe: `ESC P=1s ESC` currently prints `=1s`). Parser and state only;
   directly testable; prerequisite for every input-side row.
2. **UTF-8 and width (L, row 1).** Decode UTF-8 in the parser, evolve the cell
   model to grapheme/width-aware cells (wide cells, wrap interaction), using
   the existing `cell_width` seam (`lib.rs:64-66`) and the UAX #11 caveats in
   [terminal-landscape](../research/terminal-landscape.md). Renderer and
   snapshot shapes follow. This is the expensive, load-bearing slice.
3. **Mouse input path (M, row 2).** Window pointer events -> mode-selected
   encoder (SGR 1006 first, since Zellij requests it) -> `send_input`.
   Requires phase 1 tracking and phase 2 for correct coordinates over wide
   cells.
4. **Paste and focus input (M, rows 5, 8).** Platform paste -> PTY with 2004
   marker policy; window focus -> `CSI I`/`CSI O` when 1004 is set. Shares
   the phase 1 mode bits.
5. **Color (M, row 4).** Indexed and RGB SGR into `CellAttributes` plus
   renderer palettes; restores Zellij chrome colors once the byte trace pins
   the exact forms.
6. **Sync output and cursor state (S+S, rows 6, 7).** Answer DECRQM 2026,
   buffer between BSU/ESU for atomic presentation; expose cursor
   visibility/shape in `TerminalSnapshot`.
7. **OSC policy work (M + security review, rows 9, 10).** Bounded OSC payload
   parsing for titles, an OSC 52 write path with the read-denial and size
   boundaries from the parent matrix, and the query/reply infrastructure on
   the existing `send_reply` seam. Do not start without the Claude Code
   security scope already assigned in [zellij.md](zellij.md).

## Reproducing the byte-level probes

Feed the sequences through `crates/noren-terminal/src/parser.rs` at this head
(compilable standalone) or through `TerminalState::feed_bytes` in a scratch
test; do not modify `crates/*/src/` to observe them. Key observations:

- `CSI <0;5;7m` -> `SelectGraphicRendition{[0,5,7]}` (attribute reset);
  `CSI <2;5;7m` -> reverse video set.
- `ESC [ M SP @ /` (X10) -> `DeleteLines(1)` plus three printed coordinate
  bytes.
- `CSI <32;5;7M`, `CSI I`, `CSI O`, `CSI 3 SP q`, `CSI ?2026h`,
  `CSI ?2004h`, `CSI ?1004h`, `CSI ?25l`, mouse DECSET group -> no actions.
- `ESC P=1s ESC Z` -> prints `=1s`, then `Z` (DCS payload leaks).
- OSC 52 and OSC 0 payloads -> consumed silently.

## Non-claims

This document executes nothing in `crates/*/src/`, adds no tests, and advances
no matrix row. A gap closing still requires the parent matrix's fixtures,
`Z-PROTO`/`Z-LOCAL` evidence, and reviewer sign-off. Zellij is named
nominatively as the compatibility target; no Zellij code or assets were copied
here (see the legal boundary in [zellij.md](zellij.md)).
