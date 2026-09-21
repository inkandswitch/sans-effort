//! Step a routine one host turn at a time, with no runtime.
//!
//! In Rust you hand a routine to an executor and it polls. A Java or Python
//! host cannot poll a Rust future, so a [`Driver`] turns a routine into a
//! _steppable_ thing with sans-io's host API: [`start`](Driver::start) returns
//! the effects recorded before the first wait; each
//! [`reply`](Driver::reply) delivers one answer and returns the effects
//! recorded before the next.
//!
//! An [`Outbox`] serves every wait the same way: mint a [`ReplyHandle`], build
//! the effect around it, return an [`Awaiting`] future that _holds_ the
//! effect. Nothing has happened yet — like every Rust future, it is lazy. Its
//! first poll records the effect, opens a mailbox slot, and returns `Pending`;
//! the routine suspends; the driver hands the recorded effects to the host.
//! The host replies through the handle, which puts the value in the slot; on
//! the next poll the future finds it and the routine continues. A future
//! dropped before its first poll emitted nothing; one dropped after closes its
//! slot, and a late reply is discarded.
//!
//! ```text
//!   routine                         outbox                              host
//!   out.ask(ReadLine)  ──mint──▶   handle 7
//!   .await → Pending   ──open──▶   mail[7] = empty; effects += ReadLine(7)
//!                                                     ──start/reply──▶  [ReadLine·7]
//!                                  mail[7] = "bob"     ◀──reply(7, "bob")──
//!   .await → Ready("bob")  ◀────── take mail[7]         ◀──resume──
//! ```
//!
//! There is no waker. `Pending` from the routine means exactly "a request is
//! outstanding" — or, if none is, [`Status::Stalled`]: the routine awaited
//! something the driver cannot wake.
//!
//! # Threading
//!
//! A `Driver` is `Send` when its effect type is, because the routine's future
//! was required to be. It may be polled from any thread, one at a time; a host
//! can keep its drivers behind a `Mutex` in a `static` and step them from a
//! pool. The price is one lock around the outbox: never contended, since the
//! host polls one driver at a time, but present to give the outbox a `Sync`
//! impl and a happens-before edge between a `reply` on one thread and the
//! poll on another. About 45 ns per step over an `Rc` design, and zero
//! allocations.
//!
//! Effects are _pull-only_ — the host calls in, the routine never calls out —
//! so no foreign value ever enters a routine, and nothing on this side is
//! `!Send`. A `!Send` runtime keeps its foreign values in the shell it hands
//! you anyway (a `#[wasm_bindgen]` struct, a `PyO3` class, a NIF resource) and
//! holds a `Send` driver beside them.

mod sync;

use self::sync::{Arc, AtomicU64, Mutex};
use crate::{
    post::Post,
    reply::ReplyHandle,
    wire::{Kind, Reply, Value},
};
use alloc::{boxed::Box, vec::Vec};
use core::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::atomic::Ordering,
    task::{Context, Poll, Waker},
};

/// One suspended routine plus the outbox it writes into.
///
/// The host API is sans-io's: [`start`](Self::start), then
/// [`reply`](Self::reply) with each handle the effects hand back, until
/// [`is_finished`](Self::is_finished).
pub struct Driver<E> {
    /// `None` once the routine has completed.
    future: Option<Pin<Box<dyn Future<Output = ()> + Send>>>,
    outbox: Outbox<E>,
    status: Status,
}

