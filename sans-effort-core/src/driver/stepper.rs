//! The stepping shared by [`Driver`](super::Driver) and
//! [`LocalDriver`](super::LocalDriver): a routine's boxed future plus its
//! outbox, polled once per call. The two differ only in whether the future
//! is `Send`, so this is generic over the (unsized) future type.

use super::{
    Refused, Yield,
    mail::Delivery,
    outbox::Outbox,
    status::Status,
    sync::Arc,
    wake::{Hook, Wakeup},
};
use crate::reply::{Answer, Reply, handle::ReplyHandle};
use alloc::boxed::Box;
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Waker},
};

pub(super) struct Stepper<E, F: ?Sized> {
    /// `None` once the routine has completed.
    future: Option<Pin<Box<F>>>,
    outbox: Outbox<E>,
    status: Status,
    wakeup: Arc<Wakeup>,
    waker: Waker,
}

impl<E, F: Future<Output = ()> + ?Sized> Stepper<E, F> {
    pub(super) fn new(future: Pin<Box<F>>, outbox: Outbox<E>) -> Self {
        let (wakeup, waker) = Wakeup::new();
        Self {
            future: Some(future),
            outbox,
            status: Status::Awaiting,
            wakeup,
            waker,
        }
    }

    pub(super) fn on_wake(&self, hook: Hook) {
        self.wakeup.set_hook(hook);
    }

    pub(super) fn try_reply<A: Answer>(
        &mut self,
        reply: ReplyHandle<A>,
        value: A,
    ) -> Result<Yield<E>, Refused<A>> {
        match self.outbox.deliver(reply, value.into_wire().into_value()) {
            Delivery::Delivered => Ok(self.poll()),
            Delivery::Gone => Ok(Yield::default()),
            Delivery::Refused(reply, error) => Err(Refused { reply, error }),
        }
    }

    #[expect(
        clippy::panic,
        reason = "an answer in Rust always encodes validly; a refusal means a handle retyped from a `Pending` was replied to with bad bytes — a host bug, reported like a handle from another driver"
    )]
    pub(super) fn reply<A: Answer>(&mut self, reply: ReplyHandle<A>, value: A) -> Yield<E> {
        self.try_reply(reply, value).unwrap_or_else(|refused| {
            panic!(
                "reply to request {} refused: {}",
                refused.reply.id(),
                refused.error
            )
        })
    }

    pub(super) const fn status(&self) -> Status {
        self.status
    }

    pub(super) const fn is_finished(&self) -> bool {
        self.future.is_none()
    }

    pub(super) fn poll(&mut self) -> Yield<E> {
        let Some(future) = self.future.as_mut() else {
            return Yield::default();
        };

        self.wakeup.clear();
        let poll = future.as_mut().poll(&mut Context::from_waker(&self.waker));
        let (effects, outstanding) = self.outbox.drain();

        self.status = Status::classify(poll, outstanding);

        if poll.is_ready() {
            // Anything the routine still held is dropped with it, and closes.
            self.future = None;
        }

        Yield::new(effects, self.outbox.take_closed())
    }
}
