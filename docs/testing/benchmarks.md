# Benchmarks: how to run them, compare them, and read them

Benchmarks make performance a **measured property instead of an occasional
discovery**. This workspace twice shipped performance defects that only
surfaced when someone hand-timed them — the quadratic `SshConfig::from_blocks`
alias×pattern walk, and #137's mixed literal+wildcard resolution (72.6 s
measured under a lifted budget before the first-character filter brought it to
9.1 ms) — and twice had to convert wall-clock test assertions into operation
counts (#154, #158/#159) because a timing threshold calibrated on one machine
and one profile fails elsewhere.

Policy, in one line: **benchmarks report; they never gate.**

- No benchmark asserts a timing as pass/fail. The only assertions in bench
  files are one-shot result-shape pins (an error kind, a host count) stating
  what the case measures.
- Regression *guards* are operation-count assertions in the unit suites (see
  the `resolution_work` tests in `src/ssh_config/tests.rs`, the pattern #158
  established). A benchmark number turning red is a signal for a person, not
  for CI.
- Every number below is machine-specific. It is recorded so *this* machine's
  trend is detectable, not as a universal expectation.

## What is measured, and why these paths

| Suite | Why it is here |
| --- | --- |
| `feed_bytes` | The hot path for every byte a child program prints. Includes the `cat`-of-a-big-file shape, SGR colour churn, and mixed Latin/CJK input. |
| `ssh_config_parse` | The module with two real DoS-adjacent defects in its history; includes the verbatim #137 mixed literal+wildcard generator. |
| `renderer_frame` | The CPU half of every presented frame (`Target::new` + `glyph_vertices_for` through the shipped renderer source, `#[path]`-included the same way the frame oracle does it). |
| `snapshot` | The `TerminalEngine::snapshot()` the main loop builds per redraw; deep-clones all retained scrollback, so it grows with history. |
| `search` | Ctrl+F over a full 10,000-row scrollback — what a user feels. |

## Running

The harness is `criterion` 0.8.2 (pinned `=`, `cargo_bench_support` only).
Bench targets are behind the empty `bench-support` feature so that a normal
`cargo test --workspace` never compiles the harness (verified: identical
cold-build wall clock before/after adding it).

```sh
# everything (~2 min on the reference M4)
cargo bench --workspace --features bench-support

# one suite, e.g. after touching the parser
cargo bench -p noren-terminal --features bench-support feed_bytes
cargo bench -p noren-app --features bench-support ssh_config
```

`cargo bench` builds with the release-derived bench profile; a debug-profile
number is meaningless. Keep the machine quiet while measuring (no builds in
parallel, charger connected on laptops).

## Comparing two runs

Criterion compares automatically against the previous run in `target/criterion`
and prints a `change: [...]` line with a significance verdict. For a durable
comparison across several experiments:

```sh
# once, on the code you consider the baseline
cargo bench --workspace --features bench-support --save-baseline main

# after each change: prints each bench's delta vs that baseline
cargo bench --workspace --features bench-support --baseline main
```

Baselines live under `target/criterion` and are machine-local: never compare
your numbers against another machine's, and record the machine identity (see
below) whenever a number goes into an issue or PR.

## Recorded baseline — reference machine

Recorded 2026-08-28 by the suite's introduction commit.

- Machine: Apple M4 (arm64), 10 cores, 24 GiB RAM, macOS 26.4.1 (`Darwin …
  arm64`)
- Toolchain: rustc 1.88.0 (pinned by `rust-toolchain.toml`), criterion 0.8.2,
  bench profile (release-derived)

| Benchmark | Time | Throughput |
| --- | --- | --- |
| `feed_bytes/plain_256kib_60x160` | 424 ms | 603 KiB/s |
| `feed_bytes/plain_1mib_60x160` | 1.734 s | 590 KiB/s |
| `feed_bytes/plain_256kib_24x80` | 146 ms | 1.71 MiB/s |
| `feed_bytes/sgr_256kib_60x160` | 201 ms | 1.24 MiB/s |
| `feed_bytes/utf8_256kib_60x160` | 436 ms | 587 KiB/s |
| `ssh_config_parse/realistic_config` | 11.2 µs | 17.0 MiB/s |
| `ssh_config_parse/mixed_1mib_fast_reject` | 34.8 ms | 27.2 MiB/s |
| `ssh_config_parse/literal_20k_1mib` | 34.5 ms | 27.1 MiB/s |
| `renderer_frame/dense_full_sidebar` | 1.82 ms | — |
| `renderer_frame/idle_prompt_sidebar` | 66 µs | — |
| `snapshot/empty_scrollback_60x160` | 308 µs | — |
| `snapshot/full_scrollback_60x160` | 46.3 ms | — |
| `search/first_hit_near_end` | 9.08 ms | — |
| `search/all_hits_spread` | 9.11 ms | — |
| `search/count_no_hits` | 10.2 ms | — |

### Findings recorded at introduction

These are observations the first baseline surfaced, not fixes:

1. `feed_bytes` on a 60×160 grid runs at ~0.6 MiB/s — a 1 MiB `cat` costs
   ~1.7 s of parsing. The cost is linear (256 KiB and 1 MiB agree in MiB/s)
   and scales with grid area (24×80 is ~2.9× faster): the per-line scroll
   memmoves the whole grid. A future ring/offset-based scroll should treat
   ~0.6 MiB/s at 60×160 as the number to beat.
2. `snapshot` with a full 10,000-row scrollback costs **46.3 ms per call** on
   the per-redraw path — ~2.8× a 16.6 ms frame budget by itself, because
   `TerminalSnapshot::from_state` deep-clones every retained row each frame.
   Frame-prep proper is fine (1.82 ms dense); the snapshot is the exposure.
3. `ssh_config_parse` rejection of the #137 shape is 34.8 ms — comfortable
   against its work-count guard, and now trended rather than discovered.