impl<E> Driver<E> {
    /// Build the routine around a fresh outbox. `make` receives the outbox
    /// the routine should write into and returns the routine's future —
    /// typically `|outbox| Routine::new(outbox).run()`.
    ///
    /// `Send` is checked here, once, on the concrete future: this is where a
    /// routine holding an `Rc` across an `.await` is rejected. This is also
    /// the one `Box::pin` in the design: an `async fn`'s state machine holds
    /// borrows across awaits and must not move between polls.
    pub fn new<F: Future<Output = ()> + Send + 'static, M: FnOnce(Outbox<E>) -> F>(
        make: M,
    ) -> Self {
        let outbox = Outbox::new();
        let future = make(outbox.clone());

        Self {
            future: Some(Box::pin(future)),
            outbox,
            status: Status::Awaiting,
        }
    }

    /// Begin: poll once and return the effects recorded before the first
    /// wait. Valid once; a second call is a no-op returning nothing.
    pub fn start(&mut self) -> Vec<E> {
        self.poll()
    }

    /// Deliver the value a handle asked for and advance: the effects
    /// recorded before the next wait.
    ///
    /// Infallible. The handle is proof that this driver minted a request of
    /// this type; if the routine has since dropped that request — the losing
    /// arm of a `select`, say — the value is discarded and nothing is
    /// returned, because the routine moved on without it.
    ///
    /// # Panics
    ///
    /// If the handle was minted by another driver. That is a host bug, and it
    /// is reported as one rather than delivered to whichever slot of this
    /// driver shares the id.
    pub fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Vec<E> {
        let delivered = self
            .outbox
            .inner
            .lock()
            .mail
            .deliver(reply, value.into_value());

        if delivered { self.poll() } else { Vec::new() }
    }

    /// Ids of requests the routine dropped unanswered since the last call —
    /// so a host keeping handles of its own can drop them too.
    pub fn closed(&mut self) -> Vec<u64> {
        core::mem::take(&mut self.outbox.inner.lock().mail.closed)
    }

    /// What the last poll reported.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }

    /// `true` once the routine has returned.
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.future.is_none()
    }

    fn poll(&mut self) -> Vec<E> {
        let Some(future) = self.future.as_mut() else {
            return Vec::new();
        };

        let poll = future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()));

        let (effects, outstanding) = {
            let mut inner = self.outbox.inner.lock();
            (
                core::mem::take(&mut inner.effects),
                inner.mail.outstanding(),
            )
        };

        self.status = Status::classify(poll, outstanding);

        if poll.is_ready() {
            self.future = None;
        }

        effects
    }
}

impl<E> core::fmt::Debug for Driver<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Driver")
            .field("status", &self.status)
            .field("finished", &self.is_finished())
            .finish_non_exhaustive()
    }
}

/// What a host needs from a driver.
///
/// [`Driver`] implements it; a host layer generic over `D: Drive<E>` can also
/// wrap one — for instrumentation, recording, or replay.
pub trait Drive<E> {
    /// Begin: the effects recorded before the first wait.
    fn start(&mut self) -> Vec<E>;

    /// Deliver a value and advance: the effects recorded before the next wait.
    fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Vec<E>;

    /// Ids of requests the routine dropped unanswered since the last call.
    fn closed(&mut self) -> Vec<u64>;

    /// What the last poll reported.
    fn status(&self) -> Status;

    /// `true` once the routine has returned.
    fn is_finished(&self) -> bool;
}

impl<E> Drive<E> for Driver<E> {
    fn start(&mut self) -> Vec<E> {
        Driver::start(self)
    }

    fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Vec<E> {
        Driver::reply(self, reply, value)
    }

    fn closed(&mut self) -> Vec<u64> {
        Driver::closed(self)
    }

    fn status(&self) -> Status {
        Driver::status(self)
    }

    fn is_finished(&self) -> bool {
        Driver::is_finished(self)
    }
}

/// What one poll reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// At least one request awaits a reply; reply and resume.
    Awaiting,
    /// The routine returned. Further replies return nothing.
    Complete,
    /// Pending with no request outstanding. Will never progress: the routine
    /// awaited something the driver cannot wake.
    Stalled,
}

impl Status {
    /// `Ready` is `Complete`; `Pending` with requests outstanding is
    /// `Awaiting`; `Pending` with none is `Stalled` — there is no waker, so
    /// nothing else could ever wake it.
    const fn classify(poll: Poll<()>, outstanding: usize) -> Self {
        match poll {
            Poll::Ready(()) => Status::Complete,
            Poll::Pending if outstanding > 0 => Status::Awaiting,
            Poll::Pending => Status::Stalled,
        }
    }
}

/// Where a routine records what it wants the host to do. Cloning shares the
/// same outbox; the [`Driver`] holds one clone, the routine the other.
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

impl<E> Outbox<E> {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                effects: Vec::new(),
                mail: Mail::new(),
            })),
        }
    }
}

impl<E> Post<E> for Outbox<E> {
    type Awaiting<T: Reply> = Awaiting<E, T>;

    fn tell(&self, effect: E) {
        self.inner.lock().effects.push(effect);
    }

