# Terminal landscape research

Status: Milestone 0 research snapshot, not an architecture or adoption decision

Retrieved: 2026-08-03 (Asia/Tokyo)

Scope: GitHub Issue [#3](https://github.com/ta-061/noren/issues/3)

## Reading this report

This report records primary-source evidence about terminal projects that are
relevant to Noren's macOS and Linux goals. It does not select a terminal core,
renderer, PTY, multiplexer, protocol, or UI toolkit. Release dates and branch
commits are observations, not a maintenance score. A tagged release can lag an
active default branch, and a recent commit does not prove API stability,
security, or long-term stewardship.

The transferable lessons below are interpretations to test. They are kept
separate from the evidence table and do not authorize copying upstream code,
assets, themes, documentation, icons, names, or marks.

## Reference baseline

The projects below do not define terminal correctness by themselves. Candidate
tests also need versioned standards and protocol references:

- [ECMA-48, 5th edition](https://ecma-international.org/publications-and-standards/standards/ecma-48/)
  defines control functions, but does not specify every de facto terminal
  behavior.
- The official [xterm control-sequence reference](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)
  records xterm behavior and extensions. A cited behavior still needs a test
  fixture because terminal implementations can disagree.
- Unicode [UAX #11 revision 44](https://www.unicode.org/reports/tr11/tr11-44.html)
  and [UAX #29 revision 47](https://www.unicode.org/reports/tr29/tr29-47.html)
  correspond to Unicode 17.0.0. UAX #11 explicitly says East Asian Width is not
  an off-the-shelf solution for modern terminals, so width tailoring must remain
  measured rather than assumed.
- The [terminfo database format](https://invisible-island.net/ncurses/man/terminfo.5.html)
  describes declared capabilities. A `TERM` name is evidence of a declaration,
  not proof that an emulator implements every behavior correctly.

## Project evidence

All source, release, commit, and license links in this section are upstream
project or forge records retrieved on 2026-08-03.

### Alacritty

**Observed evidence.** Alacritty describes itself as a terminal emulator that
integrates with other applications instead of reimplementing their functions,
and lists BSD, Linux, macOS, and Windows as supported platforms in its
[official site](https://alacritty.org/). The repository separates the reusable
[`alacritty_terminal`](https://github.com/alacritty/alacritty/tree/v0.17.0/alacritty_terminal)
state layer from the GUI application and consumes the separately published
[`vte`](https://github.com/alacritty/vte) parser. Release
[`v0.17.0`](https://github.com/alacritty/alacritty/releases/tag/v0.17.0) was
published 2026-04-06; default-branch commit
[`852e971cddfa`](https://github.com/alacritty/alacritty/commit/852e971cddfa)
is dated 2026-07-14. The upstream license is
[`Apache-2.0`](https://github.com/alacritty/alacritty/blob/v0.17.0/LICENSE-APACHE).

**Interpretation to test.** Parser, terminal state, and GUI separation is a
useful replaceability lesson. Alacritty's deliberate delegation to external
tools is also relevant to Noren's promise not to replace Zellij. Neither point
proves that Alacritty's internal crates are stable public APIs or that its
feature boundary matches Noren's.

### WezTerm

**Observed evidence.** WezTerm's
[official documentation](https://wezterm.org/index.html) describes a Rust
terminal emulator and multiplexer for Linux, macOS, Windows, FreeBSD, and
NetBSD, with local and remote panes. Its source tree has visible separations for
[`term`](https://github.com/wezterm/wezterm/tree/fa0a1da0f93f/term),
[`mux`](https://github.com/wezterm/wezterm/tree/fa0a1da0f93f/mux),
[`wezterm-gui`](https://github.com/wezterm/wezterm/tree/fa0a1da0f93f/wezterm-gui),
and [`wezterm-ssh`](https://github.com/wezterm/wezterm/tree/fa0a1da0f93f/wezterm-ssh).
The latest GitHub release observed was
[`20240203-110809-5046fc22`](https://github.com/wezterm/wezterm/releases/tag/20240203-110809-5046fc22),
published 2024-02-03, while default-branch commit
[`fa0a1da0f93f`](https://github.com/wezterm/wezterm/commit/fa0a1da0f93f) is dated
2026-08-02. The source license is
[`MIT`](https://github.com/wezterm/wezterm/blob/fa0a1da0f93f/LICENSE.md).

**Interpretation to test.** The repository is useful evidence that terminal,
multiplexer, GUI, PTY, and SSH responsibilities can be isolated. The two-year
release/branch gap is a concrete reason to measure against a pinned commit or
crate version rather than the word “latest.” It is not evidence that internal
modules are supported as external APIs.

### Ghostty

**Observed evidence.** Ghostty documents a shared Zig core, `libghostty`, with
an AppKit/SwiftUI macOS frontend and GTK4 Linux frontend in its
[architecture overview](https://ghostty.org/docs/about). That page explicitly
says the standalone C API is not yet stable. The official feature page records
[Metal on macOS and OpenGL on Linux](https://ghostty.org/docs/features), and the
VT reference labels its own coverage a work in progress. The signed
[`v1.3.1` tag object](https://api.github.com/repos/ghostty-org/ghostty/git/tags/22efb0be2bbea73e5339f5426fa3b20edabcaa11)
was created 2026-03-13 at commit
[`332b2aefc6e7`](https://github.com/ghostty-org/ghostty/commit/332b2aefc6e72d363aa93ab6ecfc86eeeeb5ed28);
no separate GitHub Release object was observed. Default-branch commit
[`bab076c1a2df`](https://github.com/ghostty-org/ghostty/commit/bab076c1a2df)
is dated 2026-08-02. The source license is
[`MIT`](https://github.com/ghostty-org/ghostty/blob/v1.3.1/LICENSE).

**Interpretation to test.** A platform-native shell around a shared terminal
core is a concrete separation pattern, and the different Metal/OpenGL backends
show that renderer choice can remain platform-specific. The explicit unstable
API notice means `libghostty` must not be treated as a ready Noren dependency
without a later versioned evaluation.

### kitty

**Observed evidence.** kitty's
[official overview](https://sw.kovidgoyal.net/kitty/overview/) describes a
C/Python/Go codebase, direct OpenGL rendering, layouts, scripting, and remote
control on Linux and macOS. Its
[remote-control protocol](https://sw.kovidgoyal.net/kitty/rc_protocol/) includes
a protocol version, bounded fields for some operations, socket-only mode, and
an authenticated-encryption option. Its terminal extensions are published as
[protocol documentation](https://sw.kovidgoyal.net/kitty/protocol-extensions/).
Release [`v0.48.2`](https://github.com/kovidgoyal/kitty/releases/tag/v0.48.2)
was published 2026-07-30; default-branch commit
[`f293199d30d0`](https://github.com/kovidgoyal/kitty/commit/f293199d30d0) is
dated 2026-08-02. The repository contains the
[GPL version 3 text](https://github.com/kovidgoyal/kitty/blob/v0.48.2/LICENSE),
and GitHub reports legacy SPDX `GPL-3.0`; this pass did not resolve
`GPL-3.0-only` versus `GPL-3.0-or-later` from per-file notices.

**Interpretation to test.** Published, versioned protocols can support
black-box interoperability without adopting an implementation. Remote control
also demonstrates why IPC needs explicit authorization, framing, size limits,
and version negotiation. GPL implementation code is outside Noren's intended
MIT/Apache-2.0 licensing boundary unless a separate legal and project decision
changes that boundary; protocol documentation still needs independent,
specification-driven implementation and tests.

### foot

**Observed evidence.** foot is a Linux/Wayland terminal whose official
[`README`](https://codeberg.org/dnkl/foot/src/tag/1.27.0/README.md) describes a
client/server mode and a deliberate Wayland-only platform scope. Codeberg
release [`1.27.0`](https://codeberg.org/dnkl/foot/releases/tag/1.27.0) was
published 2026-05-15; default-branch commit
[`8db88cceb758`](https://codeberg.org/dnkl/foot/commit/8db88cceb758) is dated
2026-07-30. The source license is
[`MIT`](https://codeberg.org/dnkl/foot/src/tag/1.27.0/LICENSE).

**Interpretation to test.** A constrained display-server target can reduce the
number of event, rendering, clipboard, and IME paths. That benefit cannot be
transferred directly to Noren because Noren also targets macOS and Linux X11
remains an unresolved product-support question. The client/server mode is an
IPC study subject, not an architecture recommendation.

### Rio

**Observed evidence.** Rio's official
[`README`](https://github.com/raphamorim/rio/tree/v0.5.5#rio-terminal) describes
a Rust terminal using its Sugarloaf renderer and lists macOS, Linux, Windows,
and WebAssembly targets. The repository exposes separate
[`rio-backend`](https://github.com/raphamorim/rio/tree/v0.5.5/rio-backend),
[`rio-window`](https://github.com/raphamorim/rio/tree/v0.5.5/rio-window), and
[`sugarloaf`](https://github.com/raphamorim/rio/tree/v0.5.5/sugarloaf) trees.
Release [`v0.5.5`](https://github.com/raphamorim/rio/releases/tag/v0.5.5) was
published 2026-08-01; commit
[`497f97151bea`](https://github.com/raphamorim/rio/commit/497f97151bea) is from
the same date. The source license is
[`MIT`](https://github.com/raphamorim/rio/blob/v0.5.5/LICENSE).

**Interpretation to test.** Backend/window/renderer directories provide a
concrete comparison point for replaceable boundaries. Directory separation
alone does not establish public API stability, test coverage, or suitability
for Noren; those require a pinned-source PoC.

### Zellij

**Observed evidence.** Zellij is a terminal workspace and multiplexer, not a
terminal emulator. Its [official documentation](https://zellij.dev/documentation/)
defines modes, panes, sessions, and WebAssembly plugins. Release
[`v0.44.3`](https://github.com/zellij-org/zellij/releases/tag/v0.44.3) was
published 2026-05-13; default-branch commit
[`0e6e4404027a`](https://github.com/zellij-org/zellij/commit/0e6e4404027a) is
dated 2026-07-31. The source license is
[`MIT`](https://github.com/zellij-org/zellij/blob/v0.44.3/LICENSE.md).

**Interpretation to test.** Zellij is primarily a compatibility stakeholder:
Noren must forward input, size changes, mouse reports, terminal replies, and
session lifecycle correctly when Zellij owns layout and key modes. Zellij's
plugin model is useful comparative evidence, but its multiplexer policy must
not be silently duplicated in the emulator layer.

### iTerm2

**Observed evidence.** iTerm2 is a macOS-native terminal with documented
[tmux integration](https://iterm2.com/documentation-tmux-integration.html),
[scripting APIs](https://iterm2.com/python-api/), and an
[accessibility setting](https://iterm2.com/documentation-preferences-profiles-general.html)
for exposing the full buffer to assistive technology. The official downloads
page records [version 3.6.11](https://iterm2.com/downloads.html), built
2026-06-02; default-branch commit
[`279d3c068d46`](https://github.com/gnachman/iTerm2/commit/279d3c068d46) is
dated 2026-08-02. The repository contains the
[GPL version 2 text](https://github.com/gnachman/iTerm2/blob/279d3c068d46/LICENSE),
and GitHub reports legacy SPDX `GPL-2.0`; this pass did not resolve
`GPL-2.0-only` versus `GPL-2.0-or-later` from per-file notices.

**Interpretation to test.** Native macOS integration is a useful behavior and
accessibility reference, while its lack of a Linux frontend prevents it from
serving as a cross-platform baseline. Its tmux integration also shows that a
multiplexer can remain a distinct protocol/lifecycle concern. GPL source is
outside the intended Noren code-reuse boundary without a separate legal and
project decision.

## Cross-project lessons to validate

These are research recommendations, not selected architecture. Each has a
measurement that can falsify it.

1. **Keep byte parsing and terminal state independently testable.** Feed an
   identical bounded corpus of ECMA-48, xterm, OSC, DCS, malformed UTF-8, and
   truncated sequences through candidate boundaries. Compare normalized grid,
   cursor, mode, reply, and error events; measure peak memory and time per byte.
2. **Keep terminal state independent of a specific renderer.** Replay a fixed
   grid/damage trace through two rendering PoCs on Metal-capable macOS and
   Vulkan/OpenGL Linux. Measure frame time, upload bytes, idle CPU, resize
   behavior, device loss, and whether a renderer failure leaves the PTY alive.
3. **Treat platform input as more than key-down events.** Exercise dead keys,
   Japanese IME preedit/commit, emoji, Option/Alt, AltGr, compose, key repeat,
   focus changes, and candidate-window placement on macOS, Wayland, and X11.
   Record exact event sequences and forwarded PTY bytes.
4. **Treat multiplexers as guests, not hidden dependencies.** Run version-pinned
   Zellij and tmux fixtures locally and over SSH. Assert that Noren shortcuts do
   not consume guest input in pass-through mode and that resize, mouse, paste,
   focus, and terminal-query replies arrive unchanged.
5. **Version and authorize control surfaces.** Prototype a local-only IPC
   command with a version field, peer-identity check, least-privilege command
   allowlist, strict frame cap, timeout, and redacted audit event. Fuzz framing
   and verify that an untrusted terminal byte stream cannot invoke IPC.
6. **Measure release and source cadence separately.** For every PoC pin, record
   crate/release version, source commit, toolchain, license files, dependency
   lock, and advisory scan date. A later ADR should reject claims based only on
   stars, download counts, or an unversioned default branch.
7. **Build accessibility from semantic state, not pixels.** Expose a small grid,
   selection, focused pane, title, and scrollback window through each candidate
   accessibility path. Test VoiceOver and a Linux AT-SPI screen reader for
   navigation, selection, live updates, large scrollback bounds, and secret
   redaction; record user-visible behavior rather than API-call success alone.

## Legal and clean-room boundary

- Use official standards, public protocol documentation, and black-box behavior
  as test inputs; do not translate upstream implementation code line by line.
- Do not copy screenshots, icons, bundled fonts, themes, example assets,
  product names, or marks into Noren.
- Preserve upstream license notices for any dependency actually evaluated. A
  license identifier in this report is screening evidence, not legal advice.
- MIT/Apache-2.0 projects may still contain third-party files under other terms;
  a later dependency review must inspect the exact packaged source and lockfile.
- GPL-licensed kitty and iTerm2 are behavior/protocol references only in this
  report. No source or asset reuse is proposed.

## Explicit unknowns

- The supported VT sequence set, reply semantics, OSC/DCS limits, reflow model,
  Unicode version, grapheme policy, ambiguous-width policy, and image protocols
  for Noren are not decided.
- macOS, Wayland, and X11 IME event equivalence is not established. Linux X11
  support itself remains a product requirement question outside this report.
- None of the observed internal project modules is assumed to be a supported
  library API unless its own versioned documentation says so.
- Default-branch activity does not establish response time for security reports,
  release support windows, or future maintenance.
- No accessibility candidate has yet been tested with a terminal-sized dynamic
  text tree, large scrollback, rapid updates, or protected input.
- No project in this survey is a compatibility oracle. Differential results that
  disagree must be resolved against standards, protocol origins, documented
  application needs, and explicit Noren requirements.

## Completion evidence for this snapshot

The report covers Rust and native terminal emulators, a multiplexer that Noren
must coexist with, platform-specific and cross-platform approaches, dated
release/commit evidence, source licenses, transferable lessons, validation
experiments, and code/asset boundaries. Candidate libraries are compared
separately in [library-comparison.md](library-comparison.md); neither report
makes an adoption decision.
