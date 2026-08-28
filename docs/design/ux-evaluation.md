# Noren UI/UX evaluation — independent position B

Date: 2026-08-29 (JST)  
Scope: release binary at `7654bb2` plus the production render stream at the
default 900×600 window and 10×20 cell metrics.

## Verdict

Noren is recognizable as a terminal, but its Noren-specific value is not
discoverable from the initial surface. The shell remains the only obvious
action. The command palette that exposes workspace operations has no visible
opener, and the state after closing the last session says only `No sessions`.
That is a dead end for anyone who has not read the documentation.

The larger problem is not decorative polish. Three missing feedback mechanisms
violate ordinary terminal expectations: there is no cursor, a real selection
does not change a pixel, and retained scrollback has no viewport/position
signal. The sidebar also loses information it claims to communicate: at 16
columns, a session's `starting`, `running`, `exited`, or `failed` word is outside
the rendered row. The five row kinds are therefore only partly distinguishable.

The three themes are partly coherent. Their palettes are readable in the
captured render stream, including the light theme, but each is applied to one
flat undivided plane. A palette substitution cannot supply hierarchy, selection
feedback, or semantic state that the layout never draws.

## Evidence boundary

I built and ran the requested artifact:

```text
cargo build --release
Finished `release` profile [optimized] target(s) in 1m 02s

env -u HOME ./target/release/noren-app
process entered and remained in the application event loop
```

`HOME` was unset only for the isolated first-launch process so the existing
user session registry was neither read nor changed. The release executable and
the copy used in a disposable macOS bundle had identical SHA-256 hashes.

There are important limits to what this environment could show:

- macOS accessibility refused the disposable unbundled-app target;
- the requested native screenshot failed with `could not create image from
  display`;
- the repository's official frame oracle reported `AdapterUnavailable`;
- a second adapter request made with a real winit window surface also found no
  Metal adapter.

The native title bar, actual Metal presentation/color management, live pointer
behavior, live SSH/agent activation, keyboard quit, and an on-machine
Terminal.app/iTerm side-by-side are therefore **unverified**. In particular, I
do not repeat the earlier review's `Noren PoC` title observation as if I saw it.

To inspect layout rather than substitute a source-code claim, I captured the
production renderer's emitted, color-resolved rectangle stream with a temporary
software rasterizer. Noren's primitives are integer-aligned opaque rectangles,
so this preserves glyph shape, spacing, clipping, draw order, and the selected
theme's 8-bit values. It does **not** turn the missing GPU frame into a pass;
all judgments about native presentation remain qualified below. The temporary
capture code and bundle were removed before committing.

## 1. First launch and empty state

The initial render-stream capture contains one selected session row and the
terminal surface. At 16 columns the row is only:

```text
> session-1 loca
```

The terminal starts in the immediately adjacent cell. There is no gutter,
divider, panel background, heading, footer, or status-area hint. A prompt on
the first terminal row consequently reads as though it were concatenated to
the sidebar row.

What a new user can infer without documentation is limited:

- this is terminal-like and accepts shell input;
- `>` probably marks the current row;
- there is a session named `session-1`.

Nothing on the closed surface says that Command/Super+P opens a palette, or
that creating, switching, and closing sessions exist. Once the palette is
already open, its four commands and their one-letter keys are readable, but
that does not help a user discover the opener.

After the last session is closed, the captured surface contains only:

```text
No sessions
```

There is no `Create session` action and no `Press ⌘P` recovery instruction.
The core interaction is therefore not discoverable without documentation.

## 2. Sidebar

### Five row kinds

The following are the complete visible 16-column slices from the capture; text
to the right is discarded before the terminal starts:

| Kind | Visible row | What survives |
| --- | --- | --- |
| Session | `> session-12 loc` | generated identity and three letters of kind; lifecycle is gone |
| Project | `  PRJ-OFF lon...` | type/state and three identity characters |
| Worktree | `  worktree-ux-wi` | identity prefix only; no type token and no ellipsis |
| SSH host | `  SSH-ON  pro...` | type/state and three identity characters |
| Agent | `  AGT-ERR cod...` | type/state and three identity characters |

Projects, SSH hosts, and agents share an aligned eight-character
`TYPE-STATE` prefix and are distinguishable from one another. The actual agent
prefix in the captured row is `AGT-ERR`, not `AG-ERR`. The prefixes are
decodable, but `OFF`, `ON`, and `ERR` are terse status codes rather than a
visual state system. `SSH-ON` also collapses connecting and connected.

Sessions and worktrees have no corresponding type prefix. They are inferred
from naming conventions: `session-N` for sessions and a checkout-like name for
worktrees. That is not robust at a glance.

### Width, truncation, and hierarchy

Sixteen columns is not enough for identity plus state:

