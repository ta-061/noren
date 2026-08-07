# Handoff — renderer attribute wiring lane (Qwen 3.8 Max Preview via opencode, `qwen-render`)

## Identity

- **Lane:** `qwen-render` (wire cell colors and attributes into drawing),
  engine `qwencloud/qwen3.8-max-preview` via opencode.
- **Branch:** `agent/renderer-attributes`, branched from `origin/main` @
  `91a0536` (388 workspace tests passing at branch point).
- **Leased files:** `crates/noren-app/src/renderer.rs` (modified) and
  `crates/noren-app/tests/renderer_attributes.rs` (new). No edits to
  `lib.rs`, `main.rs`, `Cargo.toml`, or `Cargo.lock`; nothing deleted.

## Gap verification at HEAD

Confirmed before doing any work. On `origin/main` (`91a0536`)
`crates/noren-app/src/renderer.rs`:

- the WGSL fragment shader took no input and returned one constant color,
  `vec4<f32>(0.80, 0.92, 0.82, 1.0)`;
- the vertex layout carried a single `Float32x2` position attribute (stride
  8) — no color channel existed;
- the draw path consumed only `TerminalSnapshot::display_lines()` (`&[String]`),
  so `Cell`/`CellAttributes` were unreachable from drawing by construction;
- the frame clear color was a hardcoded `LoadOp::Clear`.

Every modeled color, bold, underline, and reverse flag was computed by the
terminal state and discarded at render time. The gap was real; the work below
is not invented.

## What was done

1. **Vertex format + shader:** each vertex now carries position (2 floats)
   plus RGB color (3 floats), stride 20. The vertex shader passes color
   through; the fragment shader returns it. The window clear color is derived
   from `DEFAULT_BACKGROUND` instead of a scattered literal, so clear and
   default-bg cells match exactly.
2. **Palette as one named table:** `DEFAULT_ANSI_PALETTE` holds the 16 ANSI
   colors (xterm values); `DEFAULT_PALETTE` derives the full xterm 256-entry
   table (ANSI 16 + 6×6×6 cube + grayscale ramp) from it in one `const fn`. A
   future theme replaces `DEFAULT_ANSI_PALETTE` (and the default fg/bg
   constants) in one place. `DEFAULT_FOREGROUND`/`DEFAULT_BACKGROUND` preserve
   the old hardcoded glyph/clear colors (204,235,209 / 9,11,10).
3. **Pure resolution seam:** `resolve_color(Color, default)` and
   `resolve_cell_colors(&CellAttributes) -> ResolvedCellColors` are pure
   functions called directly by the tests. Resolution order: palette-resolve
   fg/bg first, **then** reverse swaps the two resolved colors. An explicit
   underline color passes through reverse untouched; a default underline color
   follows the (possibly swapped) foreground. `Color::Rgb` is used directly.
4. **Drawing:** `glyph_vertices` now walks the captured screen's cells (one
   per display column) instead of string characters, resolving each cell's
   attributes. Background rects draw only when the resolved background differs
   from the default; wide leads span both columns for background and underline
   fills. Bold widens each glyph pixel by one physical pixel (same rect
   count). Underline draws a 2px bar at the cell bottom. The status line keeps
   default-foreground text. The `MAX_VERTICES` bound grew from `35` to
   `35 + 2` rects per cell (background + underline); bold adds no rects.
5. **Tests:** seven renderer unit tests updated for the new vertex shape (all
   pre-existing wide-character layout tests unchanged and passing), plus eight
   new integration tests in `tests/renderer_attributes.rs`, which includes
   `renderer.rs` via `#[path]` (the module belongs to the binary; `lib.rs`
   and `main.rs` are untouched per lease).

## Gate output (commands actually run, at working-tree state; see caveat)

    $ cargo fmt --all -- --check
    (no output — clean)

    $ cargo clippy --workspace --all-targets -- -D warnings
        Checking noren-terminal v0.1.0 (...)
        Checking noren-app v0.1.0 (...)
        Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.38s

    $ cargo test --workspace
    ... 26 test binaries, every line:
    test result: ok. ... 0 failed ...
    Total: 405 passed, 0 failed, 1 ignored

## Worktree collision — read this before resuming

**Two `qwen-render` opencode processes ran concurrently in this same worktree**
(`pool-render`), with two different versions of the task prompt. Evidence:
duplicate processes visible in `ps` during the run, and
`crates/noren-terminal/src/state.rs` being edited outside this lane's lease by
the other instance (adding a `display_cells()` snapshot accessor plus tests,
and at one point shipping a different `renderer.rs` draft using
`ResolvedColors`/`resolve_colors` naming).

What this lane did about it:

- never touched `state.rs`; the other instance's uncommitted `state.rs`
  changes remain in the working tree and are **not part of this commit**;
- snapshotted the other draft to
  `$TMPDIR/opencode/qwen-render-collision/renderer.theirs.rs` and
  `state.theirs.rs` for the coordinator/reviewer;
- kept this lane's `renderer.rs` self-contained: it uses only the existing,
  already-public `TerminalSnapshot::screen()`/`ScreenBuffer::row()` accessors,
  so the commit does not depend on the other lane's `display_cells()` work;
- the gate above ran with the other lane's uncommitted `state.rs` present in
  the tree (it is additive and self-contained, and its tests pass). A reviewer
  re-running the gate at exactly this commit should see the same results if
  the tree is clean, or ~3 fewer noren-terminal tests if `state.rs` was
  reverted — neither difference touches noren-app.

Whoever merges or reviews this branch should reconcile the duplicate lane
deliberately (one `display_cells()` accessor may still be wanted later; this
lane simply did not need it).

## What could NOT be verified without a rendered-frame oracle

The project has no rendered-frame oracle and this lane does not pretend
otherwise. Not verified:

- that the GPU pipeline with the new two-attribute vertex layout is accepted
  and produces correct pixels on real Metal hardware (`Renderer::new`/`render`
  compile but are never executed in tests — they need a window);
- actual on-screen appearance of colors, bold thickening, underline placement,
  reverse-video blocks, or background rects;
- interaction with window resize and the vertex-buffer regrowth path beyond
  the byte-length calculation.

What IS verified headlessly: every `Color` variant resolves to the expected
concrete RGB; palette entries 0–15/cube/grayscale endpoints; reverse swaps
after palette resolution and composes with explicit colors (including the
order proof: default-fg + indexed-bg reversed); default cells resolve to
palette defaults in both the pure function and the vertex stream; bold widens
pixel rects 2px→3px with unchanged placement; underline bar geometry and both
default and SGR-58 explicit colors; SGR-7 draw output swaps colors into
rects; wide characters keep their two-column footprint with attributes
(background rect spans two columns, following glyph lands at the right display
column).

## Known limitations (deliberate, documented)

- trailing screen rows that `display_lines` trims as entirely blank draw
  nothing even if their cells carry a non-default background; row selection
  semantics (and status-line placement) were preserved exactly rather than
  rederived;
- combining marks still render as per-column fallback glyphs inheriting the
  base cell's attributes (existing renderer behavior, unchanged);
- vertex budget covers one glyph per cell plus background and underline;
  deeper combining stacks truncate as before.
