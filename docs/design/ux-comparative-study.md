# Noren comparative UI/UX study

**Study date:** 2026-08-29  
**Scope:** current Noren preview versus the UI conventions a user brings from
cmux, Terminal.app, iTerm2, WezTerm, Alacritty, kitty, Ghostty, Warp, tmux,
Zellij, and VS Code.

## Executive finding

Noren already has a defensible product idea: one compact outer rail can expose
live sessions and not-yet-launched projects, worktrees, SSH aliases, and agents,
while Zellij owns the layout inside the selected terminal. That is a clearer
resource inventory than cmux's workspace-first default and a cleaner ownership
boundary than reproducing another pane manager.

The current preview does not yet communicate that idea at the standard set by
the products it names as peers. A first-session user encounters a missing
cursor, coarse bitmap text, an undisclosed `Super+p` gateway, and labels that
can reveal only three characters plus `...` after a kind/state prefix. Those
are not matters of taste. They remove feedback that every ordinary terminal
workflow relies on, before the user has time to appreciate the resource model.

## Method and evidence boundary

The requested release command was exercised exactly:

```console
cargo build --release
./target/release/noren-app
```

The release build completed and the application entered its macOS event loop.
The host's UI-control boundary did not expose arbitrary application windows for
capture, so visual facts were cross-checked against the shipped draw path and
its frame-oracle coverage rather than inferred from model names. In this study,
**observed** means that the actual Noren release artifact was built and run;
Noren's pixel and interaction claims are additionally traceable to current
source. It does not mean that a screenshot was available. No competitor window
was accessible. Every competitor claim is therefore marked **documented, not
observed** and cites first-party documentation.

The two Discovery documents are inputs, not current baselines:

- [`cmux-parity.md`](../compatibility/cmux-parity.md) says that its cmux binary
  was not run and records feature evidence from before Noren had a runtime.
- [`terminal-landscape.md`](../research/terminal-landscape.md) is an
  architecture/feature survey, not an interaction or visual comparison.

**F-01 — The old matrices cannot establish current parity.** Since Noren now
draws a real sidebar and cmux now documents native AppKit sidebar rows,
workspace groups, alternate sidebar views, and richer agent/SSH behavior, a
feature checkmark from Discovery says nothing about present-day density,
feedback, or discoverability. This study supersedes those documents only for
UI/UX, not for their architectural claims.

### Evidence ledger

