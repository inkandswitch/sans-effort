//! The stepping shared by [`Driver`](super::Driver) and
//! [`LocalDriver`](super::LocalDriver): a routine's boxed future plus its
//! outbox, polled once per call. The two differ only in whether the future
//! is `Send`, so this is generic over the (unsized) future type.

use super::{outbox::Outbox, status::Status, step::Step};
use crate::reply::{Reply, handle::ReplyHandle};
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
}

impl<E, F: Future<Output = ()> + ?Sized> Stepper<E, F> {
    pub(super) const fn new(future: Pin<Box<F>>, outbox: Outbox<E>) -> Self {
        Self {
            future: Some(future),
            outbox,
            status: Status::Awaiting,
        }
    }

    pub(super) fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Step<E> {
        if self.outbox.deliver(reply, value.into_value()) {
            self.poll()
        } else {
            Step::default()
        }
    }

    pub(super) const fn status(&self) -> Status {
        self.status
    }

    pub(super) const fn is_finished(&self) -> bool {
        self.future.is_none()
    }

    pub(super) fn poll(&mut self) -> Step<E> {
        let Some(future) = self.future.as_mut() else {
            return Step::default();
        };

        let poll = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));
        let (effects, outstanding) = self.outbox.drain();

        self.status = Status::classify(poll, outstanding);

        if poll.is_ready() {
            // Anything the routine still held is dropped with it, and closes.
            self.future = None;
        }

        Step::new(effects, self.outbox.take_closed())
    }
}
