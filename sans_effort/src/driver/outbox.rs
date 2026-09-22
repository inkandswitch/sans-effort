//! Where a routine's context records what it wants the host to do.

use super::{
    awaiting::Awaiting,
    mail::Mail,
    sync::{Arc, Mutex},
};
use crate::{
    reply::{Reply, handle::ReplyHandle, value::Value},
    request::{Asked, Request},
};
use alloc::vec::Vec;

/// The shared half of a [`Driver`](super::Driver): effects go in from the
/// routine's side, replies go in from the host's, and the driver takes the
/// effects out between polls.
///
/// Cloning shares the same outbox; the driver holds one clone, the routine's
/// context the other. A reifying context serves every wait through one of
/// these: [`tell`](Self::tell) records an effect and moves on;
/// [`ask`](Self::ask) records one that carries a [`ReplyHandle`] and returns
/// the future that resolves when the host replies. The names are Akka's: a
/// `tell` expects nothing back, an `ask` expects one thing.
///
/// # Laziness
///
/// `ask` records nothing until the returned future is first polled, exactly
/// as a native future does nothing until awaited. `let a = out.ask(..);
/// drop(a)` sends nothing. This matters because the same routine also runs
/// under a native context, where `tokio::time::sleep(d)` is lazy, and the
/// two must agree.
///
/// # Requests as values
///
/// [`request`](Self::request) and [`notify`](Self::notify) are the same two
/// operations with the wait as a value implementing [`Request`], and the
/// routine's requirement stated as a `From` bound on the host's vocabulary:
/// `E: From<Asked<Lookup>>`. That is what lets a reifying context be generic
/// over `E`, and a host offer a routine less than everything.
pub struct Outbox<E> {
    /// Effects and mailbox behind one lock, not two: every operation touches
    /// one or both, and taking the guard once per operation is most of what a
    /// step costs.
    inner: Arc<Mutex<Inner<E>>>,
}

struct Inner<E> {
    effects: Vec<E>,
    mail: Mail,
}

impl<E> Outbox<E> {
    pub(super) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                effects: Vec::new(),
                mail: Mail::new(),
            })),
        }
    }

    /// Record a fire-and-forget effect.
    pub fn tell(&self, effect: E) {
        self.inner.lock().effects.push(effect);
    }

    /// Build an effect that awaits a `T`, and return the future that records
    /// it on first poll and yields the reply. `make` receives the
    /// [`ReplyHandle`] the host will answer with.
    pub fn ask<T: Reply, F: FnOnce(ReplyHandle<T>) -> E>(&self, make: F) -> Awaiting<E, T> {
        let reply: ReplyHandle<T> = self.inner.lock().mail.mint();
        Awaiting::new(reply.id(), make(reply), self.clone())
    }

    /// Ask with a request value, and let the `From` bound say whether the
    /// host can carry it.
    pub fn request<R: Request>(&self, request: R) -> Awaiting<E, R::Reply>
    where
        E: From<Asked<R>>,
    {
        self.ask(|reply| E::from(Asked { request, reply }))
    }

    /// Tell with a message value, likewise.
    pub fn notify<M>(&self, message: M)
    where
        E: From<M>,
    {
        self.tell(E::from(message));
    }

    // -- the driver's and the awaiting future's side ------------------------

    /// First poll of a request: open its slot and record its effect, under
    /// one lock.
    pub(super) fn open(&self, id: u64, effect: E) {
        let mut inner = self.inner.lock();
        inner.mail.open(id);
        inner.effects.push(effect);
    }

    /// A later poll of a request: its reply, if the host has delivered one.
    pub(super) fn collect(&self, id: u64) -> Option<Value> {
        self.inner.lock().mail.collect(id)
    }

    /// The request was dropped after its first poll.
    pub(super) fn close(&self, id: u64) {
        self.inner.lock().mail.close(id);
    }

    /// The host replied. `false` if nothing awaits the handle any more.
    pub(super) fn deliver<T>(&self, reply: ReplyHandle<T>, value: Value) -> bool {
        self.inner.lock().mail.deliver(reply, value)
    }

    /// Everything recorded since the last poll, and how many requests are
    /// still open — read together so the two agree.
    pub(super) fn drain(&self) -> (Vec<E>, usize) {
        let mut inner = self.inner.lock();
        let effects = core::mem::take(&mut inner.effects);
        let outstanding = inner.mail.outstanding();
        (effects, outstanding)
    }

    pub(super) fn take_closed(&self) -> Vec<u64> {
        self.inner.lock().mail.take_closed()
    }
}

impl<E> Clone for Outbox<E> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<E> core::fmt::Debug for Outbox<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Outbox").finish_non_exhaustive()
    }
}
