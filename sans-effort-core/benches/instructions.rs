//! Instruction counts (Callgrind) and heap allocations (DHAT) for the shared
//! workloads, with Gungraun. Both are deterministic, so this is the
//! regression guard CI runs. Linux only: it runs under Valgrind.
//!
//! Each workload runs at two sizes; the difference between them is the cost
//! of the extra steps, with setup and teardown subtracted. A step that
//! allocates shows up as allocations growing with the size.
//!
//! The asking workloads carry hard limits on heap blocks, so an allocation
//! added to the path fails the run. Today a yield that carries effects
//! allocates once — the `Vec` of effects handed to the host — so `asks`
//! (one yield per ask) and `fanout` (one per round of two) grow by one block
//! per step. The ring's count includes `async-channel`'s own listeners, which
//! are not this crate's to guard.
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
    Dhat, DhatMetric, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
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

#[library_benchmark]
#[bench::one(args = (1), config = at_most(5))]
#[bench::hundred(args = (100), config = at_most(104))]
fn asks(n: u64) -> u64 {
    black_box(workloads::asks(black_box(n)))
}

#[library_benchmark]
#[bench::one(args = (1), config = at_most(5))]
#[bench::hundred(args = (100), config = at_most(104))]
fn fanout(n: u64) -> u64 {
    black_box(workloads::fanout(black_box(n)))
}

#[library_benchmark]
#[bench::ten_hops((2, 5))]
#[bench::thousand_hops((2, 500))]
fn ring((nodes, rounds): (usize, u64)) -> u64 {
    black_box(workloads::ring(black_box(nodes), black_box(rounds)))
}

library_benchmark_group!(name = driver, benchmarks = [asks, fanout, ring]);

main!(
    config = LibraryBenchmarkConfig::default().tool(Dhat::default()),
    library_benchmark_groups = driver
);