    fn ask<T: Reply, F: FnOnce(ReplyHandle<T>) -> E>(&self, make: F) -> Awaiting<E, T> {
        let reply: ReplyHandle<T> = self.inner.lock().mail.mint();

        Awaiting {
            id: reply.id(),
            effect: Some(make(reply)),
            outbox: self.clone(),
            polled: false,
            _reply: PhantomData,
        }
    }
}

/// The routine's half of a request. Lazy: holds the effect until first poll,
/// then records it and opens its mailbox slot; resolves once the host has
/// replied. `Unpin`: it borrows nothing of itself.
pub struct Awaiting<E, T> {
    id: u64,
    effect: Option<E>,
    outbox: Outbox<E>,
    polled: bool,
    _reply: PhantomData<fn() -> T>,
}

/// Holds an `E` and an outbox handle, never a reference into itself: safe to
/// move between polls whatever `E` is. Without this, a non-`Unpin` effect
/// type would make every routine generic over it non-`Unpin` too.
impl<E, T> Unpin for Awaiting<E, T> {}

impl<E, T> core::fmt::Debug for Awaiting<E, T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Awaiting")
            .field("id", &self.id)
            .field("polled", &self.polled)
            .finish_non_exhaustive()
    }
}

impl<E, T: Reply> Future for Awaiting<E, T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<T> {
        if let Some(effect) = self.effect.take() {
            let mut inner = self.outbox.inner.lock();
            inner.mail.open(self.id, T::KIND);
            inner.effects.push(effect);
            drop(inner);
            self.polled = true;
            return Poll::Pending;
        }

        let collected = self.outbox.inner.lock().mail.collect(self.id);

        match collected {
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
            self.outbox.inner.lock().mail.close(self.id);
        }
    }
}

/// Every mailbox gets a number no other mailbox in the process has: the one
/// atomic the driver touches, once, at construction.
static DRIVERS: AtomicU64 = AtomicU64::new(1);

/// The mailboxes: one slot per outstanding request. A slot exists from the
/// request's first poll until the future takes its value or is dropped;
/// slots closed by a drop are remembered in `closed` until a host asks, so
/// that a host keeping its own table of handles can forget them too.
///
/// Values are stored as [`Value`] — the closed menu — not type-erased; the
/// [`ReplyHandle<T>`]'s type says which variant to expect back. The menu is
/// the wire's: every foreign host replies across an ABI that carries exactly
/// these kinds, and the mailbox mirrors it.
struct Mail {
    driver: u64,
    /// Open slots by id. A `Vec` scanned linearly: a routine has one or two
    /// requests in flight, and a tree or a hash costs more than two compares.
    /// Wide fan-out would want a sorted `Vec` with binary search.
    slots: Vec<Slot>,
    closed: Vec<u64>,
    next: u64,
}

struct Slot {
    id: u64,
    kind: Kind,
    value: Option<Value>,
}

impl Mail {
    fn new() -> Self {
        Self {
            driver: DRIVERS.fetch_add(1, Ordering::Relaxed),
            slots: Vec::new(),
            closed: Vec::new(),
            next: 1,
        }
    }

    /// A fresh handle. Its slot is not open yet: that happens on first poll.
    const fn mint<T>(&mut self) -> ReplyHandle<T> {
        let id = self.next;
        self.next += 1;
        ReplyHandle::mint(self.driver, id)
    }

    fn open(&mut self, id: u64, kind: Kind) {
        self.slots.push(Slot {
            id,
            kind,
            value: None,
        });
    }

    fn position(&self, id: u64) -> Option<usize> {
        self.slots.iter().position(|slot| slot.id == id)
    }

    /// `false` if no request with that id is waiting: it was already
    /// answered or its future was dropped.
    fn deliver<T>(&mut self, reply: ReplyHandle<T>, value: Value) -> bool {
        let (driver, id) = reply.into_parts();
        assert_eq!(
            driver, self.driver,
            "ReplyHandle({driver}/{id}) was minted by driver {driver} and replied to driver {}",
            self.driver
        );

        match self.position(id).and_then(|at| self.slots.get_mut(at)) {
            Some(slot) => {
                debug_assert_eq!(
                    slot.kind,
                    value.kind(),
                    "ReplyHandle({driver}/{id}) opened a {:?} slot; a {:?} was delivered",
                    slot.kind,
                    value.kind()
                );
                slot.value = Some(value);
                true
            }
            None => false,
        }
    }

