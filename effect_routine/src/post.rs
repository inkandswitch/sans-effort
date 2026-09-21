//! What a routine writes into.

use crate::{
    reply::ReplyHandle,
    request::{Asked, Request},
    wire::Reply,
};
use core::future::Future;

/// An outbox, as a routine sees it.
///
/// Every wait a routine makes goes through one of these. [`tell`](Self::tell)
/// records an effect and moves on; [`ask`](Self::ask) records an effect that
/// carries a [`ReplyHandle`] and returns the future that resolves when the
/// host replies. The names are Akka's: a `tell` expects nothing back, an
/// `ask` expects one thing.
///
/// A routine is generic over `O: Post<Effect>` so that the same code runs
/// under any outbox — the [`Driver`](crate::driver::Driver)'s, or a test's.
///
/// # Laziness
///
/// `ask` records nothing until the returned future is first polled, exactly
/// as a native future does nothing until awaited. `let a = out.ask(..);
/// drop(a)` sends nothing. This matters when the same routine also runs under
/// a native context: `tokio::time::sleep(d)` is lazy, and the effect must
/// agree.
///
/// # The provided methods
///
/// [`request`](Self::request) and [`notify`](Self::notify) are the
/// _coeffects_ reading: the wait is a value implementing [`Request`], and the
/// routine states what its host must carry as a `From` bound. They are
/// spelled out from `tell` and `ask`; an outbox implements only those two.
pub trait Post<E> {
    /// `Unpin` because the concrete futures are: they hold a handle and an
    /// id, and borrow nothing of themselves. Auto traits propagate
    /// structurally, so an ordinary struct wrapping one is `Unpin` too —
    /// which is what lets a `join`, a `select`, or a hand-written combinator
    /// poll requests through `&mut` with no `Box::pin`. It stops at the first
    /// `async {}` block, which is `!Unpin` whatever it holds.
    type Awaiting<T: Reply>: Future<Output = T> + Unpin;

    /// Record a fire-and-forget effect.
    fn tell(&self, effect: E);

    /// Build an effect that awaits a `T`, and return the future that records
    /// it on first poll and yields the reply. `make` receives the
    /// [`ReplyHandle`] the host will answer with.
    fn ask<T: Reply, F: FnOnce(ReplyHandle<T>) -> E>(&self, make: F) -> Self::Awaiting<T>;

    /// Ask with a request value, and let the `From` bound say whether the
    /// host can carry it.
    fn request<R: Request>(&self, request: R) -> Self::Awaiting<R::Reply>
    where
        E: From<Asked<R>>,
    {
        self.ask(|reply| E::from(Asked { request, reply }))
    }

    /// Tell with a message value, likewise.
    fn notify<M>(&self, message: M)
    where
        E: From<M>,
    {
        self.tell(E::from(message));
    }
}