| Product | Evidence mode | Current first-party evidence used |
| --- | --- | --- |
| Noren | **Observed release runtime; renderer-verified** | Current [`main.rs`](../../crates/noren-app/src/main.rs), [`renderer.rs`](../../crates/noren-app/src/renderer.rs), [known limitations](../known-limitations.md), and [ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md) |
| cmux | **Documented, not observed** | [Concepts](https://cmux.com/docs/concepts), [workspace groups](https://cmux.com/docs/workspace-groups), [SSH](https://cmux.com/docs/ssh), [remote tmux](https://cmux.com/docs/remote-tmux), [CLI/sidebar metadata](https://cmux.com/docs/api), [notifications](https://cmux.com/docs/notifications), [agent teams](https://cmux.com/docs/agent-integrations/claude-code-teams), [configuration](https://cmux.com/docs/configuration), and [changelog](https://cmux.com/docs/changelog) |
| Terminal.app | **Documented, not observed** | Apple [profile overview](https://support.apple.com/guide/terminal/use-profiles-to-change-the-look-of-terminal-windows-trml107/mac) and [text settings](https://support.apple.com/guide/terminal/change-profiles-text-settings-trmltxt/mac) |
| iTerm2 | **Documented, not observed** | [General usage](https://iterm2.com/documentation-general-usage.html), [profiles](https://iterm2.com/documentation-preferences-profiles-general.html), [text](https://iterm2.com/documentation-preferences-profiles-text.html), and [colors](https://iterm2.com/documentation-preferences-profiles-colors.html) |
| WezTerm | **Documented, not observed** | [Launcher](https://wezterm.org/config/launch.html), [`ShowLauncherArgs`](https://wezterm.org/config/lua/keyassignment/ShowLauncherArgs.html), [workspaces](https://wezterm.org/recipes/workspaces.html), [fonts](https://wezterm.org/config/fonts.html), and [appearance](https://wezterm.org/config/appearance.html) |
| Alacritty | **Documented, not observed** | Official [manual](https://github.com/alacritty/alacritty/blob/master/extra/man/alacritty.5.scd) and [features](https://github.com/alacritty/alacritty/blob/master/docs/features.md) |
| kitty | **Documented, not observed** | Official [overview](https://sw.kovidgoyal.net/kitty/overview/), [configuration](https://sw.kovidgoyal.net/kitty/conf.html), and [font picker](https://sw.kovidgoyal.net/kitty/kittens/choose-fonts/) |
| Ghostty | **Documented, not observed** | Official [configuration overview](https://ghostty.org/docs/config) and [configuration reference](https://ghostty.org/docs/config/reference) |
| Warp | **Documented, not observed** | Official [block basics](https://docs.warp.dev/terminal/blocks/block-basics), [block actions](https://docs.warp.dev/terminal/blocks/block-actions), [block behavior](https://docs.warp.dev/terminal/appearance/blocks-behavior), and [keyboard shortcuts](https://docs.warp.dev/getting-started/keyboard-shortcuts) |
| tmux | **Documented, not observed** | Official wiki [Getting Started: sessions](https://github.com/tmux/tmux/wiki/Getting-Started#sessions), [windows and panes](https://github.com/tmux/tmux/wiki/Getting-Started#choosing-sessions-windows-and-panes), and [formats](https://github.com/tmux/tmux/wiki/Formats) |
| Zellij | **Documented, not observed** | Official [session-manager alias](https://zellij.dev/documentation/session-manager-alias.html), [session resurrection](https://zellij.dev/documentation/session-resurrection.html), [session management tutorial](https://zellij.dev/tutorials/session-management/), and [basic UI tutorial](https://zellij.dev/tutorials/basic-functionality/) |
| VS Code | **Documented, not observed** | Official [user-interface](https://code.visualstudio.com/docs/editing/userinterface) and [multi-root workspace](https://code.visualstudio.com/docs/editing/workspaces/multi-root-workspaces) documentation |

Count: **1 observed product, 11 documented-only products**.

## What the current Noren frame communicates

At the default 900×600 window and 10×20 physical-pixel cells, Noren draws a
90×30 grid. The fixed 16-column sidebar leaves 74 terminal columns. Its rows
are flat and ordered as sessions, configured projects, discovered worktrees,
SSH aliases, then configured agents. A selected row receives a leading `>`;
project, SSH, and agent rows encode state in fixed eight-character prefixes
such as `PRJ-OFF`, `SSH-ON`, and `AGT-ERR`.

That leaves 14 visible characters after the selection marker and separator.
For prefixed resource rows, only six characters remain for the name. A long
name is deliberately reduced to its first three characters plus `...`.
Session details and worktree branches are appended to the same line and then
clipped by the renderer's 16-column limit. There are no section headers,
disclosure triangles, indentation, tooltips, per-row buttons, or visible
scroll position. The sidebar itself can scroll vertically with the mouse.

**F-02 — Noren's kind/state encoding wins certainty by spending nearly all of
its identity budget.** The prefix always tells a user whether `SSH`, `PRJ`, or
`AGT` is offline, online, or failed, but two resources named `frontend` and
`freight` both become a very small, hard-to-disambiguate fragment. This is a
structural collision between fixed width and fixed prefixes, not merely an
unfortunate example label. Evidence: current
[`main.rs`](../../crates/noren-app/src/main.rs) computes a six-character target
budget and current [`renderer.rs`](../../crates/noren-app/src/renderer.rs)
clips every sidebar row at 16 scalar values.

## cmux-specific comparison

### What each sidebar regards as a thing

cmux's documented hierarchy is `Window → Workspace → Pane → Surface → Panel`;
the ordinary left-sidebar row is a running workspace. Noren's rail is a mixed
catalog: it contains runtime sessions and launchable facts before they become
sessions. This difference explains both Noren's strongest distinction and
most of the density mismatch.

| User concept | cmux presentation — **documented, not observed** | Noren presentation — **observed runtime, source-verified** |
| --- | --- | --- |
| Project | A workspace exposes title, cwd, Git branch, ports, and attached status. Named, collapsible [workspace groups](https://cmux.com/docs/workspace-groups) can collect related workspaces; project-local [custom commands](https://cmux.com/docs/custom-commands) appear in the command palette or configured buttons. | A persistent `PRJ-OFF`/`PRJ-ERR` launch row exists before a session. The configured root is not printed in the row. |
| Worktree | Current cmux release notes document a switchable built-in **Project Worktrees** sidebar view and an extension sample for project/worktree views; the ordinary workspace rail otherwise represents a worktree once a workspace's cwd is inside it. The [changelog](https://cmux.com/docs/changelog) does not document the alternate view's exact row layout, so none is assumed here. | Every discovered worktree is a first-class pre-launch row. Its final path component is the label and its branch is appended as detail; the branch is often outside the 16-column viewport. |
| SSH target | [`cmux ssh`](https://cmux.com/docs/ssh) accepts an SSH-config alias and creates a named remote workspace. [`cmux ssh-tmux`](https://cmux.com/docs/remote-tmux) projects remote tmux sessions into sidebar workspaces. The docs do not show a pre-connection alias browser or the config file that supplied an alias. | Literal aliases discovered from the bounded SSH-config subset are visible before connection. Selecting one exposes a stable source tag and a root-relative source label in the status row; wildcard omissions and caps are reported. |
| Agent | Agents run in terminal surfaces. Hooks feed workspace unread badges, notifications, status pills, progress, and logs; the [Task Manager](https://cmux.com/docs/task-manager) attributes processes to known coding agents. [Claude Code Teams](https://cmux.com/docs/agent-integrations/claude-code-teams) maps teammates to native splits. | A configured agent is a pre-launch `AGT-OFF` row, becoming failed on launch failure. It does not show attention, progress, last output, or an unread state. |

**F-03 — Noren and cmux do not actually expose the same ontology.** cmux is
workspace-first and enriches running workspaces with repository and agent
state. Noren is resource-first and lets dormant projects, worktrees, hosts, and
agent commands coexist with live sessions. Calling Noren's five row kinds
"workspace tabs" understates its useful pre-launch catalog and obscures why a
single row template is under such pressure.

### What cmux's sidebar does that Noren's does not

cmux documents all of the following in its current
[workspace-group guide](https://cmux.com/docs/workspace-groups),
[sidebar API](https://cmux.com/docs/api),
[notifications guide](https://cmux.com/docs/notifications), and
[changelog](https://cmux.com/docs/changelog):

- collapsible named groups with indentation, chevrons, icons, colors, pinned
  tiers, persistent collapse state, multi-selection, and drag reorder;
- a header `+`, row close control on hover, right-click actions, hover
  tooltips, reorder shortcuts, and alternate sidebar views;
- cwd, Git branch, ports, status pills, progress bars, logs, unread badges, and
  jump-to-unread behavior;
- viewport-aware path abbreviation, wrapped long workspace titles,
  configurable sidebar font size, precise native selection/hover, and a scroll
  indicator that appears while scrolling.

Noren provides one leading selection marker, one line per item, wheel
scrolling, and status prefixes. It provides none of the hierarchy, row actions,
activity metadata, alternate views, or overflow-recovery affordances above.
Its palette is prepended inside the same narrow rail, so opening it also
consumes rows that would otherwise show resources.

**F-04 — cmux uses progressive disclosure; Noren uses irreversible clipping.**
cmux can spend more height on a long title, abbreviate a path to the available
viewport, hide controls until hover, and collapse a group. Noren discards
everything after column 16 during every draw, with no hover, wrap, resize, or
drill-in path to recover the hidden identity. This is the largest cmux-specific
sidebar gap.

**F-05 — cmux rows answer "what needs me?"; Noren rows mainly answer "what can
I launch?"** cmux's unread lifecycle, status pills, progress, logs, agent
attribution, and notification jump turn the rail into an attention router.
Noren's explicit offline/online/failed prefixes are honest but mostly static.
Once two agents or builds run in the background, there is no equivalent signal
for completion, waiting, unseen output, or progress.

### What Noren's sidebar does that cmux's default does not

Noren's advantages are narrower, but real:

- **One pre-launch catalog.** A user can see configured projects, discovered
  worktrees, literal SSH aliases, configured agent commands, and live/restored
  sessions without first creating a workspace or switching sidebar views.
- **SSH provenance and honest incompleteness.** The selected SSH alias reports
  its source tag and bounded root-relative source label. It also reports
  wildcard aliases that cannot be listed and rows omitted by caps. cmux says
  that it reads `~/.ssh/config`, but its [SSH documentation](https://cmux.com/docs/ssh)
  does not document source-file attribution or an alias-inventory completeness
  statement.
- **Dormant and failed are first-class.** `OFF` and `ERR` distinguish a
  launchable fact from a running session without requiring the user to infer
  it from missing output. cmux has much richer running activity, but its
  default rail is not documented as a dormant resource inventory.

### Is the Zellij divergence principled or missing?

[ADR 0003](../adr/0003-noren-zellij-responsibility-boundary.md) deliberately
assigns projects, worktrees, SSH, agents, and sessions to Noren while assigning
tabs, panes, layouts, and focus inside the terminal to Zellij. That boundary is
**principled**: one user action should not have two competing tab models, two
split trees, or two persistence authorities. Zellij's own documented status
bar and session UI can remain internally consistent.

The current experience is nevertheless **incomplete at the handoff**. Nothing
in Noren's visible chrome says "tabs and panes are in Zellij," shows whether
Zellij owns the selected session, or points to Zellij's mode/status hints. A
cmux user sees the native pane/surface controls disappear and receives no
replacement explanation from Noren. Therefore native tabs and panes are not a
Noren parity gap; the absent visual handoff that makes the delegation
understandable is a gap.

**F-06 — The architecture reads as principled only after reading the ADR.** In
the preview itself, the same decision initially reads as missing functionality.
The distinction is important: evaluating Noren should not demand native pane
controls, but it should count the invisible ownership boundary as a UX deficit.

## Workspace-sidebar patterns elsewhere

The products below do not all solve the same product problem. The useful
comparison is how each allocates space among identity, hierarchy, truncation,
and state.

| Pattern | Density and hierarchy | Truncation and recovery | State and selection | Implication for Noren |
| --- | --- | --- | --- | --- |
| VS Code Explorer — **documented, not observed** | A resizable tree uses indentation, disclosure controls, icons, reorderable sections, and drag/drop. | Filtering can highlight or narrow matches; multi-root labels can use name or path formats to disambiguate. Sidebar width and location are user-controlled. | Full-row selection, active-file reveal, badges in the Activity Bar, hover actions, and persisted UI state. [Source](https://code.visualstudio.com/docs/editing/userinterface) | Five semantic kinds need grouping or another strong hierarchy signal; a prefix alone should not consume half a row. |
| Zellij session manager — **documented, not observed** | A floating task surface separates active and exited sessions and has room for tab counts, client/share metadata, names, folders, and layouts. | It is modal rather than permanently narrow; search and category switching spend the terminal area while the task is active. | Focus highlight plus active/exited/resurrectable state; create, rename, kill, and delete actions are co-located. [Sources](https://zellij.dev/documentation/session-manager-alias.html), [resurrection](https://zellij.dev/documentation/session-resurrection.html) | A temporary wide manager is a valid density strategy. Permanent visibility is not valuable if identities cannot be read. |
| tmux choose-tree — **documented, not observed** | `choose-tree` can show sessions only or an indented session/window/pane tree, with a preview pane below. | It uses the available terminal width; formats and filters are configurable, and search/sort/tag actions reduce the list. | Current/selected rows are highlighted; kill, tag, expand, and preview are in the same temporary mode. [Sources](https://github.com/tmux/tmux/wiki/Getting-Started#choosing-sessions-windows-and-panes), [formats](https://github.com/tmux/tmux/wiki/Formats) | Even a text UI does not require a permanently tiny rail: it borrows space and exposes modes/actions when management is underway. |
| Warp Blocks — **documented, not observed** | Command plus output is one atomic vertical block. Dividers, compact mode, and sticky command headers preserve structure in a dense history. | Long output remains scrollable; block selection and actions operate on the unit instead of relying on a clipped title. | Borders mark selected blocks, non-zero exits receive error styling, bookmarks mark position, and hover/right-click actions expose operations. [Sources](https://docs.warp.dev/terminal/blocks/block-basics), [actions](https://docs.warp.dev/terminal/blocks/block-actions) | A heterogeneous list benefits from visually bounded units and state channels beyond six characters of text. Blocks are a history hierarchy, not a workspace substitute. |
| WezTerm Launcher — **documented, not observed** | A modal list can combine tabs, domains, workspaces, launch items, and commands. | Numeric/alphabetic jump keys, vi navigation, and `/` fuzzy filtering reduce density rather than clipping every identity. | The focused item is highlighted and the launcher can display its own help text. [Source](https://wezterm.org/config/lua/keyassignment/ShowLauncherArgs.html) | Combining row kinds is viable when type, filter, and a wider task surface make them distinguishable. |
| iTerm2 Profiles — **documented, not observed** | Named profiles are launch presets for command, working directory, terminal behavior, and appearance; they are not live workspace rows. Tabs separately show live sessions. | Profiles are managed in a dedicated preferences surface rather than compressed into the terminal edge. | Selected profile is standard macOS list state; tabs use indicators for new output, activity, or a dead session. [Sources](https://iterm2.com/documentation-preferences-profiles-general.html), [general usage](https://iterm2.com/documentation-general-usage.html) | Noren usefully combines launch presets with live objects, but must keep their lifecycle distinction as legible as iTerm2's separate surfaces do. |

**F-07 — VS Code proves that persistent sidebars scale through hierarchy and
resizing, not fixed clipping.** Its tree is denser than Noren's rail, yet depth,
icons, full-row selection, filtering, and adjustable width preserve identity.

**F-08 — Zellij spends the whole task surface when session management is the
task.** Its active/exited sections and session metadata are readable because
the manager is temporary. Noren has chosen constant context at the cost of
usable identity once rows become realistic.

**F-09 — tmux shows that terminal-cell UI is not the cause of Noren's density
problem.** A text-only choose-tree still supplies indentation, highlight,
preview, filtering, sorting, tagging, and format control.

**F-10 — Warp's block model makes state spatial and redundant.** Boundary,
color, sticky command identity, exit state, bookmark position, and selection
border work together. Noren puts kind, lifecycle, identity, and detail into a
single clipped string, so losing characters also loses meaning.

**F-11 — WezTerm validates a mixed-kind launcher, but not a mixed-kind
16-column rail.** Its launcher combines categories successfully because it is
temporary, filterable, keyboard-labelled, and wide enough for the task.

**F-12 — iTerm2 keeps presets and runtime sessions separate.** Noren's unified
catalog is faster to scan in principle, but only if dormant resources and live
sessions remain visually distinct without consuming the resource's name.

## Missing terminal conventions

There is one correction to make to the premise that every peer "shows
scrollback position." A persistent scrollbar is not universal. WezTerm's
scrollbar is [disabled by default](https://wezterm.org/config/lua/config/enable_scroll_bar.html),
and Alacritty documents a line-position indicator specifically for search and
vi mode rather than a permanent bar. The stronger shared convention is this:
once the viewport leaves the bottom, the user can navigate history and receives
some orienting feedback in the products or modes that hide a permanent bar.
Noren supplies neither navigation nor orientation.

| Terminal | Cursor convention | Selection convention | Scrollback position/orientation convention |
| --- | --- | --- | --- |
| cmux — **documented, not observed** | Terminal rendering and preferences come from Ghostty, including visible block/bar/underline cursor behavior. | Ghostty selection is visible; cmux additionally documents optional copy-on-select. | Ghostty supplies terminal scrollback behavior; cmux separately documents transient scroll indicators for overflowing sidebars. [Sources](https://cmux.com/docs/configuration), [changelog](https://cmux.com/docs/changelog) |
| Terminal.app — **documented, not observed** | Profiles choose block, underline, or bar, blinking, and cursor color. | Profiles choose a selection color. | Profile window/scrollback settings retain history in the standard scrollable terminal view. [Sources](https://support.apple.com/guide/terminal/change-profiles-text-settings-trmltxt/mac), [profiles](https://support.apple.com/guide/terminal/use-profiles-to-change-the-look-of-terminal-windows-trml107/mac) |
| iTerm2 — **documented, not observed** | Block, vertical bar, underline, blink, cursor guide, context-aware color, and cursor boost are configurable. | Selected foreground/background are configurable and the selected range remains visibly differentiated. | A configured history size is navigable with ordinary terminal scrolling; scrollback and selection can continue while output arrives. [Sources](https://iterm2.com/documentation-preferences-profiles-text.html), [colors](https://iterm2.com/documentation-preferences-profiles-colors.html), [terminal](https://iterm2.com/documentation-preferences-profiles-terminal.html) |
| WezTerm — **documented, not observed** | The documented default is a visible `SteadyBlock`; applications may request another style. | `selection_fg` and `selection_bg` style the active selection, with default mouse bindings for cell, word, line, and block ranges. | Copy mode and scroll actions navigate history. An optional thumb represents the current viewport, although the scrollbar is off by default. [Sources](https://wezterm.org/config/lua/config/default_cursor_style.html), [appearance](https://wezterm.org/config/appearance.html), [mouse](https://wezterm.org/config/mouse.html), [scrollbar](https://wezterm.org/config/lua/config/enable_scroll_bar.html) |
| Alacritty — **documented, not observed** | Block, underline, or beam shape plus blink timing and cursor colors are configurable. | Selection foreground/background and save-to-clipboard behavior are configurable; vi-mode movement visibly extends selections. | History defaults to 10,000 lines. Search and vi mode expose a line indicator showing position in history; there is no permanent GUI scrollbar. [Sources](https://github.com/alacritty/alacritty/blob/master/extra/man/alacritty.5.scd), [features](https://github.com/alacritty/alacritty/blob/master/docs/features.md) |
| kitty — **documented, not observed** | A visible cursor has configurable shape, blink, color, and text color. | Mouse word/line/column selections remain visibly selected and can copy automatically. | An interactive right-edge scrollbar shows the current scrollback position when scrolled by default; keys, mouse, search, and an external pager navigate history. [Sources](https://sw.kovidgoyal.net/kitty/conf.html), [overview](https://sw.kovidgoyal.net/kitty/overview/) |
| Ghostty — **documented, not observed** | Cursor style, color, opacity, inversion, and blink are configurable. | Selection foreground/background default to contrasting terminal colors and can persist after copy. | `scrollbar = system` is the documented default and follows platform visibility behavior; `never` is an explicit opt-out. [Source](https://ghostty.org/docs/config/reference) |
| Warp — **documented, not observed** | Bar, block, or underline plus blinking are exposed in Settings and the command palette. | Normal, semantic, rectangular, and whole-block selections are visibly highlighted; a border marks selected blocks. | The scrollable block list has sticky command headers, bookmarks with positional previews, and standard page/top/bottom navigation. [Sources](https://docs.warp.dev/terminal/appearance/text-fonts-cursor/), [selection](https://docs.warp.dev/terminal/more-features/text-selection/), [blocks](https://docs.warp.dev/terminal/blocks/block-basics) |
| Noren — **observed runtime, source-verified** | Cursor coordinates exist in terminal state, but the shipped renderer emits no cursor geometry. | Drag and select-all state can be copied, but no selected cell is drawn differently. | History is bounded in memory, but rendering is hard-wired to its newest suffix. There is no terminal scroll-offset input, navigation, thumb, percentage, or line indicator. [Source](../known-limitations.md) |

tmux and Zellij are multiplexers rather than font/cursor rasterizers, so they
inherit the host terminal's basic cursor. Their own management and copy modes
still make focus and selection visible: tmux's choose/copy modes highlight the
current item, while Zellij's default top/bottom bars expose tab, session, mode,
and shortcut state. That makes them additional evidence that invisible state
is not an expected consequence of a terminal-cell UI.

### Cost rank for the three missing conventions

1. **Cursor — highest cost.** The convention is a visible block, bar, or
   underline at the terminal's authoritative insertion position, sometimes
   blinking and sometimes changed by the application. Without it, every shell
   edit, command-history edit, REPL, password prompt, and full-screen TUI loses
   the answer to "where will the next byte go?" A user can type, but cannot
   safely predict insertion or focus. The cost starts before or on the first
   keystroke.
2. **Scrollback navigation and position — second-highest cost.** The convention
   is that wheel/page/search/copy-mode input can leave the bottom, with a thumb,
   line indicator, status mode, sticky block header, or other feedback showing
   that the viewport moved. Noren's absence is more costly than a missing
   scrollbar: build output, test failures, SSH diagnostics, and agent logs that
   leave the screen cannot be revisited at all. The first long command exposes
   it.
3. **Selection highlight — third-highest cost.** The convention is reversible
   visual contrast over the exact cell range about to be copied, often with
   word, line, rectangular, or semantic expansion. Noren copies an invisible
   range. Users cannot verify whether they included a prompt, omitted the final
   character, crossed a wrapped line, or captured a secret. The first copy
   attempt exposes it.

**F-13 — The missing cursor is the fastest and most expensive convention
failure.** Terminal.app, iTerm2, WezTerm, Alacritty, kitty, Ghostty, and Warp
all document a visible, configurable cursor; cmux inherits one from Ghostty.
Noren has authoritative cursor state but withholds the feedback needed to use
that state.

**F-14 — Noren is missing the scrollback interaction, not merely scrollbar
chrome.** Minimal peers prove that a permanent bar is optional. None of them
uses Noren's combination of retained history, a permanently bottom-anchored
view, and no navigation or position feedback.

**F-15 — Invisible selection converts a familiar direct manipulation into a
blind command.** Every compared emulator documents contrasting selection or
selected-block state. Noren's copy result may be correct internally, but the
user has no perceptual evidence before committing it to the clipboard.

## Discoverability of the primary interaction

Noren's default `Super+p` opens a four-command palette at the top of the
sidebar. Once open, it shows `C`, `S`, `X`, and `F` next to the commands and
supports arrows, Enter, and Escape. Before it opens, there is no on-screen
shortcut hint, button, menu, hover target, or welcome message. Sidebar rows do
respond to click for selection, switching, or launching, but their chrome does
not advertise that behavior and those clicks do not expose the palette's
create/close commands. `F` currently dispatches a no-op because the sidebar is
always visible. Evidence: current
[`main.rs`](../../crates/noren-app/src/main.rs) and [known
limitations](../known-limitations.md).

| Product | How the primary management interaction surfaces |
| --- | --- |
| cmux — **documented, not observed** | The workspace rail is itself visible. Group headers expose `+` and context menus, rows expose close on hover, and commands also live in a searchable palette. Inline shortcut hints appear in newer navigation surfaces. [Sources](https://cmux.com/docs/workspace-groups), [changelog](https://cmux.com/docs/changelog) |
| VS Code — **documented, not observed** | Activity Bar icons, Explorer buttons, menus, title-bar layout controls, hover shortcut labels, and the Command Palette all lead to the same capabilities. `Shift+Cmd+P`/F1 is important, not exclusive. [Source](https://code.visualstudio.com/docs/editing/userinterface) |
| Warp — **documented, not observed** | Warp opens with a dismissible shortcut screen and keeps shortcuts discoverable through the command palette, searchable settings, and Resource Center. Its macOS palette is also `Cmd+P`. It does **not** expect documentation before the primary interaction. [Source](https://docs.warp.dev/getting-started/keyboard-shortcuts) |
| WezTerm — **documented, not observed** | The visible tab-bar `+` creates a tab; right-clicking it opens the launcher. Inside, selection keys and `/` filtering are explained, and configured launchers can include tabs, domains, workspaces, and commands. Advanced workspace composition may require configuration, but basic launch does **not** require documentation first. [Sources](https://wezterm.org/config/launch.html), [`ShowLauncherArgs`](https://wezterm.org/config/lua/keyassignment/ShowLauncherArgs.html) |
| Zellij — **documented, not observed** | The default status bar continuously lists mode-entry and context-dependent immediate-action keys; the session manager has a documented default chord and then exposes its actions in a focused surface. [Sources](https://zellij.dev/tutorials/basic-functionality/), [session manager](https://zellij.dev/documentation/session-manager-alias.html) |
| tmux — **documented, not observed** | tmux is documentation-first for `prefix` sequences such as `C-b s` and `C-b w`, although its persistent status line at least makes sessions/windows and active state visible. It is a keyboard-first precedent, not a GUI-discoverability precedent. [Source](https://github.com/tmux/tmux/wiki/Getting-Started) |
| kitty — **documented, not observed** | kitty explicitly describes itself as designed for power keyboard users and publishes the default tab, window, scroll, and search bindings in its overview. Advanced operation is documentation/configuration-led. It is the closest GUI terminal to docs-first, but its keyboard model is the product posture rather than one hidden gateway to an otherwise visible workspace manager. [Source](https://sw.kovidgoyal.net/kitty/overview/) |
| Terminal.app and iTerm2 — **documented, not observed** | Standard macOS menus, tab/window controls, profile windows, and preferences expose the primary objects; shortcuts accelerate visible commands. iTerm2 can show the profiles window at startup. [Sources](https://support.apple.com/guide/terminal/use-profiles-to-change-the-look-of-terminal-windows-trml107/mac), [iTerm2 profiles](https://iterm2.com/documentation-preferences-profiles-general.html) |

**F-16 — Noren is a discoverability outlier among GUI workspace tools, though
not among all keyboard-first terminal software.** tmux and kitty show that a
docs-first posture can be deliberate. Noren's mismatch is that its sidebar
looks product-like while the only command surface for creating or closing a
session is behind an undisclosed key. Clickable rows partially cover selection
and launch, but do not reveal that command surface. The palette is recoverable
only after the user already knows how to recover it. For the yes/no product decision:
**discoverability outlier = yes**.

## Legibility at normal viewing distance

Noren's printable text uses hand-authored 5×7 bitmaps. Each lit bit becomes a
2×2 physical-pixel square, so ordinary text has a maximum 10×14-pixel ink
envelope inside a 10×20-pixel cell, with a fixed three-pixel top inset. The
bitmap is not anti-aliased or shaped. Increasing configured cell width or
height increases the grid spacing but leaves ordinary text at the same 2×2
pixel scale; only box-drawing glyphs stretch to the whole cell. Printable
ASCII, Latin-1, and Box Drawing have bitmaps. Other Unicode, including CJK and
emoji, becomes a visible replacement glyph even though cell width remains
correct.

On a common 2× Retina surface, the default cell corresponds to roughly 5×10
logical points and the maximum text ink to 5×7 logical points. The comparison
set normally starts around an 11–13-point real font and lets the rasterizer use
the display scale. At ordinary laptop distance, Noren's short ASCII is
decipherable, but visibly stair-stepped and materially smaller in apparent ink
height. A prompt using Nerd Font symbols, emoji, Japanese, Chinese, or Korean
does not merely look worse; its distinguishing characters collapse to the same
replacement form.

| Product | Documented text baseline — **not observed** |
| --- | --- |
| cmux | Uses Ghostty's font stack; its own configuration example uses SF Mono 13 for terminal text and a separately configurable 14-point sidebar font. [Source](https://cmux.com/docs/configuration) |
| Terminal.app | Uses a selected macOS font, typeface, and point size with optional font smoothing. [Source](https://support.apple.com/guide/terminal/change-profiles-text-settings-trmltxt/mac) |
| iTerm2 | Uses real primary and non-ASCII fonts, anti-aliasing, ligatures, fallback, bold/italic variants, and configurable point sizes. [Source](https://iterm2.com/documentation-preferences-profiles-text.html) |
| WezTerm | Bundles JetBrains Mono, Symbols Nerd Font, and Noto Color Emoji, with font fallback, shaping, hinting, rasterizer, anti-aliasing, line-height, and point-size controls. [Source](https://wezterm.org/config/fonts.html) |
| Alacritty | The macOS default is Menlo at 11.25 points; family, style, size, offset, and glyph offset are configurable. [Source](https://github.com/alacritty/alacritty/blob/master/extra/man/alacritty.5.scd) |
| kitty | Defaults to an 11-point font, supports separate regular/bold/italic faces and Unicode-range fallbacks, and provides an on-screen picker with rendered previews. [Sources](https://sw.kovidgoyal.net/kitty/conf.html), [font picker](https://sw.kovidgoyal.net/kitty/kittens/choose-fonts/) |
| Ghostty | Embeds JetBrains Mono as a usable zero-config default and resolves configured font families, styles, fallbacks, and color emoji at a point size appropriate to the display. [Sources](https://ghostty.org/docs/config), [reference](https://ghostty.org/docs/config/reference) |
| Warp | Defaults to Hack and exposes font family, weight, size, line height, stroke treatment, minimum contrast, and ligatures in Settings. [Source](https://docs.warp.dev/terminal/appearance/text-fonts-cursor/) |
| tmux and Zellij | Render through the host emulator's chosen font. Their text UI therefore inherits Terminal.app/iTerm2/WezTerm/kitty/Ghostty-class glyph coverage rather than imposing a 5×7 raster. |

**F-17 — The legibility gap is a rendering-generation gap, not a font-choice
preference.** Peers rasterize point-sized fonts with anti-aliasing, shaping,
styles, fallback, and Unicode coverage. Noren draws a fixed 10×14 physical-pixel
bitmap envelope, cannot enlarge normal glyph ink through its cell-size setting,
and substitutes most Unicode. The result reads as a proof-of-concept at first
glance and becomes functionally unreadable for common international or
symbol-rich terminal output.

## Where Noren is better

The following are present advantages, not promises:

1. **Cross-kind pre-launch context.** No documented default comparator in this
   set puts configured projects, discovered worktrees, SSH aliases, agent
   commands, and live/restored terminal sessions into one continuously visible
   launch surface. WezTerm combines several runtime/launch categories in a
   modal launcher and cmux has alternate/custom views, but Noren's five-kind
   outer catalog is the more direct statement of external context.
2. **SSH source attribution.** Noren tells the user which bounded config source
   supplied a selected alias without exposing the HOME prefix, and distinguishes
   literal aliases from unlistable wildcard rules. cmux reads SSH aliases,
   WezTerm can enumerate SSH hosts, and iTerm2 profiles can launch SSH, but none
   of their cited documentation describes equivalent per-alias source-file
   provenance.
3. **Honest partial and failure state.** Caps, wildcard omissions, unreadable
   configuration, missing roots, offline resources, and launch failure become
   explicit state or diagnostics rather than silent absence. That is unusually
   careful for preview UI and helps a user distinguish "not configured," "not
   running," and "could not run."
4. **One layout authority.** ADR 0003 avoids a Noren tab/pane tree fighting the
   nested multiplexer. cmux's native panes are richer on screen, but Noren's
   boundary is conceptually cleaner for a product explicitly built around
   Zellij—once the user has learned the boundary.

**F-18 — Noren's strongest differentiation is provenance-aware external
context, not terminal chrome.** The mixed pre-launch catalog, SSH source
attribution, explicit partial/offline/failure states, and non-duplicated Zellij
layout ownership are all defensible advantages. They do not offset the basic
feedback gaps, but they are the reasons the preview is worth making legible.

## Gap ranking by time to notice

This ordering is intentionally **not severity order**. It predicts the first
session: the trigger that exposes a gap, and which prior-product muscle memory
makes it obvious.

| Notice rank | Gap | First ordinary trigger | Who notices fastest | Likely first-session reading |
| ---: | --- | --- | --- | --- |
| 1 | No visible cursor (F-13) | Looking at the first prompt or pressing the first key: seconds | Users from every terminal, especially shell/REPL and TUI users | "Input or focus is broken." This is the top gap by time to notice. |
| 2 | Fixed 5×7 bitmap and replacement Unicode (F-17) | Reading the initial prompt; immediate for CJK, emoji, or Nerd Font prompts | Everyone; users from Ghostty, kitty, WezTerm, iTerm2, and Warp notice the rendering regression most sharply | "This is a renderer prototype, not yet a daily terminal." |
| 3 | Hidden `Super+p` primary gateway (F-16) | First attempt to create or close a session, or to look for all workspace commands: usually under a minute | cmux, VS Code, Warp, WezTerm, Terminal.app, and iTerm2 users who scan visible controls/menus | "I can select rows, but I do not know how to manage the workspace." |
| 4 | Zellij ownership is not communicated (F-06) | First attempt to make a tab or split | cmux, iTerm2, WezTerm, kitty, and tmux users; Zellij users understand only after starting Zellij themselves | "Tabs/panes are missing," even though their omission is intentional. |
| 5 | Six-character resource identity and fixed 16-column clipping (F-02/F-04) | First launch with realistic project/host/agent names or several worktrees | cmux and VS Code users with populated sidebars; SSH users with similarly prefixed aliases | "I cannot tell my resources apart." |
| 6 | Invisible selection (F-15) | First drag or `Cmd+A`, then copy | Users from every terminal; log/SSH users reach it quickly | "Did it select anything, and what will I copy?" |
| 7 | No terminal scrollback navigation or position (F-14) | First command whose output exceeds the pane | Developers running builds/tests and users arriving from kitty, Warp, iTerm2, or tmux copy mode | "Earlier output is gone," despite being retained internally. |
| 8 | No unread/progress/attention routing (F-05) | First background agent, build, or remote task completing out of view | cmux and Warp users, then multi-session users generally | "The sidebar lists things but does not tell me where to look." |

### Ranked inventories

- **Missing conventions by ordinary-work cost:** cursor → scrollback
  navigation/position → selection highlight.
- **cmux-specific gaps:** progressive disclosure for row identity → grouping
  and hierarchy → activity/unread/progress state → visible row actions →
  search/reorder/alternate views → a visible Zellij handoff.
- **Noren-over-cmux advantages:** one heterogeneous pre-launch catalog → SSH
  source provenance → explicit partial/offline/failure state → no duplicated
  pane/layout authority.

## Bottom line

Noren's sidebar model is not a smaller cmux sidebar. It is a different and, in
some ways, better outer-resource inventory. Yet the preview asks users to infer
that distinction through clipped labels and a hidden palette while withholding
the three feedback loops that make terminal interaction trustworthy. The
first-session abandonment predictor is therefore not missing cmux pane parity;
it is the absent cursor, followed immediately by bitmap legibility and the
undisclosed route into workspace management.

**Total findings: 18 (F-01 through F-18).**
