# Configuration and diagnostics

- Status: implemented in the app configuration/diagnostics lane for Issue
  #59 (Milestone 2 acceptance). Code lives in
  `crates/noren-app/src/config.rs` and `crates/noren-app/src/diagnostics.rs`.

The PoC reads one optional TOML file. Every setting is optional, every
setting has a working default, and a missing or empty file behaves exactly
as it did before configuration existed. A file that exists but is malformed,
non-UTF-8, oversized, or out of range is a hard error, never a silent
fallback, because configuration is untrusted input under the
[threat model](security/threat-model.md).

## File location

- Standard path (macOS): `~/Library/Application Support/Noren/config.toml`
- Override: the `NOREN_CONFIG` environment variable names an explicit file
  path. When it is set, the file must exist and parse; absence is an error
  rather than a silent default.
- With `HOME` unset the standard path is unresolvable and defaults apply.

## Format

Strict TOML ([`toml_edit`](https://github.com/toml-rs/toml), exact version
pinned in the root `Cargo.toml`). The schema is closed: unknown tables,
unknown keys, wrong value types, and duplicate keys are errors so a typo or
a hostile value can never masquerade as a working setting. The file is read
with a hard cap of 64 KiB (streamed, so even a pathological target cannot
exhaust memory), must be valid UTF-8, and must resolve to a regular file;
symlinks are followed like any user-owned file under those same bounds.

## Keys

### `[font]`

| Key | Type | Default | Accepted range | Meaning |
| --- | --- | --- | --- | --- |
| `cell_width` | integer | `10` (`POC_CELL_WIDTH`) | `POC_CELL_WIDTH..=1024` | Cell width in physical pixels used to convert window size into the terminal grid. |
| `cell_height` | integer | `20` (`POC_CELL_HEIGHT`) | `POC_CELL_HEIGHT..=1024` | Cell height in physical pixels used to convert window size into the terminal grid. |

The lower bound is the renderer's built-in cell constant (`MAX_CELL_EDGE`,
1024, is the upper bound). A cell smaller than the renderer constant would
make the terminal grid larger than the grid the renderer can draw, silently
hiding terminal content, so values below the floor are rejected rather than
accepted-and-truncated. Zero is rejected too because grid division by zero
would fault. The derived grid is still clamped to the renderer's drawable
grid (`MAX_RENDER_ROWS` by `MAX_RENDER_COLS`, 60 by 160), which is far inside
the terminal foundation's `MAX_SCREEN_CELLS` bound, so no configuration value
can push the grid past that ceiling.

### `[sidebar]`

Controls how many cell columns the sidebar occupies. This is an optional
workspace preference: the default 16-column row already shows a distinct
lifecycle marker in its final cell, without configuration.

| Key | Type | Default | Accepted range | Meaning |
| --- | --- | --- | --- | --- |
| `columns` | integer | `16` | `8..=159` | Sidebar width in terminal cell columns. |

Session identity text uses the available cells and truncates with `...` when
necessary; the lifecycle cell is always reserved and never truncated. The
upper bound is one less than `MAX_RENDER_COLS`, so even the widest configured
sidebar leaves one drawable terminal column. Unknown keys, wrong types, and
out-of-range widths are hard errors under the same closed-schema rules as the
other tables.

### `[keys]`

Configurable key chords for workspace chrome and the terminal scrollback
viewport. Every key is optional; an absent `[keys]` table keeps the
compiled-in defaults. Existing workspace defaults remain unchanged, and the
scrollback actions follow conventional Shift+PageUp/Shift+PageDown defaults.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `palette_open` | string | `"super+p"` | Chord that opens the command palette while it is closed. |
| `session_create` | string | `"c"` | Palette command dispatching `session.create` (New Session); also the direct recovery action while the workspace is empty. |
| `session_select` | string | `"s"` | Palette command dispatching `session.select` (Switch Session). |
| `session_close` | string | `"x"` | Palette command dispatching `session.close` (Close Session). |
| `sidebar_focus` | string | `"f"` | Palette command dispatching `sidebar.focus` (Focus Sidebar). |
| `scroll_page_up` | string | `"shift+pageup"` | Scroll the primary terminal viewport one page toward older retained history. |
| `scroll_page_down` | string | `"shift+pagedown"` | Scroll the primary terminal viewport one page toward the live tail. |

