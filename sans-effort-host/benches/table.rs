//! Whether the process-wide handle table contends under a parallel host.
//!
//! `threads` threads each drive their own machine through the table — `asks`
//! ask–reply round trips, every call a `table::resume` or `table::reply` — as
//! a parallel host with one machine per worker would. The machines share
//! nothing but the table, so throughput that fails to grow with the thread
//! count is the table's lock.
//!
//! ```sh
//! cargo bench -p sans-effort-host --bench table
//! ```

#![expect(
    clippy::expect_used,
    reason = "a bench's preconditions: every call on a live machine succeeds"
)]

mod through_table;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use std::thread;

fn contention(c: &mut Criterion) {
    let asks = 1_000;
    let mut group = c.benchmark_group("table");
    for threads in [1_u64, 2, 4, 8] {
        group.throughput(Throughput::Elements(threads * asks));
        group.bench_with_input(BenchmarkId::new("threads", threads), &threads, |b, &n| {
            b.iter(|| {
                thread::scope(|scope| {
                    for _ in 0..n {
                        scope.spawn(|| through_table::drive(through_table::asker(asks)));
                    }
                });
            });
        });
    }
    group.finish();
}

criterion_group!(benches, contention);
criterion_main!(benches);