- a two-digit session leaves three columns for its kind and zero for lifecycle;
- project, SSH, and agent identities get three characters plus `...`;
- worktrees are hard-clipped without an ellipsis, so clipping and a complete
  short name look the same;
- secondary detail such as branch, `connected`, `launch failed`, or `running`
  is outside the visible slice.

All five kinds use the same foreground, cell size, row density, left edge, and
background. There are no section headings, groups, icons, indentation levels,
badges, or separators. The only selection treatment is `>`. The sidebar is a
flat list of strings, and the lack of a divider lets its last cell visually
touch terminal content. Overall result: **partly distinguishable**, not
distinguishable at a scan.

## 3. Legibility, measured

I rasterized all 62 ASCII letters and digits at the shipped 10×20 cell size and
counted non-background pixels per cell:

```text
mean ink       27.5% of the full cell
median ink     28.0%
range          18.0%–40.0%
bitmap envelope 10×14 px = 70% of the cell
edge pixels    0 intermediate-color pixels
```

Each set bit in the 5×7 source becomes a hard 2×2 block. The bitmap can span
the full 10-pixel cell width, leaving no guaranteed horizontal side bearing,
while the 14-pixel-tall envelope has three blank pixels above and below. This
combination makes the page simultaneously sparse in total ink and blocky at
character edges.

Short ASCII labels are readable at the default 900×600 size. Sustained reading
is not comfortable: curves staircase, diagonal letters are coarse, dense rows
run together, and the absence of antialiasing is especially obvious on the
light background. A normal Terminal.app or iTerm user expects an antialiased,
hinted monospace face with usable side bearings and point-size control. I could
not capture those apps side by side here, so the direct visual comparison is
**unverified**; Noren's measured hard-edge raster nevertheless falls below
that ordinary expectation.

## 4. State communication

The session row itself does not distinguish any lifecycle state because the
state word begins after column 16. State is recoverable only when a separate
status row happens to be present.

| Session state | Stable, explicit visible signal | Result |
| --- | --- | --- |
| Starting | lifecycle clipped; live duration could not be exercised | No / unverified timing |
| Running | lifecycle clipped; activity can only be inferred from terminal output | No |
| Exited | status row says the shell exited | Yes |
| Failed | status row says the PTY/launch failed; typed rows can carry `ERR` | Yes |

That is **2/4** states distinguishable without inference. Starting and running
look like the same selected `session-N` row. The missing cursor makes the
"running terminal" inference weaker than it would be in an ordinary terminal.

The rendered error variants are clearer for typed rows: an SSH failure can
carry `SSH-ERR`, and a missing agent command can carry `AGT-ERR`, accompanied
by a failure status line. Those are visible but still plain text in the same
style as every other row. Conversely, connecting and connected both use
`SSH-ON`; the row does not distinguish them. Live clicks that produce these
states were **unverified** because the app could not be driven through the
available UI channel.

## 5. Themes

The render-stream captures show three genuinely different palettes:

- `dark` uses a near-black ground and pale green default text. It is visually
  coherent as a retro terminal, though that aesthetic reinforces the bitmap
  font's coarseness.
- `light` uses an off-white ground and dark gray default text. It is usable for
  decoding short content, not merely invisible compliance, but the hard pixel
  edges are harsher and the large unstructured blank area is stark.
- `high-contrast` is pure black with white default text and brighter ANSI
  colors. It is coherent for its stated purpose but visually severe.

The themes do not establish UI roles. Sidebar text, selection marker, status
copy, lifecycle text, and terminal text all occupy the same plane. Errors are
not made more scannable, the selected row has no background, and there is no
sidebar/terminal separation. Theme coherence is therefore **partly**, not
fully: the color sets hang together, but the interface beneath them has no
semantic color or surface hierarchy. Exact on-screen color appearance through
the unavailable Metal surface is **unverified**.

## 6. Missing terminal feedback

These are expectation violations, not requests for ornament.

### Cursor — absent

I rendered `abc`, moved the terminal cursor two columns left without changing
the cells, and captured again. The two frames were byte-identical. A user
cannot locate the insertion point, confirm focus, distinguish insert from
command movement, or safely edit a long shell command.

### Selection highlight — absent

A real character selection extracted `ab`, but the frames before and after
selection were byte-identical. Copying is therefore blind: users cannot verify
which cells will reach the clipboard, especially across wrapped lines.

### Scrollback viewport indicator — absent

I compared identical visible cells with zero retained history and with one
retained scrollback row. The frames were byte-identical. There is no scrollbar,
thumb, position mark, `n/m` counter, or other indication of whether the view is
at the live tail or how much history exists. This removes orientation during
log reading and makes returning to current output uncertain. Live scrollback
navigation itself was **unverified**.

## Findings by severity

Severity is based on task failure and miscommunication, not personal taste:
**critical** blocks discovery or a baseline terminal interaction; **high**
causes frequent ambiguity or unreliable use; **medium** adds recurring friction
without blocking the task.

