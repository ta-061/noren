# Test strategy: first macOS local-PTY PoC

- Status: Accepted strategy; codex-lab correction check clean at `2bdd7bea`
- Date: 2026-08-03
- Requirements: [v0.1 requirements](../requirements/v0.1.md)
- Threat model: [local-PTY PoC](../security/threat-model.md)

Tests are deterministic below the final macOS smoke layer. Interactive zsh is a
demonstration target, not a test oracle. Each requirement test names its fixture,
platform, pass bar, and captured evidence; no screenshot alone proves PTY or
input correctness.

## Test layers

| Layer | Owner and seam | Required coverage |
| --- | --- | --- |
| Unit | `noren-app`: `KeyEncoder`, size conversion, shutdown state machine | Exact bytes for printable UTF-8, Enter/Backspace/Tab/Escape/arrows/Ctrl, press/repeat/release and unsupported modifiers; zero bytes plus payload-free drop event for unsupported input; checked pixel-to-cell boundaries; duplicate/zero/overflow sizes; every valid and repeated shutdown transition. |
| Unit | `noren-terminal`: `TerminalEngine`, `CellWidth` | Prompt/ASCII/UTF-8/control/scroll fixtures, primary/alternate state subset used by the PoC, resize snapshots, bounded title/OSC, hostile streams, Unicode version and width fixtures; no authority side effect. |
| Unit/headless | `noren-app`: `RenderBackend` fake | A numbered immutable grid produces deterministic glyph batches/damage; parser errors and renderer errors stay isolated; safe exit status is visible. |
| Integration | `noren-pty`: fixed Rust helper behind `PtyBackend` | Structured argv; validated `$HOME` cwd and invalid-home failure; inherited sentinel environment with exact TERM overrides and COLUMNS/LINES removal; controlling TTY; exact binary/UTF-8 round-trip; partial reads/writes; EOF, exit/signal, blocked output, close races, descriptor ownership, child reaping. |
| Integration | Real macOS PTY kernel | Final dimensions read by the helper through `TIOCGWINSZ` after duplicate, zero-size, rapid, and final resize traces; no zero dimension reaches the PTY. |
| App integration | Fake PTY + fake renderer + replayed app events | Window lifecycle, bounded drain/redraw scheduling, input ordering, output backpressure, final resize ordering, component failure, and repeated close without an interactive window. |
| macOS smoke | Real window + real PTY + deterministic helper, then `/bin/zsh` manual check | Window appears; fixture text is visible; supported keys round-trip; resize reaches kernel; EOF/close shows status and exits cleanly. Record macOS/architecture/toolchain, not user terminal contents. |

The deterministic helper is a test-only binary target inside `noren-pty`. It
supports explicit subcommands such as `report-argv`, `echo-bytes`,
`report-winsize`, `burst`, `exit-code`, and `wait-for-eof`; it never evaluates
input as a shell command.

## Fault, security, and resource cases

- Inject spawn, read, write, resize, parser, renderer, channel-disconnect, and
  child-wait failures one at a time. After each PTY/parser/renderer/channel
  failure, enqueue a sentinel lifecycle/redraw event and require it within
  100 ms; then Close must complete teardown within 2 s.
- Fill each bounded channel before close. Cover normal blocked-read EOF/join and
  a helper descendant that retains the slave: the latter must emit
  `ReaderJoinTimeout`, detach, and exit within NFR-004 rather than hang.
- Feed truncated escape sequences, oversized OSC/title data, invalid UTF-8,
  control bytes, wide/combining sequences, and malformed font fixtures; assert
  bounds and no authority call or panic.
- Drive terminal queries that generate replies; assert exact opaque PTY bytes,
  the 4 KiB/turn and 64 KiB/s caps, and no app interpretation.
- Capture logs with unique sentinels in input, PTY output, cwd, and environment;
  no sentinel may appear.
- Run at least 100 rapid resize/input/output interleavings and preserve the
  smallest failing seed as a regression fixture.

## Performance measurements

Criterion benchmarks over the paths with a defect history — `feed_bytes`
throughput, `SshConfig::parse` (including the #137 mixed shape), renderer
frame preparation, per-frame `snapshot`, and scrollback search — live behind
the `bench-support` feature and are run with `cargo bench`; see
[benchmarks.md](benchmarks.md) for commands, comparison workflow, the
recorded reference baseline, and the report-never-gate policy. They are
deliberately outside `cargo test --workspace`.

On a recorded Apple Silicon reference Mac in `--release`:

1. Timestamp app-owned key receipt, PTY write, helper echo, terminal update, and
   presented frame for at least 100 samples; FR/NFR pass at p95 <= 100 ms.
2. Stream 100 MiB from the helper while issuing lifecycle events. Record
   throughput, maximum queue occupancy, main-loop gaps, peak RSS, and RSS after
   60 s idle. Pass only if maximum gap <= 250 ms, output occupancy <= 64 chunks
   of <= 16 KiB, command occupancy <= 256 messages, peak RSS increase <= 64 MiB,
   and post-idle increase <= 16 MiB. Do not claim a stable throughput benchmark
   from one machine.
3. Replay at least 100 final-size changes; record event-to-PTY and
   event-to-present latency, exact final kernel dimensions, and duplicate count.
4. Measure natural and forced shutdown from close request through child reap
   and worker joins. Normal paths join within 2 s; the retained-slave case must
   detach with explicit failure and process exit within the same deadline.

## CI and evidence

The first Draft PR must contain:

- a pinned toolchain and lockfile;
- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- `cargo test --workspace`;
- `python3 scripts/check_docs.py` and
  `python3 -m unittest scripts/test_check_docs.py`, each exiting zero;
- `git diff --check`;
- recorded `rustc`/`cargo` verbose versions, installed targets, runner OS and
  architecture;
- direct dependency versions/features/licenses and any unreviewed `unsafe`.

Pure/unit/fake-backend tests run on every PR. Kernel PTY and real-window smoke
tests run on a macOS runner; a test that cannot create a GUI must report a clear
skip and does not count as the manual/recorded window acceptance gate. Fuzz,
sanitizer, Linux, visual-regression, accessibility, IME, SSH, and soak suites
remain later Issues, except that corpus/fault cases above must already be
deterministic regression tests.

## Completion rule

The PoC Issue may close only when FR-001 through FR-007 map to passing evidence,
NFR-001 through NFR-008 are measured or explicitly fail, codex-lab has reviewed
the PTY I/O and resize tests, Claude Code has reviewed the spawn/input boundary,
and remaining failures are saved in the PR/Issue. A running demo without the
cleanup and resize oracles is incomplete.