    /// `Some(value)` closes the slot; `None` leaves it waiting.
    fn collect(&mut self, id: u64) -> Option<Value> {
        let at = self.position(id)?;
        if self.slots.get(at)?.value.is_some() {
            self.slots.swap_remove(at).value
        } else {
            None
        }
    }

    /// The routine dropped a polled request. Its slot goes, and its id is
    /// kept for [`Drive::closed`].
    fn close(&mut self, id: u64) {
        if let Some(at) = self.position(id) {
            self.slots.swap_remove(at);
            self.closed.push(id);
        }
    }

    const fn outstanding(&self) -> usize {
        self.slots.len()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::run::Run;
    use alloc::string::String;
    use core::ops::ControlFlow;

    #[derive(Debug)]
    enum Effect {
        Ask(ReplyHandle<String>),
        Say(String),
    }

    struct Echo {
        outbox: Outbox<Effect>,
        turns: u32,
    }

    impl Run for Echo {
        async fn turn(&mut self) -> ControlFlow<()> {
            self.outbox.tell(Effect::Say(String::from("?")));
            let answer = self.outbox.ask(Effect::Ask).await;
            self.outbox.tell(Effect::Say(answer));
            self.turns += 1;

            if self.turns == 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }

    fn echo() -> Driver<Effect> {
        Driver::new(|outbox| Echo { outbox, turns: 0 }.run())
    }

    /// What was said, and the handles in the batch.
    fn split(effects: Vec<Effect>) -> (Vec<String>, Vec<ReplyHandle<String>>) {
        let mut said = Vec::new();
        let mut handles = Vec::new();

        for effect in effects {
            match effect {
                Effect::Ask(r) => handles.push(r),
                Effect::Say(s) => said.push(s),
            }
        }

        (said, handles)
    }

    fn one(mut handles: Vec<ReplyHandle<String>>) -> ReplyHandle<String> {
        assert_eq!(handles.len(), 1, "exactly one request in the batch");
        handles.pop().expect("one")
    }

    #[test]
    fn start_reply_reply_finished() {
        let mut driver = echo();

        let (said, handles) = split(driver.start());
        assert_eq!(said, ["?"]);
        assert_eq!(driver.status(), Status::Awaiting);

        let (said, handles) = split(driver.reply(one(handles), String::from("a")));
        assert_eq!(said, ["a", "?"]);
        assert_eq!(driver.status(), Status::Awaiting);

        let (said, _) = split(driver.reply(one(handles), String::from("b")));
        assert_eq!(said, ["b"]);
        assert_eq!(driver.status(), Status::Complete);
        assert!(driver.is_finished());
        assert!(
            driver.start().is_empty(),
            "start after completion is a no-op"
        );
    }

    #[test]
    fn driver_is_send_and_migrates() {
        fn assert_send<T: Send>(_: &T) {}

        let mut driver = echo();
        assert_send(&driver);

        let (_, handles) = split(driver.start());
        let handle = one(handles);

        // Reply on another thread: the outbox's lock is the happens-before edge.
        let (driver, said) = std::thread::spawn(move || {
            let (said, _) = split(driver.reply(handle, String::from("far")));
            (driver, said)
        })
        .join()
        .expect("thread");

        assert_eq!(said, ["far", "?"]);
        assert_eq!(driver.status(), Status::Awaiting);
    }

    /// Polls a future exactly once, then hands it back — enough to make a
    /// request record itself without waiting for its reply.
    struct PollOnce<F>(Option<F>);

    impl<F: Future + Unpin> Future for PollOnce<F> {
        type Output = F;

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F> {
            let mut inner = self.0.take().expect("polled once");
            drop(Pin::new(&mut inner).poll(cx));
            Poll::Ready(inner)
        }
    }

    /// Three requests: one never polled (nothing recorded), one polled then
    /// dropped (recorded, then its late reply discarded), then a live one.
    struct Impatient(Outbox<Effect>);

    impl Run for Impatient {
        async fn turn(&mut self) -> ControlFlow<()> {
            let never = self.0.ask(Effect::Ask);
            drop(never);

            let abandoned = PollOnce(Some(self.0.ask(Effect::Ask))).await;
            drop(abandoned);

            let kept = self.0.ask(Effect::Ask).await;
            self.0.tell(Effect::Say(kept));
            ControlFlow::Break(())
        }
    }

    #[test]
    fn unpolled_requests_emit_nothing_and_abandoned_ones_discard_late_replies() {
        let mut driver = Driver::new(|outbox| Impatient(outbox).run());

        let (_, handles) = split(driver.start());
        let [abandoned, live]: [ReplyHandle<String>; 2] = handles
            .try_into()
            .expect("the polled-then-dropped and the live request were recorded; the never-polled one was not");
        assert_eq!(
            (abandoned.id(), live.id()),
            (2, 3),
            "ids are minted at ask time, even for the unpolled one"
        );
        assert_eq!(
            driver.closed(),
            [2],
            "the abandoned request is reported closed"
        );

        assert!(driver.reply(abandoned, String::from("lost")).is_empty());
        assert_eq!(
            driver.status(),
            Status::Awaiting,
            "still waiting on the live one"
        );

        let (said, _) = split(driver.reply(live, String::from("kept")));
        assert_eq!(said, ["kept"]);
        assert!(driver.is_finished());
    }

    /// Two requests polled in one turn record two effects in one batch; the
    /// host replies in the *other* order, and each reply yields only what the
    /// routine did on it. This is what the ids are for.
    struct FanOut(Outbox<Effect>);

    impl Run for FanOut {
        async fn turn(&mut self) -> ControlFlow<()> {
            let (a, b) = crate::join::join(self.0.ask(Effect::Ask), self.0.ask(Effect::Ask)).await;
            self.0.tell(Effect::Say(alloc::format!("{a}+{b}")));
            ControlFlow::Break(())
        }
    }

    #[test]
    fn fan_out_replies_in_any_order() {
        let mut driver = Driver::new(|outbox| FanOut(outbox).run());

        let (_, handles) = split(driver.start());
        let [first, second]: [ReplyHandle<String>; 2] =
            handles.try_into().expect("two requests in one batch");

        let (said, _) = split(driver.reply(second, String::from("b")));
        assert!(said.is_empty(), "one of two replied: nothing to say yet");
        assert_eq!(driver.status(), Status::Awaiting);

        let (said, _) = split(driver.reply(first, String::from("a")));
        assert_eq!(said, ["a+b"]);
        assert_eq!(driver.status(), Status::Complete);
    }

    /// Whatever order the host replies in, the routine's output is the same.
    #[test]
    fn fan_out_is_order_independent() {
        bolero::check!()
            .with_type::<bool>()
            .for_each(|first_first| {
                let mut driver = Driver::new(|outbox| FanOut(outbox).run());
                let (_, handles) = split(driver.start());
                let [a, b]: [ReplyHandle<String>; 2] = handles.try_into().expect("two");

                // Each handle keeps its own value; only the order of delivery varies.
                let replies = if *first_first {
                    [(a, "a"), (b, "b")]
                } else {
                    [(b, "b"), (a, "a")]
                };
                let mut said = Vec::new();
                for (handle, value) in replies {
                    said.extend(split(driver.reply(handle, String::from(value))).0);
                }

                assert_eq!(said, ["a+b"]);
                assert_eq!(driver.status(), Status::Complete);
            });
    }

    /// A handle names the driver that minted it. Replying to another driver
    /// with it is a programming error, and is reported as one rather than
    /// landing in whichever of that driver's slots shares the id.
    #[test]
    #[should_panic(expected = "minted by driver")]
    fn a_handle_replies_only_to_its_own_driver() {
        let mut a = echo();
        let mut b = echo();
        let (_, handles) = split(a.start());
        drop(b.start());

        // Both drivers minted id 1; the handle knows whose it is.
        b.reply(one(handles), String::from("misrouted"));
    }

    #[test]
    fn foreign_wait_is_stalled() {
        struct Never;

        impl Future for Never {
            type Output = ();

            fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
                Poll::Pending
            }
        }

        let mut driver = Driver::<Effect>::new(|_| Never);
        assert!(driver.start().is_empty());
        assert_eq!(driver.status(), Status::Stalled);
    }
}
