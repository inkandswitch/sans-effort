//! A recorded request, waiting for its reply.

use super::outbox::Outbox;
use crate::reply::Reply;
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

/// A request that has been recorded: the future an [`Ask`](super::ask::Ask)
/// becomes. It resolves once the host has replied.
///
/// It always has an id — there is no way to build one without — so an id,
/// once minted, cannot be lost. Dropping it before the reply arrives closes
/// its mailbox slot: a late reply is discarded, and the id is reported in
/// the next step's [`closed`](super::step::Step::closed). Once the reply has
/// been collected the slot is gone, and dropping it closes nothing.
///
/// `Unpin`, structurally: it holds an id and an outbox handle and borrows
/// nothing of itself, so a `join`, a `select`, or a hand-written combinator
/// can poll it through `&mut` with no `Box::pin`.
pub struct Awaiting<E, T> {
    id: u64,
    outbox: Outbox<E>,
    _reply: PhantomData<fn() -> T>,
}

impl<E, T> Awaiting<E, T> {
    pub(super) const fn new(id: u64, outbox: Outbox<E>) -> Self {
        Self {
            id,
            outbox,
            _reply: PhantomData,
        }
    }
}

impl<E, T: Reply> Future for Awaiting<E, T> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<T> {
        match self.outbox.collect(self.id) {
            Some(value) => {
                Poll::Ready(T::from_value(value).unwrap_or_else(|| {
                    unreachable!("a ReplyHandle<T> is only minted for a T slot")
                }))
            }
            None => Poll::Pending,
        }
    }
}

impl<E, T> Drop for Awaiting<E, T> {
    fn drop(&mut self) {
        self.outbox.close(self.id);
    }
}

impl<E, T> core::fmt::Debug for Awaiting<E, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Awaiting")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
