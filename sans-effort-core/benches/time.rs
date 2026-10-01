//! Wall-clock time for the shared workloads, with criterion: for local A/B
//! comparisons against a saved baseline, and for numbers quoted in docs. Not
//! run in CI — time is noisy there; `instructions` is the regression guard.
//!
//! ```sh
//! cargo bench -p sans-effort-core --bench time -- --save-baseline before
//! cargo bench -p sans-effort-core --bench time -- --baseline before
//! ```

mod workloads;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::hint::black_box;

fn asks(c: &mut Criterion) {
    let mut group = c.benchmark_group("asks");
    for n in [1_u64, 100, 10_000] {
        group.throughput(Throughput::Elements(n));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| workloads::asks(black_box(n)));
        });
    }
    group.finish();
}

fn fanout(c: &mut Criterion) {
    let mut group = c.benchmark_group("fanout");
    for n in [1_u64, 100, 10_000] {
        group.throughput(Throughput::Elements(2 * n));
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, &n| {
            b.iter(|| workloads::fanout(black_box(n)));
        });
    }
    group.finish();
}

fn ring(c: &mut Criterion) {
    let mut group = c.benchmark_group("ring");
    for (nodes, rounds) in [(2_usize, 500_u64), (16, 250)] {
        let hops = u64::try_from(nodes).unwrap_or(u64::MAX) * rounds;
        group.throughput(Throughput::Elements(hops));
        group.bench_with_input(
            BenchmarkId::new("nodes", nodes),
            &(nodes, rounds),
            |b, &(nodes, rounds)| b.iter(|| workloads::ring(black_box(nodes), black_box(rounds))),
        );
    }
    group.finish();
}

criterion_group!(benches, asks, fanout, ring);
criterion_main!(benches);
