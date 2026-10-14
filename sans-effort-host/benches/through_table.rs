//! The workload both harnesses measure: one machine driven through the
//! process-wide handle table, every call a `table::resume` or `table::reply`
//! over bytes. It is the path a foreign host takes, less the FFI.
//!
//! [`drive`] creates a machine whose routine asks for a `u64` `asks` times,
//! answers each ask from the frames it yields, and frees the machine.

use core::ops::ControlFlow;
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
use std::hint::black_box;

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

/// A new machine that will ask `asks` times, not yet begun.
pub(crate) fn asker(asks: u64) -> u64 {
    table::new(move |outbox| Asker { outbox, left: asks }.run())
}

/// Run `handle` to completion through the table, answering every ask with
/// `1`, then free it. Returns how many calls it took.
pub(crate) fn drive(handle: u64) -> u64 {
    let (mut frames, mut status) = table::resume(handle).expect("begins");
    let mut calls = 1;
    while status != Status::Complete {
        let id = ask_id(&frames);
        let mut reply = Writer::with_capacity(17);
        reply.u8(2);
        reply.u64(id);
        reply.u64(1);
        (frames, status) = table::reply(handle, &reply.finish()).expect("replied");
        calls += 1;
    }
    table::free(handle).expect("freed");
    calls
}

/// The id in the one ask frame a step yields.
fn ask_id(frames: &[u8]) -> u64 {
    let mut r = Reader::new(frames);
    r.u8().expect("a frame kind");
    let mut view = Reader::new(r.bytes().expect("a payload"));
    view.u8().expect("the tag");
    view.u64().expect("the id")
}
