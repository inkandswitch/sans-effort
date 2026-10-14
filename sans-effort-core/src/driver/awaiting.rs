//! A recorded request, waiting for its reply.

use super::outbox::Outbox;
use crate::reply::{Answer, Reply};
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

/// A request that has been recorded: the future an [`Asking`](super::asking::Asking)
/// becomes. It resolves once the host has replied.
///
/// It always has an id — there is no way to build one without — so an id,
/// once minted, cannot be lost. Dropping it before the reply arrives closes
/// its mailbox slot: a late reply is discarded, and the id is reported in
/// the next step's [`closed`](super::Yield::closed). Once a reply has
/// arrived — collected or not, as when a `select` takes the other branch on
/// the same poll — the request was answered, and dropping it reports
/// nothing.
///
/// `Unpin`, structurally: it holds an id and an outbox handle and borrows
/// nothing of itself, so a `join`, a `select`, or a hand-written combinator
/// can poll it through `&mut` with no `Box::pin`.
#[must_use = "an `Awaiting` does nothing until it is awaited, and dropping it closes the request"]
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

impl<E, A: Answer> Future for Awaiting<E, A> {
    type Output = A;

    #[expect(
        clippy::panic,
        reason = "a reply that passed `Answer::check` and fails `from_wire` is a broken `Answer` impl, reported against the type"
    )]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<A> {
        match self.outbox.collect(self.id, cx.waker()) {
            Some(value) => {
                let wire = A::Wire::from_value(value).unwrap_or_else(|| {
                    unreachable!("a ReplyHandle is only minted for a slot of its kind")
                });
                Poll::Ready(A::from_wire(wire).unwrap_or_else(|error| {
                    panic!(
                        "`{}`'s `Answer::check` accepted a reply its `from_wire` refuses \
                         ({error}): the two must agree",
                        core::any::type_name::<A>()
                    )
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
