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

### `[keys]`

Configurable key chords for workspace chrome. Every key is optional; an
absent `[keys]` table keeps the compiled-in defaults, which are exactly the
chords the app shipped with before configuration existed.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `palette_open` | string | `"super+p"` | Chord that opens the command palette while it is closed. |
| `session_create` | string | `"c"` | Palette command dispatching `session.create` (New Session). |
| `session_select` | string | `"s"` | Palette command dispatching `session.select` (Switch Session). |
| `session_close` | string | `"x"` | Palette command dispatching `session.close` (Close Session). |
| `sidebar_focus` | string | `"f"` | Palette command dispatching `sidebar.focus` (Focus Sidebar). |

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
  and navigation.

The four command chords apply only while the palette is open — the palette
intercepts all keys then, so command chords never steal input from Zellij or
the terminal — which is why only `palette_open` is validated against the
Zellij corpus. Chords with modifiers dispatch on the exact modifier set; a
modifier-free character binding also matches the character with any
modifiers held, as the pre-configuration palette did.

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

[keys]
palette_open = "super+k"
session_create = "ctrl+shift+t"
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

- **No shell selection.** The threat model (TM-01) fixes the spawn at
  `/bin/zsh` with structured argv and accepts no configured additions, so
  there is no shell key and none may be added.
- **No credentials.** No key names a credential, key, or sensitive path; the
  schema exposes no path keys at all.
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
