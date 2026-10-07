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

use core::ops::ControlFlow;
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use sans_effort_core::{
    boundary::{
        codec::{Encode, Reader, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    driver::{outbox::Outbox, status::Status},
    reply::handle::ReplyHandle,
    step::Step,
};
use sans_effort_host::table;
use std::{hint::black_box, thread};

/// Asks for a `u64`, over and over.
enum Effect {
    Ask(ReplyHandle<u64>),
}

/// An ask as the host sees it: tag `1`, then the id.
struct View(u64);

impl HostEffect for Effect {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        let Effect::Ask(reply) = self;
        (View(reply.id()), Some(Pending::U64(reply)))
    }
}

impl Encode for View {
    fn encode(&self, w: &mut Writer) {
        w.u8(1);
        w.u64(self.0);
    }
}

struct Asker {
    outbox: Outbox<Effect>,
    left: u64,
}

impl Step for Asker {
    async fn step(&mut self) -> ControlFlow<()> {
        if self.left == 0 {
            return ControlFlow::Break(());
        }
        self.left -= 1;
        black_box(self.outbox.ask(Effect::Ask).await);
        ControlFlow::Continue(())
    }
}

/// One machine through the table: `asks` round trips, then free it.
fn drive(asks: u64) {
    let handle = table::new(move |outbox| Asker { outbox, left: asks }.run());
    let (mut frames, mut status) = table::resume(handle).expect("begins");
    while status != Status::Complete {
        let id = ask_id(&frames);
        let mut reply = Writer::new();
        reply.u8(2);
        reply.u64(id);
        reply.u64(1);
        (frames, status) = table::reply(handle, &reply.finish()).expect("replied");
    }
    table::free(handle).expect("freed");
}

/// The id in the one ask frame a step yields.
fn ask_id(frames: &[u8]) -> u64 {
    let mut r = Reader::new(frames);
    r.u8().expect("a frame kind");
    let mut view = Reader::new(r.bytes().expect("a payload"));
    view.u8().expect("the tag");
    view.u64().expect("the id")
}

fn contention(c: &mut Criterion) {
    let asks = 1_000;
    let mut group = c.benchmark_group("table");
    for threads in [1_u64, 2, 4, 8] {
        group.throughput(Throughput::Elements(threads * asks));
        group.bench_with_input(BenchmarkId::new("threads", threads), &threads, |b, &n| {
            b.iter(|| {
                thread::scope(|scope| {
                    for _ in 0..n {
                        scope.spawn(|| drive(asks));
                    }
                });
            });
        });
    }
    group.finish();
}

criterion_group!(benches, contention);
criterion_main!(benches);
