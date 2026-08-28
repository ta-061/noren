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
