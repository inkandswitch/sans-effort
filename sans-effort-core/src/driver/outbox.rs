//! Where a routine's context records what it wants the host to do.

use super::{
    Yield,
    asking::Asking,
    batch::Batch,
    mail::{Check, Delivery, Mail},
    sync::{Arc, Mutex},
};
use crate::reply::{Answer, handle::ReplyHandle, value::Value};
use alloc::vec::Vec;
use core::task::Waker;

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
/// `ask` records nothing until the returned [`Asking`] is awaited, exactly as a
/// native future does nothing until awaited. `let a = out.ask(..); drop(a)`
/// sends nothing. This matters because the same routine also runs
/// under a native context, where `tokio::time::sleep(d)` is lazy, and the
/// two must agree.
///
/// # One Vocabulary, or Any
///
/// Whoever calls `ask` names a variant of `E` — `out.ask(Effect::Lookup)` —
/// so a context written this way serves one vocabulary. A context generic
/// over _any_ vocabulary describes each wait as a request value instead, and
/// states what the host must carry as a `From` bound; that pattern, and the
/// effect traits built on it, are `sans-effort-effects`.
pub struct Outbox<E> {
    /// Effects and mailbox behind one lock, not two: every operation touches
    /// one or both, and taking the guard once per operation is most of what a
    /// step costs.
    inner: Arc<Mutex<Inner<E>>>,
}

/// Effects leave in the order they were recorded, and a request is recorded
/// when its id is minted — before its effect is built, which runs the
/// caller's code with no lock held. So minting reserves the request's place;
/// anything recorded meanwhile, by that code or another thread, waits behind
/// it in `held`; and once the effect is built, the ready front of `held`
/// moves to `effects`, which the driver drains.
///
/// Almost always nothing is recorded while an effect is built, so the one
/// request being built is kept in `building`, not `held`, which then stays
/// empty and unallocated. Invariant: `building` is set only while `held` is
/// empty; the first thing that must wait moves it into `held`.
struct Inner<E> {
    effects: Batch<E>,
    building: Option<u64>,
    held: Vec<Held<E>>,
    mail: Mail,
    /// The driver's own waker. A request polled with it — awaited directly,
    /// or through `join` or `select` — needs no waker kept: the driver polls
    /// the routine after every reply anyway.
    driver_waker: Option<Waker>,
}

/// A place in the order: an effect, or the request whose effect is being
/// built.
enum Held<E> {
    Ready(E),
    Reserved(u64),
}

impl<E> Inner<E> {
    /// Record `held` after everything already recorded: at once if nothing
    /// is being built, otherwise behind it.
    fn record(&mut self, held: Held<E>) {
        if let Some(id) = self.building.take() {
            self.held.push(Held::Reserved(id));
        }
        if !self.held.is_empty() {
            self.held.push(held);
            return;
        }
        match held {
            Held::Ready(effect) => self.effects.push(effect),
            Held::Reserved(id) => self.building = Some(id),
        }
    }

    /// Move the ready front of `held` to `effects`.
    fn release(&mut self) {
        let ready = self
            .held
            .iter()
            .position(|held| matches!(held, Held::Reserved(_)))
            .unwrap_or(self.held.len());
        for held in self.held.drain(..ready) {
            if let Held::Ready(effect) = held {
                self.effects.push(effect);
            }
        }
    }
}

impl<E> Outbox<E> {
    pub(super) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                effects: Batch::new(),
                building: None,
                held: Vec::new(),
                mail: Mail::new(),
                driver_waker: None,
            })),
        }
    }

    /// Record a fire-and-forget effect.
    pub fn tell(&self, effect: E) {
        self.inner.lock().record(Held::Ready(effect));
    }

    /// Asking for a `T`: an [`Asking`] that, when awaited, mints a
    /// [`ReplyHandle`], builds the effect around it with `make`, records it,
    /// and then yields the reply. Nothing happens until it is awaited — not
    /// even numbering.
    pub fn ask<A: Answer, F: FnOnce(ReplyHandle<A>) -> E>(&self, make: F) -> Asking<E, A, F> {
        Asking::new(make, self.clone())
    }

    // -- the driver's and the awaiting future's side ------------------------

    /// Recording a request: the next id, as a handle for its reply, with
    /// its place in the order reserved until [`open`](Self::open) or
    /// [`unreserve`](Self::unreserve).
    pub(super) fn mint<T>(&self) -> ReplyHandle<T> {
        let mut inner = self.inner.lock();
        let reply: ReplyHandle<T> = inner.mail.mint();
        inner.record(Held::Reserved(reply.id()));
        reply
    }

    /// Recording a request: open its slot and put its effect in its place,
    /// under one lock.
    pub(super) fn open(&self, id: u64, effect: E, check: Check) {
        let mut inner = self.inner.lock();
        inner.mail.open(id, check);
        if inner.building == Some(id) {
            inner.building = None;
            inner.effects.push(effect);
        } else if let Some(place) = inner
            .held
            .iter_mut()
            .find(|held| matches!(held, Held::Reserved(r) if *r == id))
        {
            *place = Held::Ready(effect);
            inner.release();
        }
    }

    /// The request's effect could not be built: give up its place, so
    /// nothing waits behind it. Its id is never seen.
    pub(super) fn unreserve(&self, id: u64) {
        let mut inner = self.inner.lock();
        if inner.building == Some(id) {
            inner.building = None;
        } else {
            inner
                .held
                .retain(|held| !matches!(held, Held::Reserved(r) if *r == id));
            inner.release();
        }
    }

    /// The driver's own waker, once, as the driver is built.
    pub(super) fn adopt(&self, waker: &Waker) {
        self.inner.lock().driver_waker = Some(waker.clone());
    }

    /// A later poll of a request: its reply, if the host has delivered one;
    /// otherwise `waker` is kept, to be woken when it does — unless it is the
    /// driver's own.
    pub(super) fn collect(&self, id: u64, waker: &Waker) -> Option<Value> {
        let mut inner = self.inner.lock();
        let own = inner
            .driver_waker
            .as_ref()
            .is_some_and(|own| own.will_wake(waker));
        inner.mail.collect(id, (!own).then_some(waker))
    }

    /// The request was dropped after it was recorded.
    pub(super) fn close(&self, id: u64) {
        self.inner.lock().mail.close(id);
    }

    /// The host replied. `false` if nothing awaits the handle any more.
    pub(super) fn deliver<T>(&self, reply: ReplyHandle<T>, value: Value) -> Delivery<T> {
        self.inner.lock().mail.deliver(reply, value)
    }

    /// Everything since the last poll — the effects recorded and the ids
    /// closed — and how many requests are still open, read under one lock so
    /// the three agree.
    pub(super) fn drain(&self) -> (Yield<E>, usize) {
        let mut inner = self.inner.lock();
        let effects = core::mem::take(&mut inner.effects);
        let closed = inner.mail.take_closed();
        let outstanding = inner.mail.outstanding();
        (Yield::new(effects, closed), outstanding)
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
