//! Instruction counts (Callgrind) and heap allocations (DHAT) for the shared
//! workloads, with Gungraun. Both are deterministic, so this is the
//! regression guard CI runs. Linux only: it runs under Valgrind.
//!
//! Each workload runs at two sizes; the difference between them is the cost
//! of the extra steps, with setup and teardown subtracted. A step that
//! allocates shows up as allocations growing with the size.
//!
//! The asking workloads carry hard limits on heap blocks, so an allocation
//! added to the path fails the run. A yield of two effects or fewer
//! allocates nothing, so `asks` (one yield per ask) and `fanout` (one per
//! round of two) take the same blocks at any size: the driver, the routine,
//! and the host's queue. The ring's count includes `async-channel`'s own
//! listeners, which are not this crate's to guard.
//!
//! `wide` and `wide_in_order`, with thousands of requests in flight answered
//! newest first and oldest first, carry a ceiling on instructions instead,
//! about 1.5 times the current count: room for compiler drift, and a failure
//! if the cost per reply grows with the number of requests in flight.
//!
//! ```sh
//! nix develop --command bench:instructions
//! ```

#![expect(
    missing_docs,
    reason = "gungraun's attribute macros generate undocumented modules"
)]

mod workloads;

use gungraun::{
    Callgrind, Dhat, DhatMetric, EventKind, LibraryBenchmarkConfig, library_benchmark,
    library_benchmark_group, main,
};
use std::hint::black_box;

/// Fail the run if the benchmark allocates more than `blocks` heap blocks.
fn at_most(blocks: u64) -> LibraryBenchmarkConfig {
    let mut dhat = Dhat::default();
    dhat.hard_limits([(DhatMetric::TotalBlocks, blocks)]);
    let mut config = LibraryBenchmarkConfig::default();
    config.tool(dhat);
    config
}

/// Fail the run if the benchmark executes more than `instructions`.
fn under(instructions: u64) -> LibraryBenchmarkConfig {
    let mut callgrind = Callgrind::default();
    callgrind.hard_limits([(EventKind::Ir, instructions)]);
    let mut config = LibraryBenchmarkConfig::default();
    config.tool(callgrind);
    config
}

#[library_benchmark]
#[bench::one(args = (1), config = at_most(5))]
#[bench::hundred(args = (100), config = at_most(5))]
fn asks(n: u64) -> u64 {
    black_box(workloads::asks(black_box(n)))
}

#[library_benchmark]
#[bench::one(args = (1), config = at_most(5))]
#[bench::hundred(args = (100), config = at_most(5))]
fn fanout(n: u64) -> u64 {
    black_box(workloads::fanout(black_box(n)))
}

#[library_benchmark]
#[bench::thousand(args = (1000), config = under(3_300_000))]
#[bench::ten_thousand(args = (10_000), config = under(36_000_000))]
fn wide(n: u64) -> u64 {
    black_box(workloads::wide(black_box(n)))
}

#[library_benchmark]
#[bench::ten_thousand(args = (10_000), config = under(36_000_000))]
fn wide_in_order(n: u64) -> u64 {
    black_box(workloads::wide_in_order(black_box(n)))
}

#[library_benchmark]
#[bench::ten_hops((2, 5))]
#[bench::thousand_hops((2, 500))]
fn ring((nodes, rounds): (usize, u64)) -> u64 {
    black_box(workloads::ring(black_box(nodes), black_box(rounds)))
}

library_benchmark_group!(
    name = driver,
    benchmarks = [asks, fanout, wide, wide_in_order, ring]
);

main!(
    config = LibraryBenchmarkConfig::default().tool(Dhat::default()),
    library_benchmark_groups = driver
);