A chord is zero or more modifiers followed by exactly one key, joined with
`+`: the modifiers are `super`, `ctrl`, `alt`, and `shift` (each at most
once, case-insensitive), and the key is a single character (case-folded) or
a named key — `enter`, `tab`, `backspace`, `escape`, `space`, `up`, `down`,
`left`, `right`, `home`, `end`, `pageup`, `pagedown`, `insert`, `delete`,
`f1` through `f24`. Examples: `super+p`, `ctrl+shift+t`, `f2`.

Rejections follow the same hard-error discipline as the rest of the schema —
never a silent fallback and never a silently dead binding:

- an unparseable chord (empty text, an empty `+` part, a non-modifier before
  the key, an unknown key name, a repeated modifier, a control or whitespace
  character, a function key outside F1–F24) is an error naming the key and
  the offending value;
- an unknown action name is an unknown-key error;
- two actions bound to the same chord are an error, including a configured
  value that collides with an action the table left at its default;
- `palette_open` must stay claimable: a chord that collides with the pinned
  Zellij v0.44.3 default corpus or with the frozen `Super+Escape` exit
  leader is an error, because Noren could never honor it;
- the four palette command chords must not use `escape`, `enter`, `up`, or
  `down`, which the open palette always interprets as dismissal, confirm,
  and navigation;
- the two global scrollback chords cannot use the frozen `super+escape`
  exit-to-workspace leader. Other child/Zellij overlap is allowed when a user
  deliberately configures it.

The four command chords normally apply only while the palette is open — the
palette intercepts all keys then, so command chords never steal input from
Zellij or the terminal — which is why only `palette_open` is validated against
the Zellij corpus. There is one bounded recovery exception: when the entire
sidebar is empty and no session can receive input, the configured
`session_create` chord creates a session directly, exactly as the terminal-side
empty-state action says. Chords with modifiers dispatch on the exact modifier
set; inside the palette, a modifier-free character binding also matches the
character with any modifiers held, as the pre-configuration palette did.

The two scrollback chords apply only on the primary screen while the palette
is closed. They are consumed locally even at a scroll boundary, matching an
ordinary terminal's Shift+PageUp/Shift+PageDown ownership; on an alternate
screen they are forwarded unchanged to the running application. Rebinding
either action changes the live input match and the history indicator together.

The mouse wheel follows the terminal/application boundary rather than a user
toggle. When the running application has enabled DEC mouse tracking mode 9,
1000, 1002, or 1003, every terminal-side wheel click is forwarded to it
(including with Shift held); this preserves X10 applications and Zellij/vim
ownership. With no tracking mode, the wheel scrolls Noren's retained
primary-screen history locally. The sidebar is Noren-owned chrome and keeps its
own local wheel behavior.

At offset zero, new output follows the live tail automatically. After a user
deliberately scrolls above it, ordinary output preserves that non-zero offset
instead of yanking the view down. A `History -N` indicator is then the first
status-row segment and names the configured `scroll_page_down` chord followed
by `Latest`; reaching offset zero removes the indicator and restores the live
caret. Entering an alternate screen or an application mouse mode rejoins the
live surface immediately, and primary scrollback is never drawn into an
alternate screen.

### `[ui]`

Optional application chrome. The palette affordance is visible by default in
the permanent status row and is generated from the active `[keys]`
`palette_open` binding, so rebinding the command never leaves a stale shortcut
on screen. Configuration is not required for discoverability.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `show_palette_hint` | boolean | `true` | Show the configured palette opener followed by `Commands` in the terminal-side status row. Set to `false` to remove the persistent hint. |

The setting is an explicit opt-out: omitting `[ui]` keeps the hint visible. A
non-boolean value or an unknown key is a typed configuration error rather than
a silently ignored preference. Turning the persistent palette hint off does
not remove the actionable empty state: `No sessions` still shows the active
`session_create` chord in the terminal area, and pressing it creates a session
directly.

### `[theme]`

Selects the built-in colour palette. The table is optional; an absent
`[theme]` keeps `dark`, which is the palette the app shipped with before
themes existed except for the five ANSI entries issue #168 lifted to
clear WCAG AA.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `name` | string | `"dark"` | Which built-in palette drawing resolves colours through. |

Accepted names, matched exactly (case-sensitive, closed vocabulary):

