//! Instruction counts (Callgrind) and heap allocations (DHAT) for a machine
//! driven through the handle table, with Gungraun. Both are deterministic,
//! so this is the regression guard CI runs: the byte layer, the handle
//! lookup, the machine's lock, and the recording check on every call. Linux
//! only: it runs under Valgrind.
//!
//! Each workload runs at two sizes; the difference between them is the cost
//! of the extra round trips, with setup and teardown subtracted. The hard
//! limits on heap blocks encode today's counts, so an allocation added to a
//! call fails the run. A round trip takes 2 blocks (the reply record this
//! host builds and the call's output bytes) and 4 when it is recorded (the
//! log keeps its own copy of each record and outcome).
//!
//! ```sh
//! nix develop --command bench:instructions
//! ```

#![expect(
    missing_docs,
    reason = "gungraun's attribute macros generate undocumented modules"
)]
#![expect(
    clippy::expect_used,
    reason = "a bench's preconditions: every call on a live machine succeeds"
)]

mod through_table;

use gungraun::{
    Dhat, DhatMetric, LibraryBenchmarkConfig, library_benchmark, library_benchmark_group, main,
};
use sans_effort_host::record::record;
use std::hint::black_box;

/// Fail the run if the benchmark allocates more than `blocks` heap blocks.
fn at_most(blocks: u64) -> LibraryBenchmarkConfig {
    let mut dhat = Dhat::default();
    dhat.hard_limits([(DhatMetric::TotalBlocks, blocks)]);
    let mut config = LibraryBenchmarkConfig::default();
    config.tool(dhat);
    config
}

// `asks` round trips through the table, nothing recording. (Plain comments:
// Gungraun's attribute allows no doc comment on a benchmark.)
#[library_benchmark]
#[bench::one(args = (1), config = at_most(14))]
#[bench::hundred(args = (100), config = at_most(212))]
fn round_trips(asks: u64) -> u64 {
    black_box(through_table::drive(through_table::asker(black_box(asks))))
}

// The same, recorded: every call is logged, with its outcome. A session
// also takes one block for the root's taps.
#[library_benchmark]
#[bench::one(args = (1), config = at_most(21))]
#[bench::hundred(args = (100), config = at_most(422))]
fn recorded(asks: u64) -> usize {
    let handle = through_table::asker(black_box(asks));
    let recorder = record(handle);
    black_box(through_table::drive(handle));
    black_box(recorder.finish().events().len())
}

library_benchmark_group!(name = table, benchmarks = [round_trips, recorded]);

main!(
    config = LibraryBenchmarkConfig::default().tool(Dhat::default()),
    library_benchmark_groups = table
);
