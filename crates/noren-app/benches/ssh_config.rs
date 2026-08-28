//! `SshConfig::parse` benchmarks — the module with two real DoS-adjacent
//! performance defects in its history.
//!
//! The `from_blocks` alias×pattern cross-product was quadratic before the
//! indexed resolution rework, and #137's mixed literal+wildcard shape took a
//! measured 72.6 s under a lifted budget before the first-character filter
//! brought it to 9.1 ms (the default-limits rejection of that file measured
//! ~20 ms). Those numbers were discovered by hand-timing; these benchmarks
//! make them a measured property instead.
//!
//! Cases, in order of how a user feels them:
//!
//! - `realistic_config` — the shape of an ordinary `~/.ssh/config` (handful
//!   of hosts, one `Host *` default block): the sidebar-load path.
//! - `mixed_1mib_fast_reject` — the exact #137 generator (14,189 alternating
//!   literal + wildcard pairs, ~1 MiB). Default limits must reject it with
//!   `ResolutionComplexityExceeded`; the time of that rejection is the DoS
//!   property. The unit suite pins the work *count*; this measures the wall
//!   cost trend.
//! - `literal_20k_1mib` — a ~1 MiB all-literal config that parses
//!   successfully (20,000 hosts): the accepted-path upper bound.
//!
//! Benchmarks report; they never assert timings. The one-shot result-shape
//! checks below are correctness pins (result kind, host count), not gates on
//! speed — if one of those changes, the benchmark's meaning changed, not the
//! machine.
//!
//! Run with: `cargo bench -p noren-app --features bench-support ssh_config`.

use std::fmt::Write;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use noren_app::ssh_config::{SshConfig, SshConfigErrorKind};

/// The ordinary-config shape from the unit suite's realistic fixture: a few
/// literal hosts with settings, one `Host *` default block.
fn realistic_config() -> String {
    r#"
# Work hosts
Host web staging
    HostName web.internal.example
    User deploy
    Port 2222

Host database
    HostName db.internal.example
    User postgres
    Port 5432

Host *
    User nobody
"#
    .to_owned()
}

/// The #137 mixed literal+wildcard generator, verbatim in shape: 14,189
/// alternating pairs filling roughly one MiB.
fn mixed_dos_1mib() -> String {
    let mut text = String::with_capacity(1024 * 1024);
    for index in 0..14_189 {
        writeln!(text, "Host literal-alias-{index:05}").expect("write to string");
        text.push_str("HostName x\n");
        writeln!(&mut text, "Host impossible-{index:05}*").expect("write to string");
        text.push_str("HostName y\n");
    }
    text
}

/// A ~1 MiB all-literal config that parses successfully: 20,000 hosts.
fn literal_20k_1mib() -> String {
    let mut text = String::with_capacity(1024 * 1024);
    for index in 0..20_000 {
        writeln!(text, "Host literal-alias-{index:05}").expect("write to string");
        text.push_str("HostName target.example\n");
    }
    text
}

fn bench_ssh_config_parse(c: &mut Criterion) {
    let realistic = realistic_config();
    let mixed = mixed_dos_1mib();
    let literal = literal_20k_1mib();

    // One-shot correctness pins (untimed): these state what each benchmark
    // measures. They are deliberately NOT timing assertions — see the module
    // docs. The wall-clock ceilings that used to live in tests were replaced
    // by operation-count assertions in #154/#158; this suite never brings
    // them back.
    let rejected = SshConfig::parse(&mixed)
        .expect_err("the #137 shape must stay rejected under default limits");
    assert_eq!(
        rejected.kind(),
        &SshConfigErrorKind::ResolutionComplexityExceeded
    );
    let accepted = SshConfig::parse(&literal).expect("large literal config parses");
    assert_eq!(accepted.hosts().len(), 20_000);

    let mut group = c.benchmark_group("ssh_config_parse");
    group.throughput(Throughput::Bytes(realistic.len() as u64));
    group.bench_function("realistic_config", |b| {
        b.iter(|| std::hint::black_box(SshConfig::parse(std::hint::black_box(&realistic))))
    });

    group.throughput(Throughput::Bytes(mixed.len() as u64));
    group.bench_function("mixed_1mib_fast_reject", |b| {
        b.iter(|| std::hint::black_box(SshConfig::parse(std::hint::black_box(&mixed))))
    });

    group.throughput(Throughput::Bytes(literal.len() as u64));
    group.bench_function("literal_20k_1mib", |b| {
        b.iter(|| std::hint::black_box(SshConfig::parse(std::hint::black_box(&literal))))
    });
    group.finish();
}

criterion_group!(benches, bench_ssh_config_parse);
criterion_main!(benches);
