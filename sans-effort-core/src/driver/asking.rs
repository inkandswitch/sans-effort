//! A request before it is recorded.

use super::{awaiting::Awaiting, outbox::Outbox};
use crate::reply::{self, Answer, handle::ReplyHandle};
use core::{future::IntoFuture, marker::PhantomData};

/// What [`Outbox::ask`] returns: a request that has not been recorded yet.
///
/// It holds only the closure that builds the effect, and has no id. It is
/// not a future: turning it into one — `.await` does this, and so do
/// [`join`](crate::join::join), [`select`](crate::select::select), and
/// [`poll_once`](crate::testing::poll_once) when they start — mints the id,
/// builds the effect around a [`ReplyHandle`] for it, records the effect,
/// and returns an [`Awaiting`], which always has an id. The move from "no
/// id" to "an id" is that one consuming call, so it happens once and cannot
/// be undone.
///
/// Like a Rust future, an `Asking` does nothing until then: a request dropped
/// before it is awaited was never recorded and never numbered, so the ids a
/// host sees are gapless and increase in the order effects were recorded.
#[must_use = "an `Asking` does nothing until it is awaited"]
pub struct Asking<E, T, F> {
    make: F,
    outbox: Outbox<E>,
    _reply: PhantomData<fn() -> T>,
}

impl<E, T, F> Asking<E, T, F> {
    pub(super) const fn new(make: F, outbox: Outbox<E>) -> Self {
        Self {
            make,
            outbox,
            _reply: PhantomData,
        }
    }
}

impl<E, A: Answer, F: FnOnce(ReplyHandle<A>) -> E> IntoFuture for Asking<E, A, F> {
    type Output = A;
    type IntoFuture = Awaiting<E, A>;

    fn into_future(self) -> Awaiting<E, A> {
        let reply: ReplyHandle<A> = self.outbox.mint();
        let id = reply.id();
        // Built with no lock held: `make` is the caller's closure.
        let effect = (self.make)(reply);
        self.outbox.open(id, effect, reply::check::<A>);
        Awaiting::new(id, self.outbox)
    }
}

impl<E, T, F> core::fmt::Debug for Asking<E, T, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Asking").finish_non_exhaustive()
    }
}
