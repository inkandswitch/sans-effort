//! Test routines and a host vocabulary shared by the crate's tests.

use alloc::{format, string::String, vec::Vec};
use core::ops::ControlFlow;
use sans_effort::{
    boundary::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    driver::outbox::Outbox,
    reply::handle::ReplyHandle,
    run::Run,
    testing::poll_once,
};

/// Asks once (tag 1), says the answer (tag 2), finishes.
pub(crate) enum Effect {
    Ask(ReplyHandle<String>),
    Say(String),
}

#[derive(Debug, PartialEq)]
pub(crate) enum View {
    Ask(u64),
    Say(String),
}

impl HostEffect for Effect {
    type View = View;

    fn split(self) -> (View, Option<Pending>) {
        match self {
            Effect::Ask(reply) => (View::Ask(reply.id()), Some(Pending::Str(reply))),
            Effect::Say(text) => (View::Say(text), None),
        }
    }
}

impl Encode for View {
    fn encode(&self, w: &mut Writer) {
        match self {
            View::Ask(id) => {
                w.u8(1);
                w.u64(*id);
            }
            View::Say(text) => {
                w.u8(2);
                w.str(text);
            }
        }
    }
}

pub(crate) struct Echo(pub(crate) Outbox<Effect>);

impl Run for Echo {
    async fn step(&mut self) -> ControlFlow<()> {
        let answer = self.0.ask(Effect::Ask).await;
        self.0.tell(Effect::Say(answer));
        ControlFlow::Break(())
    }
}

pub(crate) struct Both(pub(crate) Outbox<Effect>);

impl Run for Both {
    async fn step(&mut self) -> ControlFlow<()> {
        let (a, b) =
            sans_effort::join::join(self.0.ask(Effect::Ask), self.0.ask(Effect::Ask)).await;
        self.0.tell(Effect::Say(format!("{a}+{b}")));
        ControlFlow::Break(())
    }
}

pub(crate) struct Impatient(pub(crate) Outbox<Effect>);

impl Run for Impatient {
    async fn step(&mut self) -> ControlFlow<()> {
        let abandoned = poll_once(self.0.ask(Effect::Ask)).await;
        drop(abandoned);

        let kept = self.0.ask(Effect::Ask).await;
        self.0.tell(Effect::Say(kept));
        ControlFlow::Break(())
    }
}

/// Polls a request, keeps it unanswered, says "done", and returns: the
/// request is dropped with the routine and closes on completion.
pub(crate) struct Holds(pub(crate) Outbox<Effect>);

impl Run for Holds {
    async fn step(&mut self) -> ControlFlow<()> {
        let _held = poll_once(self.0.ask(Effect::Ask)).await;
        self.0.tell(Effect::Say(String::from("done")));
        ControlFlow::Break(())
    }
}

/// What the byte layer emits for these views: one frame each, `FRAME_ASK`
/// for an `Ask`, `FRAME_TELL` for a `Say`.
pub(crate) fn framed(views: &[View]) -> Vec<u8> {
    let mut w = Writer::new();
    for view in views {
        w.u8(match view {
            View::Ask(_) => crate::contract::FRAME_ASK,
            View::Say(_) => crate::contract::FRAME_TELL,
        });
        w.bytes(&view.to_bytes());
    }
    w.finish()
}

/// A closed frame for `id`, as the byte layer emits it after the effects.
pub(crate) fn closed_frame(id: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(crate::contract::FRAME_CLOSED);
    w.bytes(&id.to_le_bytes());
    w.finish()
}

pub(crate) fn reply_str_record(id: u64, s: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(1);
    w.u64(id);
    w.str(s);
    w.finish()
}