- `dark` — the xterm default ANSI colours on the near-black background,
  with the five entries that failed WCAG AA minimally brightened to clear
  4.5:1 (issue #168); every theme-owned foreground keeps AA on the
  default background;
- `light` — an off-white background with darkened ANSI entries; every
  theme-owned foreground keeps WCAG AA (≥ 4.5:1) on the default
  background;
- `high-contrast` — pure white on pure black with pastel ANSI entries;
  every theme-owned foreground keeps WCAG AAA (≥ 7:1), strictly exceeding
  the other themes' measured minima.

Each built-in palette also owns its selection foreground and background.
Selection is visible with no setting: the pair is the palette's normal
foreground/background inverted, painted over exactly the cells that copy will
extract. Choosing `[theme] name` therefore controls the selection treatment as
well as ordinary terminal colours; it is not a renderer constant that requires
patching source. Measured from the RGBA8 channel values with the WCAG sRGB
transfer and luminance formula, selected text is 15.3887:1 in `dark`,
14.5632:1 in `light`, and 21.0000:1 in `high-contrast`, all above the 4.5:1 AA
floor for normal text.

Rejections follow the hard-error discipline: a non-string value is an
error naming the key, and an unknown name is a typed error naming the
offending value (clipped to 120 characters, like the `[keys]` chord echo:
a theme name is closed-vocabulary grammar text, never a credential) —
never a silent fallback to `dark`. Near-misses such as `Dark` or
`highcontrast` are rejected for the same reason.

The contrast contract: the checked set is every theme-owned foreground —
the default foreground and the sixteen ANSI entries — against the theme's
default background, plus the theme-owned selection pair above. The floor is
WCAG AA for normal text (4.5:1) because terminal glyphs are normal-size text;
`high-contrast` targets AAA (7:1). Program-paired colours (`SGR 31;41`) and
the shared 256-colour cube (`16..=255`) are outside any palette's control
(identical colours are 1:1 by definition; the cube's black corner fails on
every possible background) and are therefore not part of the checked set.

**Fixed with issue #168:** the default `dark` palette used to fail the
4.5:1 floor for five ANSI slots on its own background — black at 1.06:1,
blue at 2.10:1, red at 3.38:1, bright blue at 4.16:1, and magenta at
4.21:1. Issue #168 made the deliberate decision to move exactly those
five entries the minimum distance that clears the floor (black
`[0,0,0]`→`[121,121,121]`, red `[205,0,0]`→`[243,0,0]`, blue
`[0,0,238]`→`[0,113,255]`, magenta `[205,0,205]`→`[213,0,213]`, bright
blue `[92,92,255]`→`[100,100,255]`), preserving ANSI slot semantics; the
default's measured minimum is now 4.50:1 (magenta) and `high-contrast`
(7.84:1) remains the choice for AAA. The minima are pinned by tests
(`crates/noren-app/tests/theme.rs`, plus a pixel-level pin in
`tests/frame_oracle.rs`), so any further palette change — fix or
regression — is a visible failure. Residual caveat: ANSI black and
bright black now sit close together (`[121,121,121]` vs
`[127,127,127]`), and the contract still excludes the shared 256-colour
cube, truecolor, and program-paired colours — those can draw below the
floor under any palette.

### `[cursor]`

Cursor appearance (issues #197/#200). The table is optional and the caret
**ships drawn**: an absent `[cursor]` renders a focused inverse-video block,
using the cursor cell's resolved foreground for the block and its resolved
background for the glyph. On an unstyled cell this is the active theme's
contrast-verified cursor/default-foreground pair. On an SGR-painted cell it
is the pair that actually occupies that cell, not a fixed colour measured
only on the theme background. If that inverse foreground misses 4.5:1, the
renderer chooses whichever of black/white has greater contrast; this keeps
both the caret and the glyph inside a block at or above 4.5:1 on every sRGB
background. Configuration changes how the caret looks, never whether it
exists; visibility belongs to the program through DECTCEM (`CSI ?25l`/`?25h`),
not to a preference that could quietly reproduce the every-keystroke-blind
defect.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `shape` | string | `"block"` | `block` (filled, glyph inverted beneath), `bar` (left stroke), or `underline` (bottom stroke). |
| `color` | string | inverse cell foreground | One preferred `#rrggbb` cursor colour. |

Accepted shapes, matched exactly (case-sensitive, closed vocabulary):
`block`, `bar`, `underline`. An unfocused window always draws the caret as
a hollow outline of the block footprint regardless of shape — the classic
signal that the window no longer receives keys — because the shape setting
is a *focused* typing aid.

`color` is a preference, not permission to draw an invisible cursor. The
configured colour is used exactly when it reaches 4.5:1 against the actual
cursor-cell background. If it does not, the renderer falls back to a readable
inverse cell foreground, then to contrast-maximising black/white if the cell
pair itself is below 4.5:1. A usable override therefore remains fully under
user control, while an override equal to an SGR background cannot silently
erase the caret.

Rejections follow the hard-error discipline: an unknown shape is a typed
error naming the offending value (clipped to 120 characters, like the
`[theme]` name echo: shape names are closed-vocabulary grammar text, never
a credential); a non-string value, an unknown key (including `blink`,
deliberately not offered), or a colour that is not one `#rrggbb` value is
an error naming the key — never a silent fallback, and never a guessed
colour.

Blink is deliberately absent: a blinking caret forces timer-driven
repaints roughly twice a second even while idle, and this renderer rebuilds
the full vertex list every frame. That is a CPU/battery decision, not a
visual one, and it is not taken silently by a default.

### `[[agents]]`

Configured AI-agent entries: each `[[agents]]` table names one agent with a
display `name` and the `command` to launch when its sidebar row is selected.
An optional `args` array supplies argv words after the program. Entries
appear in file order, after the worktree and SSH rows, capped at 24 rows
(the omitted count is reported on the status row, exactly like the SSH host
and worktree caps).

| Key | Type | Required | Accepted range | Meaning |
| --- | --- | --- | --- | --- |
| `name` | string | yes | 1..=1024 bytes | Display name on the sidebar row. |
| `command` | string | yes | 1..=1024 bytes, absolute path | Program launched in a PTY when the row is selected. |
| `args` | array of strings | no | each element at most 1024 bytes | argv words after the program. |

The launch is **argv, never a shell**: `command` becomes `argv[0]` and each
`args` element becomes exactly one argv word, so a value containing `;`,
`$(...)`, or a backtick is literal data to the agent program — no `sh -c`
ever interprets it. The command must be an absolute path with a leading `/`;
`PATH` lookup is deliberately not performed, so a writable `PATH` entry
cannot substitute a different binary (the same reasoning that fixes the SSH
client at `/usr/bin/ssh`).

A command that is missing or not executable is a visible failure when the
row is selected — the configured row reports the launch failure, the
created session row shows `failed`, and the status row carries a fixed
failure line — never a hang and never a silent no-op.

Rejections follow the same hard-error discipline as the rest of the schema:
an entry missing `name` or `command`, a wrong-typed field, a non-array
`args`, a non-string `args` element, a relative `command`, an empty or
oversized field, or an unknown key inside an entry is an error naming the
offending key. The `agents` table must be spelled as an array of tables
(`[[agents]]`); `agents = [...]` with inline tables is rejected. Error
messages never echo the field values.

### `[[projects]]`

Configured project entries: each `[[projects]]` table names one project
with a display `name` and the absolute `root` directory a session starts in
when the row is selected. Entries appear in file order, between the session
rows and the worktree rows, capped at 24 rows (the omitted count is reported
on the status row, exactly like the other bounded lists).

Projects are **configured, not discovered**. A git worktree has an
authoritative machine-readable source (`git worktree list --porcelain`), so
worktrees are discovered; a project has no such source — any directory can
be one — and scanning a directory tree for `.git` folders would be slow,
unbounded, and would guess at the user's intent. A `[[projects]]` entry is
the user telling Noren what counts.

| Key | Type | Required | Accepted range | Meaning |
| --- | --- | --- | --- | --- |
| `name` | string | yes | 1..=1024 bytes | Display name on the sidebar row. |
| `root` | string | yes | 1..=1024 bytes, absolute path | Directory the session's shell starts in when the row is selected. |

The `root` must be an absolute path with a leading `/`: neither `~`
expansion nor resolution against the launch directory is performed, so the
configured text and the directory the session starts in can never silently
diverge. Existence is deliberately not checked at load time — a configured
root whose directory is gone is a runtime fact, refused visibly when the
row is selected (exactly like a registered-but-deleted worktree), not a
load-time guess.

A project row is visually distinguishable from a worktree row: it carries
the fixed eight-character state prefix (`PRJ-OFF` idle, `PRJ-ERR` after a
refused launch) like the SSH and agent rows, while a worktree row shows its
checkout's final path component and branch.

Rejections follow the same hard-error discipline: an entry missing `name`
or `root`, a wrong-typed field, a relative or tilde-relative `root`, an
empty or oversized field, or an unknown key inside an entry is an error
naming the offending key. The table must be spelled as an array of tables
(`[[projects]]`); `projects = [...]` with inline tables is rejected. Error
messages never echo the field values — a root can embed a username or a
private directory name, so neither `Debug` output nor any error `Display`
prints it.

### Rejected keys, by design

- **`[terminal]` / `scrollback_lines`.** Scrollback retention is enforced
  inside the terminal foundation at its fixed hard cap `MAX_SCROLLBACK_LINES`
  (10,000), with no API at this milestone to lower it. Accepting a scrollback
  key the app cannot honor would be a silent no-op, so the `[terminal]` table
  is rejected as an unknown key until the foundation exposes a configurable
  cap. Configuration can therefore never raise the cap.
- **Shell selection.** There is no shell key and none may be added (see
  below). The spawn program stays `/bin/zsh`.

### Example

```toml
[font]
cell_width = 12
cell_height = 24

[sidebar]
columns = 24

[keys]
palette_open = "super+k"
session_create = "ctrl+shift+t"

[ui]
show_palette_hint = false

[theme]
name = "light"

[[projects]]
name = "noren"
root = "/Users/dev/noren"

[[projects]]
name = "zellij"
root = "/Users/dev/tooling/zellij"

[[agents]]
name = "claude"
command = "/usr/local/bin/claude"
args = ["--login"]

[[agents]]
name = "aider"
command = "/opt/homebrew/bin/aider"
```

## Error behavior

- Missing standard file, or empty file: all defaults; behavior identical to
  a pre-configuration build.
- Existing file with invalid TOML, invalid UTF-8, unknown keys, wrong types,
  out-of-range values, or a size above 64 KiB: the app prints a clear
  one-line error to standard error and exits without opening a window.
- Error messages never embed file contents: hostile key names and parser
  details are clipped to a bounded length, and file values are not echoed.
  The one deliberate exception is `[keys]` chord text (issue #150): a
  chord is keybinding grammar, never a credential, and an error that
  cannot show the offending binding — which value fails to parse, which
  one collides with a pinned Zellij default — is not actionable. Chord
  text is clipped to 120 characters like key names.

## What configuration deliberately cannot do

- **No shell selection.** The threat model (TM-01) fixes the shell spawn at
  `/bin/zsh` with structured argv and accepts no configured additions, so
  there is no shell key and none may be added. The `[[agents]]` `command`
  and the `[[projects]]` `root` are different, explicit surfaces: a program
  the user asks Noren to launch and a directory the user asks Noren to open,
  validated (absolute, bounded) and never echoed in errors or `Debug`
  output.
- **No credentials.** No key names a credential, key, or other sensitive
  value; the only path-shaped keys are the agent `command` and the project
  `root` described above.
- **No raised ceilings.** `MAX_SCROLLBACK_LINES` and `MAX_SCREEN_CELLS` stay
  hard caps. Configuration has no key that touches the scrollback ceiling
  (the rejected `[terminal]` table), and every grid derived from cell sizes
  remains clamped far inside `MAX_SCREEN_CELLS`.

## Diagnostics

Press **Super+D** (Command+D) to toggle a bounded status overlay without a
debugger. Each activation emits exactly one single-line report — to the
window title and to standard error — covering:

- grid geometry (`rows x cols`),
- active terminal modes (alternate screen, application cursor keys,
  application keypad),
- scrollback length against the terminal foundation's hard cap,
- PTY child status (`running`, `exited(code=N)`, `exited`, `not launched`).

Super chords are dropped by the key encoder anyway, so the chord consumes no
terminal input. Pressing the chord again clears the overlay.

### Privacy rule

Diagnostics report counters and flags only. They never include PTY output
bytes, screen cell text, scrollback contents, terminal replies, or keystrokes,
because that content is user data and may contain secrets. There is no opt-in
for content: `crates/noren-app/src/diagnostics.rs` offers no API that accepts
or returns screen text, and the module tests prove a secret fed through the
terminal never appears in a report.
