//! The routine's half of a request.

use super::outbox::Outbox;
use crate::reply::Reply;
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

/// What [`Outbox::ask`] returns: a future that records its effect on first
/// poll and resolves once the host has replied.
///
/// Lazy: holds the effect until first poll, then records it and opens its
/// mailbox slot. `Unpin`: it holds an id and an outbox handle and borrows
/// nothing of itself, so a `join`, a `select`, or a hand-written combinator
/// can poll it through `&mut` with no `Box::pin`. (Auto traits propagate
/// structurally, so a plain struct wrapping one is `Unpin` too; the chain
/// stops at the first `async {}` block, which is `!Unpin` whatever it holds.)
pub struct Awaiting<E, T> {
    id: u64,
    effect: Option<E>,
    outbox: Outbox<E>,
    polled: bool,
    _reply: PhantomData<fn() -> T>,
}

impl<E, T> Awaiting<E, T> {
    pub(super) const fn new(id: u64, effect: E, outbox: Outbox<E>) -> Self {
        Self {
            id,
            effect: Some(effect),
            outbox,
            polled: false,
            _reply: PhantomData,
        }
    }

    /// The request id this future is waiting on.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }
}

/// Holds an `E` and an outbox handle, never a reference into itself: safe to
/// move between polls whatever `E` is. Without this, a non-`Unpin` effect
/// type would make every context generic over it non-`Unpin` too.
impl<E, T> Unpin for Awaiting<E, T> {}

impl<E, T: Reply> Future for Awaiting<E, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<T> {
        if let Some(effect) = self.effect.take() {
            self.outbox.open(self.id, effect);
            self.polled = true;
            return Poll::Pending;
        }

        match self.outbox.collect(self.id) {
            Some(value) => {
                self.polled = false;
                Poll::Ready(T::from_value(value).unwrap_or_else(|| {
                    unreachable!("a ReplyHandle<T> is only minted for a T slot")
                }))
            }
            None => Poll::Pending,
        }
    }
}

/// Dropping a polled-but-unanswered request closes its slot, so a late reply
/// is discarded rather than kept forever. Dropping an unpolled one is a
/// no-op: nothing was ever recorded.
impl<E, T> Drop for Awaiting<E, T> {
    fn drop(&mut self) {
        if self.polled {
            self.outbox.close(self.id);
        }
    }
}

impl<E, T> core::fmt::Debug for Awaiting<E, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Awaiting")
            .field("id", &self.id)
            .field("polled", &self.polled)
            .finish_non_exhaustive()
    }
}
