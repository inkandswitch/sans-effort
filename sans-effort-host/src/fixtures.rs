//! Test routines and a host vocabulary shared by the crate's tests.

use alloc::{format, string::String, vec::Vec};
use core::{future::Future, ops::ControlFlow};
use sans_effort::{
    boundary::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
    driver::outbox::Outbox,
    reply::handle::ReplyHandle,
    run::Run,
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

/// Polls a future exactly once, then hands it back — enough to make a
/// request record itself without waiting for its reply.
pub(crate) struct PollOnce<F>(pub(crate) Option<F>);

impl<F: Future + Unpin> Future for PollOnce<F> {
    type Output = F;

    fn poll(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<F> {
        #[expect(
            clippy::expect_used,
            reason = "a test fixture that is polled exactly once by construction"
        )]
        let mut inner = self.0.take().expect("polled once");
        drop(core::pin::Pin::new(&mut inner).poll(cx));
        core::task::Poll::Ready(inner)
    }
}

pub(crate) struct Impatient(pub(crate) Outbox<Effect>);

impl Run for Impatient {
    async fn step(&mut self) -> ControlFlow<()> {
        let abandoned = PollOnce(Some(self.0.ask(Effect::Ask))).await;
        drop(abandoned);

        let kept = self.0.ask(Effect::Ask).await;
        self.0.tell(Effect::Say(kept));
        ControlFlow::Break(())
    }
}

/// What the byte layer emits for these views: one frame each, `FRAME_ASK`
/// for an `Ask`, `FRAME_TELL` for a `Say`.
pub(crate) fn framed(views: &[View]) -> Vec<u8> {
    let mut w = Writer::new();
    for view in views {
        w.u8(match view {
            View::Ask(_) => crate::code::FRAME_ASK,
            View::Say(_) => crate::code::FRAME_TELL,
        });
        w.bytes(&view.to_bytes());
    }
    w.finish()
}

pub(crate) fn reply_str_record(id: u64, s: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(1);
    w.u64(id);
    w.str(s);
    w.finish()
}