1. **Critical — The primary Noren interaction is undiscoverable.** The closed
   surface contains no Command/Super+P hint, and `No sessions` gives no recovery
   action. This hides the product's workspace value and can strand a new user.
   This is a product-usability failure, not a generic terminal convention.
2. **Critical — No visible cursor violates terminal editing expectations.**
   Cursor movement produces an identical frame. Every interactive shell edit
   is less trustworthy, not merely less attractive.
3. **High — Selection is functional but visually unverifiable.** A selection
   can contain text while changing no pixels. Users can copy the wrong range
   without feedback; this violates a terminal expectation.
4. **High — The 16-column sidebar removes identity and lifecycle information.**
   Session state is always clipped, typed identities shrink to three
   characters, and worktree truncation is unmarked. This is information loss,
   not a preference for a wider panel.
5. **High — Only two of four session lifecycle states are explicit.** Exited
   and failed have status copy; starting and running require inference. SSH
   additionally merges connecting and connected under `SSH-ON`.
6. **High — Default text is readable only in the narrow sense.** Alphanumeric
   ink averages 27.5% of a cell and uses hard 2×2 blocks with no edge pixels.
   It is adequate for short ASCII labels but below the comfort expected for
   sustained terminal work.
7. **High — Five row kinds are presented as one flat text stream.** Prefixes
   distinguish three typed rows, while sessions/worktrees rely on names; no
   divider or hierarchy separates rows from one another or from terminal
   content.
8. **Medium — Themes are palette-complete but interface-incomplete.** Dark,
   light, and high-contrast are individually readable in the render stream,
   but none adds semantic state or surface hierarchy. Light is usable, yet
   visually stark and especially unforgiving of the bitmap font.
9. **High — Retained scrollback has no viewport signal.** Identical visible
   content renders identically with and without history, removing position and
   live-tail orientation expected during ordinary terminal use.
10. **Medium — The open palette lacks modal separation.** Its commands are
    readable once opened, but it replaces the same 16-column text strip with no
    title, boundary, or backdrop and visually touches terminal content. This is
    recurring friction; richer animation or decoration would only be a
    nice-to-have.

## Ranked proposals: smallest changes with the largest effect

1. **Add an always-visible command-palette affordance and make the empty state
   actionable.** Show the configured opener—`⌘P Commands` by default—and, when
   empty, `No sessions — press ⌘P, then C to create one`. Rank 1 because a few
   cells of accurate copy unlock every existing Noren action for every new
   user; nothing else matters if the product cannot be discovered.
2. **Render the terminal cursor.** Draw a theme-aware block/bar/underline at
   the snapshot cursor, with clear focused/unfocused behavior. Rank 2 because
   it repairs the most frequent baseline terminal feedback failure with a
   small renderer primitive.
3. **Make sidebar geometry preserve type, identity, and state.** Give the panel
   an adaptive minimum width, a one-cell divider/gutter, and stable type/state
   columns; apply ellipsis only to identity and expose full detail on selection
   or in the status area. Rank 3 because one layout change fixes row-kind
   scanning, lifecycle clipping, long-name ambiguity, and terminal
   concatenation together.
4. **Draw selection and scrollback orientation.** Add a theme-owned selection
   background and a compact scroll position/live-tail indicator. Rank 4 because
   both restore confidence in existing terminal operations; they follow the
   cursor because selection and history are less constant than typing.
5. **Replace the 5×7 bitmap path with an antialiased monospace text stack.**
   Start with a normal terminal default size and preserve configurable cell
   metrics. Rank 5 because the comfort and character-coverage gain is large,
   but implementation scope is materially larger than the preceding feedback
   and layout fixes.
6. **Add semantic styling only after information survives layout.** Use
   theme-owned selected, error, starting, connected, and offline treatments,
   plus minimal group labels if needed. Rank 6 because color/icons cannot fix
   state text that is currently clipped; this is the first ranked item that is
   partly enhancement rather than baseline repair.

The deliberately omitted "improvements" are animations, decorative icons,
rounded panels, and theme proliferation. They would be nicer, but none fixes a
task failure observed here.

## Issue filing status

Live duplicate searches found no matching open issues. Creation was attempted,
but every available write path was unavailable: the GitHub connector requires
an approval mode this environment cannot grant, the GitHub CLI token is
invalid, and the browser extension is not installed in the connected Chrome
profile. No issue number is claimed.

These five titles and acceptance directions are ready to file from the ranked
proposals above:

1. `Make the command palette discoverable and the empty state actionable`
2. `Render a visible terminal cursor`
3. `Preserve sidebar type, identity, and lifecycle state without 16-column clipping`
4. `Render selection highlight and scrollback orientation feedback`
5. `Replace the 5x7 bitmap font with a terminal-grade monospace text path`
