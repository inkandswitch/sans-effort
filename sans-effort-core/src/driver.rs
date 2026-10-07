//! Resume a routine one wait at a time, with no runtime.
//!
//! In Rust you hand a routine to an executor and it polls. A Java or Python
//! host cannot poll a Rust future, so a [`Driver`] turns a routine into a
//! _steppable_ thing with sans-io's host API: [`resume`](Driver::resume)
//! runs it to its next wait — the first call begins it — and each
//! [`reply`](Driver::reply) delivers one answer and runs it to the next. Both
//! return what it yielded, a [`Yield`]: the effects it recorded, and the ids
//! of any requests it abandoned.
//!
//! An [`Outbox`] serves every wait the same way: return an
//! [`Asking`](asking::Asking) holding the closure that builds the effect. Nothing has
//! happened yet — like a Rust future, it is lazy. Awaiting it turns it into
//! an [`Awaiting`](awaiting::Awaiting): that one consuming step mints a
//! [`ReplyHandle`], builds the effect around it, records it, and opens a
//! mailbox slot; the first poll returns `Pending`; the routine suspends; the
//! driver hands the recorded effects to the host. Because ids are minted
//! there, the ids a host sees are gapless and increase in the order effects
//! were recorded.
//! The host replies through the handle, which puts the value in the slot; on
//! the next poll the future finds it and the routine continues. An `Asking`
//! dropped before it is awaited emitted nothing and was never numbered; an
//! `Awaiting` dropped before its reply closes its
//! slot, its id is reported in the yield's [`closed`](Yield::closed), and a late
//! reply is discarded.
//!
//! ```text
//!   routine                         outbox                              host
//!   out.ask(ReadLine)  ──mint──▶   handle 7
//!   .await → Pending   ──open──▶   mail[7] = empty; effects += ReadLine(7)
//!                                                     ──resume/reply─▶  [ReadLine·7]
//!                                  mail[7] = "bob"     ◀──reply(7, "bob")──
//!   .await → Ready("bob")  ◀────── take mail[7]         ◀──resume──
//! ```
//!
//! `Pending` from the routine means "a request is outstanding" — or, if none
//! is, [`Status::Idle`]: the routine is waiting on something inside the
//! process, such as a channel another routine sends on. Each driver polls
//! with a waker of its own; when whatever the routine waits on calls it, the
//! driver calls the hook set by [`on_wake`](Driver::on_wake) — once per wait
//! — and whoever set it polls again with [`resume`](Driver::resume). A host
//! layer turns that into "machine 9 can run"; with no hook, a wake is simply
//! not reported, and resuming idle machines still works.
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

pub mod asking;
pub mod awaiting;
pub mod outbox;
pub mod status;

mod mail;
mod stepper;
pub(crate) mod sync;
mod wake;

use self::{outbox::Outbox, status::Status, stepper::Stepper};
use crate::{
    boundary::codec::DecodeError,
    reply::{Answer, handle::ReplyHandle},
};
use alloc::{boxed::Box, collections::VecDeque, vec::Vec};
use core::{future::Future, pin::Pin};

/// A routine's future, boxed and `Send`: what a [`Driver`] steps.
///
/// Any `Future<Output = ()> + Send + 'static` fits; in practice it is a
/// routine's `run()`. The same type as `futures-core`'s
/// `BoxFuture<'static, ()>`, so the two interchange freely.
pub type BoxedRoutine = Pin<Box<dyn Future<Output = ()> + Send>>;

/// A routine's future, boxed, not necessarily `Send`: what a local driver
/// ([`Driver::local`]) steps.
///
/// Any `Future<Output = ()> + 'static` fits. The same type as
/// `futures-core`'s `LocalBoxFuture<'static, ()>`.
pub type LocalBoxedRoutine = Pin<Box<dyn Future<Output = ()>>>;

/// One suspended routine plus the outbox its context writes into.
///
/// The host API is sans-io's: [`resume`](Self::resume) to begin, then
/// [`reply`](Self::reply) with each handle the effects hand back, and
/// [`resume`](Self::resume) again when the routine is [`Idle`](Status::Idle)
/// and may have something new — until [`is_finished`](Self::is_finished).
///
/// `F` is the routine's future, boxed and unsized. By default it is `Send`,
/// so a `Driver` may be polled from any thread, one at a time. For a routine
/// whose future is not `Send` — one holding an `Rc`, or a foreign value tied
/// to its thread, across an `.await` — [`Driver::local`] builds a _local_
/// driver, `Driver<E, dyn Future<Output = ()>>`: the same type over a future
/// that need not be `Send`, which the compiler keeps on the thread that built
/// it.
pub struct Driver<E, F: ?Sized = dyn Future<Output = ()> + Send> {
    stepper: Stepper<E, F>,
}

impl<E> Driver<E> {
    /// Build the routine around a fresh outbox. `make` receives the outbox
    /// the routine's context should write into and returns the routine's
    /// future — typically `|outbox| Routine::new(Ctx::new(outbox)).run()`.
    ///
    /// `Send` is checked here, once, on the concrete future: this is where a
    /// context holding an `Rc` across an `.await` is rejected. This is also
    /// the one `Box::pin` in the design: an `async fn`'s state machine holds
    /// borrows across awaits and must not move between polls.
    pub fn new<Fut: Future<Output = ()> + Send + 'static, M: FnOnce(Outbox<E>) -> Fut>(
        make: M,
    ) -> Self {
        let outbox = Outbox::new();
        let future = make(outbox.clone());

        Self {
            stepper: Stepper::new(Box::pin(future), outbox),
        }
    }

    /// As [`new`](Self::new), for a routine that is already boxed — a spawned
    /// child, say — so it is not boxed twice.
    pub fn from_boxed<M: FnOnce(Outbox<E>) -> BoxedRoutine>(make: M) -> Self {
        let outbox = Outbox::new();
        let future = make(outbox.clone());

        Self {
            stepper: Stepper::new(future, outbox),
        }
    }

    /// As [`new`](Self::new), without requiring the future to be `Send`: a
    /// local driver, which is not `Send` either and so stays on this thread.
    pub fn local<Fut: Future<Output = ()> + 'static, M: FnOnce(Outbox<E>) -> Fut>(
        make: M,
    ) -> Driver<E, dyn Future<Output = ()>> {
        let outbox = Outbox::new();
        let future = make(outbox.clone());

        Driver {
            stepper: Stepper::new(Box::pin(future), outbox),
        }
    }

    /// As [`local`](Self::local), for a routine that is already boxed — a
    /// pinned child, say.
    pub fn local_boxed<M: FnOnce(Outbox<E>) -> LocalBoxedRoutine>(
        make: M,
    ) -> Driver<E, dyn Future<Output = ()>> {
        let outbox = Outbox::new();
        let future = make(outbox.clone());

        Driver {
            stepper: Stepper::new(future, outbox),
        }
    }
}

impl<E, F: Future<Output = ()> + ?Sized> Driver<E, F> {
    /// Deliver the answer a handle asked for and advance: the effects
    /// recorded before the next wait.
    ///
    /// The handle is proof that this driver minted a request for this
    /// answer; if the routine has since dropped that request — the losing
    /// arm of a [`select`](crate::select::select), say — the answer is
    /// discarded and the yield is empty, because the routine moved on without
    /// it.
    ///
    /// # Panics
    ///
    /// If the handle was minted by another driver, or if it was retyped to a
    /// wire kind (through a [`Pending`](crate::boundary::pending::Pending))
    /// and `value` does not decode as the answer awaited. Both are host bugs,
    /// reported as such rather than delivered. A host replying with wire
    /// values from outside the process uses [`try_reply`](Self::try_reply).
    pub fn reply<A: Answer>(&mut self, reply: ReplyHandle<A>, value: A) -> Yield<E> {
        self.stepper.reply(reply, value)
    }

    /// As [`reply`](Self::reply), but a value that does not decode as the
    /// answer awaited is refused rather than a panic: for a host whose
    /// replies arrive as bytes it cannot vouch for.
    ///
    /// # Errors
    ///
    /// [`Refused`], holding the handle back, if `value` is not a valid answer.
    /// The request stays open; reply again.
    ///
    /// # Panics
    ///
    /// If the handle was minted by another driver.
    pub fn try_reply<A: Answer>(
        &mut self,
        reply: ReplyHandle<A>,
        value: A,
    ) -> Result<Yield<E>, Refused<A>> {
        self.stepper.try_reply(reply, value)
    }

    /// Run the routine to its next wait without delivering anything: what it
    /// yielded. The first call begins it.
    ///
    /// After that, for an [`Idle`](Status::Idle) routine — one waiting on
    /// something in the process, such as a channel another routine sends on —
    /// once that may have changed. Harmless when nothing has: the routine finds
    /// nothing new, and the yield is empty; after completion it is always
    /// empty.
    pub fn resume(&mut self) -> Yield<E> {
        self.stepper.poll()
    }

    /// Call `hook` when the routine, suspended with nothing outstanding for
    /// the host, may be able to progress — something it waits on in the
    /// process has changed. Called at most once per poll, possibly from
    /// another thread, possibly during another machine's poll; it should only
    /// note the fact. Replaces any earlier hook.
    pub fn on_wake<H: Fn() + Send + Sync + 'static>(&self, hook: H) {
        self.stepper.on_wake(alloc::boxed::Box::new(hook));
    }

    /// What the last poll reported.
    #[must_use]
    pub const fn status(&self) -> Status {
        self.stepper.status()
    }

    /// `true` once the routine has returned.
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.stepper.is_finished()
    }
}

impl<E, F: Future<Output = ()> + ?Sized> core::fmt::Debug for Driver<E, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Driver")
            .field("status", &self.status())
            .field("finished", &self.is_finished())
            .finish_non_exhaustive()
    }
}

/// What the routine yielded on one [`resume`](Driver::resume) or
/// [`reply`](Driver::reply): the effects it recorded before its next wait,
/// and the ids of requests it abandoned along the way.
///
/// It iterates over the effects, so a host that has nothing to cancel uses it
/// like the `Vec` it replaces:
///
/// ```
/// # use sans_effort_core::driver::Yield;
/// # use std::collections::VecDeque;
/// let yielded = Yield::new(vec!["WriteLine", "ReadLine"], vec![]);
/// let queue: VecDeque<_> = yielded.into();
/// assert_eq!(queue, ["WriteLine", "ReadLine"]);
/// ```
///
/// A host whose effects cost something to perform — a timer, a network
/// request — reads [`closed`](Self::closed) and stops the work for those ids.
/// A closed id means the routine no longer needs the reply, not that the
/// effect did not happen.
#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use = "a yield holds effects to perform and ids whose work can stop"]
pub struct Yield<T> {
    effects: Vec<T>,
    closed: Vec<u64>,
}

impl<T> Yield<T> {
    /// A yield from its parts.
    pub const fn new(effects: Vec<T>, closed: Vec<u64>) -> Self {
        Self { effects, closed }
    }

    /// The effects recorded, in order.
    #[must_use]
    pub fn effects(&self) -> &[T] {
        &self.effects
    }

    /// Ids of requests the routine abandoned unanswered. A reply to one is
    /// discarded.
    #[must_use]
    pub fn closed(&self) -> &[u64] {
        &self.closed
    }

    /// `true` if there are no effects and nothing was closed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.effects.is_empty() && self.closed.is_empty()
    }

    /// The effects and the closed ids.
    #[must_use]
    pub fn into_parts(self) -> (Vec<T>, Vec<u64>) {
        (self.effects, self.closed)
    }

    /// The same yield with each effect transformed; the closed ids are kept.
    pub fn map<U, F: FnMut(T) -> U>(self, f: F) -> Yield<U> {
        Yield {
            effects: self.effects.into_iter().map(f).collect(),
            closed: self.closed,
        }
    }
}

impl<T> Default for Yield<T> {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new())
    }
}

impl<T> IntoIterator for Yield<T> {
    type Item = T;
    type IntoIter = alloc::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.effects.into_iter()
    }
}

impl<T> From<Yield<T>> for Vec<T> {
    fn from(yielded: Yield<T>) -> Self {
        yielded.effects
    }
}

impl<T> From<Yield<T>> for VecDeque<T> {
    fn from(yielded: Yield<T>) -> Self {
        yielded.effects.into()
    }
}

/// A reply that does not decode as the answer its request awaits, refused
/// before the routine could see it. The request stays open.
#[derive(thiserror::Error)]
#[error("reply to request {} refused: {error}", reply.id())]
pub struct Refused<A> {
    reply: ReplyHandle<A>,
    error: DecodeError,
}

impl<A> Refused<A> {
    /// Why the value was refused.
    #[must_use]
    pub const fn error(&self) -> &DecodeError {
        &self.error
    }

    /// The handle, to reply through again, and why the value was refused.
    pub const fn into_parts(self) -> (ReplyHandle<A>, DecodeError) {
        (self.reply, self.error)
    }
}

impl<A> core::fmt::Debug for Refused<A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Refused")
            .field("reply", &self.reply)
            .field("error", &self.error)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        boundary::{codec::DecodeError, pending::Pending},
        reply::Answer,
        step::Step,
        testing::poll_once,
    };
    use alloc::string::String;
    use core::{
        ops::ControlFlow,
        task::{Context, Poll, Waker},
    };
    use std::sync::PoisonError;
    use testresult::TestResult;

    #[derive(Debug)]
    enum Effect {
        Ask(ReplyHandle<String>),
        Say(String),
    }

    struct Echo {
        outbox: Outbox<Effect>,
        steps: u32,
    }

    impl Step for Echo {
        async fn step(&mut self) -> ControlFlow<()> {
            self.outbox.tell(Effect::Say(String::from("?")));
            let answer = self.outbox.ask(Effect::Ask).await;
            self.outbox.tell(Effect::Say(answer));
            self.steps += 1;

            if self.steps == 2 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }

    fn echo() -> Driver<Effect> {
        Driver::new(|outbox| Echo { outbox, steps: 0 }.run())
    }

    /// What was said, and the handles in the batch.
    fn split(effects: Yield<Effect>) -> (Vec<String>, Vec<ReplyHandle<String>>) {
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

    fn one(handles: Vec<ReplyHandle<String>>) -> TestResult<ReplyHandle<String>> {
        let [handle] = handles.try_into().map_err(|handles: Vec<_>| {
            format!("exactly one request in the batch, not {}", handles.len())
        })?;
        Ok(handle)
    }

    /// Whatever it holds, a yield gives it back as it was: emptiness is
    /// "no effects and no closed ids", the parts come out unchanged, `map`
    /// touches only the effects, and a `Vec` is the effects alone.
    #[test]
    fn a_yield_gives_back_what_it_holds() {
        bolero::check!()
            .with_type::<(Vec<u8>, Vec<u64>)>()
            .for_each(|(effects, closed)| {
                let made = || Yield::new(effects.clone(), closed.clone());
                assert_eq!(made().is_empty(), effects.is_empty() && closed.is_empty());
                assert_eq!(made().into_parts(), (effects.clone(), closed.clone()));

                let mapped = made().map(|e| u16::from(e) + 1);
                let expected: Vec<u16> = effects.iter().map(|e| u16::from(*e) + 1).collect();
                assert_eq!(mapped.effects(), expected.as_slice());
                assert_eq!(mapped.closed(), closed.as_slice());

                assert_eq!(Vec::from(made()), *effects);
            });
    }

    /// A yield, as the host reads it: each request by id, each saying by
    /// what was said.
    fn seen(effects: Yield<Effect>) -> Vec<String> {
        effects
            .into_iter()
            .map(|effect| match effect {
                Effect::Ask(reply) => format!("ask {}", reply.id()),
                Effect::Say(said) => format!("say {said}"),
            })
            .collect()
    }

    /// Requests kept alive past the call that built them, so none closes.
    type Kept = std::sync::Arc<std::sync::Mutex<Vec<awaiting::Awaiting<Effect, String>>>>;

    /// A request's effect built by code that records more of its own — two
    /// requests and a saying. Everything leaves in the order it was
    /// recorded, and the outer request was recorded first, when its id was
    /// minted.
    #[test]
    fn what_is_recorded_while_a_request_is_built_follows_it() {
        let kept = Kept::default();
        let mut driver = Driver::new({
            let kept = Kept::clone(&kept);
            move |outbox: Outbox<Effect>| async move {
                let inner = outbox.clone();
                outbox
                    .ask(move |reply| {
                        let mut kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
                        kept.push(inner.ask(Effect::Ask).into_future());
                        inner.tell(Effect::Say(String::from("inside")));
                        kept.push(inner.ask(Effect::Ask).into_future());
                        Effect::Ask(reply)
                    })
                    .await;
            }
        });
        assert_eq!(
            seen(driver.resume()),
            ["ask 1", "ask 2", "say inside", "ask 3"]
        );
    }

    /// A request whose effect cannot be built — its constructor panics, and
    /// the routine catches it — leaves no hole that later effects wait
    /// behind, whether it was the only one being built or another was
    /// waiting ahead of it. Its id is never seen.
    #[test]
    #[expect(clippy::panic, reason = "the effect's constructor panics on purpose")]
    fn a_request_whose_effect_panics_holds_nothing_up() {
        fn unbuildable(outbox: &Outbox<Effect>) {
            let failed = std::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
                outbox
                    .ask(|_: ReplyHandle<String>| -> Effect { panic!("no effect") })
                    .into_future()
            }));
            assert!(failed.is_err(), "the constructor panicked");
        }

        let mut alone = Driver::new(|outbox: Outbox<Effect>| async move {
            unbuildable(&outbox);
            outbox.tell(Effect::Say(String::from("after")));
            outbox.ask(Effect::Ask).await;
        });
        assert_eq!(seen(alone.resume()), ["say after", "ask 2"]);

        let mut nested = Driver::new(|outbox: Outbox<Effect>| async move {
            let inner = outbox.clone();
            outbox
                .ask(move |reply| {
                    unbuildable(&inner);
                    inner.tell(Effect::Say(String::from("after")));
                    Effect::Ask(reply)
                })
                .await;
        });
        assert_eq!(seen(nested.resume()), ["ask 1", "say after"]);
    }

    /// Says goodbye through the outbox when dropped.
    struct Farewell(Outbox<Effect>);

    impl Drop for Farewell {
        fn drop(&mut self) {
            self.0.tell(Effect::Say(String::from("goodbye")));
        }
    }

    /// A future that completes on its first poll, holding a [`Farewell`]
    /// until it is dropped — as an `async move` block holds what it captured.
    struct Holding {
        _farewell: Farewell,
    }

    impl Future for Holding {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }

    /// What a completed routine's destructors tell is part of its last
    /// yield: the future is dropped before the outbox is drained.
    #[test]
    fn what_a_completed_routine_tells_as_it_is_dropped_is_delivered() {
        let mut driver = Driver::new(|outbox| Holding {
            _farewell: Farewell(outbox),
        });
        let (said, _) = split(driver.resume());
        assert_eq!(said, ["goodbye"]);
        assert_eq!(driver.status(), Status::Complete);
    }

    /// Raced against a message: the reply arrives on the same poll as the
    /// message, `select` takes the message (it polls first), and the ask is
    /// dropped unread. It was answered, not abandoned: not closed.
    #[test]
    fn an_answered_request_dropped_unread_is_not_closed() -> TestResult {
        let (tx, rx) = async_channel::unbounded::<()>();
        let mut driver = Driver::new(move |outbox: Outbox<Effect>| async move {
            let won = crate::select::select(rx.recv(), outbox.ask(Effect::Ask)).await;
            let said = if matches!(won, crate::select::Either::Left(_)) {
                "message"
            } else {
                "reply"
            };
            outbox.tell(Effect::Say(String::from(said)));
        });
        let (_, handles) = split(driver.resume());
        let ask = one(handles)?;

        tx.try_send(())?;
        let (effects, closed) = driver.reply(ask, String::from("late")).into_parts();
        let said: Vec<String> = effects
            .into_iter()
            .filter_map(|e| match e {
                Effect::Say(s) => Some(s),
                Effect::Ask(_) => None,
            })
            .collect();
        assert_eq!(said, ["message"], "the receive is polled first, and wins");
        assert!(closed.is_empty(), "the ask had its reply: nothing to close");
        assert_eq!(driver.status(), Status::Complete);
        Ok(())
    }

    /// Answers whose decoding can fail, one per wire kind, so that what a
    /// mailbox checks — the value as delivered — is visible.
    #[derive(Debug)]
    struct NonEmpty(String);

    impl Answer for NonEmpty {
        type Wire = String;

        fn into_wire(self) -> String {
            self.0
        }

        fn from_wire(wire: String) -> Result<Self, DecodeError> {
            if wire.is_empty() {
                Err(DecodeError::UnknownTag { tag: 0 })
            } else {
                Ok(Self(wire))
            }
        }
    }

    #[derive(Debug)]
    struct Positive(u64);

    impl Answer for Positive {
        type Wire = u64;

        fn into_wire(self) -> u64 {
            self.0
        }

        fn from_wire(wire: u64) -> Result<Self, DecodeError> {
            if wire == 0 {
                Err(DecodeError::UnknownTag { tag: 0 })
            } else {
                Ok(Self(wire))
            }
        }
    }

    #[derive(Debug)]
    enum Typed {
        Text(ReplyHandle<NonEmpty>),
        Number(ReplyHandle<Positive>),
        Either(ReplyHandle<Result<String, u64>>),
        Said(String),
    }

    /// The only effect of a yield that must hold exactly one.
    fn only(effects: Yield<Typed>) -> TestResult<Typed> {
        let effects: Vec<_> = effects.into_iter().collect();
        let [effect] = effects
            .try_into()
            .map_err(|effects: Vec<_>| format!("exactly one effect, not {}", effects.len()))?;
        Ok(effect)
    }

    /// A host replying on the wire: a value that does not decode as the
    /// answer is refused, the request staying open; one that does is
    /// delivered. For a string, a number, and bytes alike.
    #[test]
    fn try_reply_delivers_what_decodes_and_refuses_the_rest() -> TestResult {
        let mut driver = Driver::new(|outbox: Outbox<Typed>| async move {
            let text = outbox.ask(Typed::Text).await;
            let number = outbox.ask(Typed::Number).await;
            let either = outbox.ask(Typed::Either).await;
            outbox.tell(Typed::Said(alloc::format!(
                "{} {} {either:?}",
                text.0,
                number.0
            )));
        });
        assert!(!driver.is_finished());

        let Typed::Text(text) = only(driver.resume())? else {
            return Err("asks for text first")?;
        };
        let Pending::Str(wire) = NonEmpty::pending(text) else {
            unreachable!("text crosses as a string");
        };
        let (wire, _) = driver
            .try_reply(wire, String::new())
            .err()
            .ok_or("empty is not a NonEmpty")?
            .into_parts();
        assert_eq!(driver.status(), Status::Awaiting);

        let Typed::Number(number) = only(driver.try_reply(wire, String::from("hi"))?)? else {
            return Err("a good string is delivered, and the routine asks for a number")?;
        };
        let Pending::U64(wire) = Positive::pending(number) else {
            unreachable!("a number crosses as a u64");
        };
        let (wire, _) = driver
            .try_reply(wire, 0)
            .err()
            .ok_or("0 is not a Positive")?
            .into_parts();

        let Typed::Either(either) = only(driver.try_reply(wire, 7)?)? else {
            return Err("a good number is delivered, and the routine asks for a result")?;
        };
        let Pending::Bytes(wire) = <Result<String, u64>>::pending(either) else {
            unreachable!("a result crosses as bytes");
        };
        let (wire, _) = driver
            .try_reply(wire, alloc::vec![9])
            .err()
            .ok_or("9 is not an encoded result")?
            .into_parts();

        let good = Ok::<String, u64>(String::from("x")).into_wire();
        let Typed::Said(said) = only(driver.try_reply(wire, good)?)? else {
            return Err("good bytes are delivered, and the routine says what it got")?;
        };
        assert_eq!(said, r#"hi 7 Ok("x")"#);
        assert!(driver.is_finished());
        Ok(())
    }

    #[test]
    fn resume_reply_reply_finished() -> TestResult {
        let mut driver = echo();

        let (said, handles) = split(driver.resume());
        assert_eq!(said, ["?"]);
        assert_eq!(driver.status(), Status::Awaiting);

        let (said, handles) = split(driver.reply(one(handles)?, String::from("a")));
        assert_eq!(said, ["a", "?"]);
        assert_eq!(driver.status(), Status::Awaiting);

        let (said, _) = split(driver.reply(one(handles)?, String::from("b")));
        assert_eq!(said, ["b"]);
        assert_eq!(driver.status(), Status::Complete);
        assert!(driver.is_finished());
        assert!(
            driver.resume().is_empty(),
            "resume after completion is a no-op"
        );
        Ok(())
    }

    #[test]
    fn driver_is_send_and_migrates() -> TestResult {
        fn assert_send<T: Send>(_: &T) {}

        let mut driver = echo();
        assert_send(&driver);

        let (_, handles) = split(driver.resume());
        let handle = one(handles)?;

        // Reply on another thread: the outbox's lock is the happens-before edge.
        let (driver, said) = std::thread::spawn(move || {
            let (said, _) = split(driver.reply(handle, String::from("far")));
            (driver, said)
        })
        .join()
        .map_err(|_| "the replying thread panicked")?;

        assert_eq!(said, ["far", "?"]);
        assert_eq!(driver.status(), Status::Awaiting);
        Ok(())
    }

    /// Three requests: one never polled (nothing recorded), one polled then
    /// dropped (recorded, then its late reply discarded), then a live one.
    struct Impatient(Outbox<Effect>);

    impl Step for Impatient {
        async fn step(&mut self) -> ControlFlow<()> {
            let never = self.0.ask(Effect::Ask);
            drop(never);

            let abandoned = poll_once(self.0.ask(Effect::Ask)).await;
            drop(abandoned);

            let kept = self.0.ask(Effect::Ask).await;
            self.0.tell(Effect::Say(kept));
            ControlFlow::Break(())
        }
    }

    #[test]
    fn unpolled_requests_emit_nothing_and_abandoned_ones_discard_late_replies() -> TestResult {
        let mut driver = Driver::new(|outbox| Impatient(outbox).run());

        let step = driver.resume();
        assert_eq!(
            step.closed(),
            [1],
            "the abandoned request is reported closed, in the yield of the call that dropped it"
        );
        let (_, handles) = split(step);
        let [abandoned, live]: [ReplyHandle<String>; 2] = handles
            .try_into()
            .map_err(|_| "the polled-then-dropped and the live request were recorded; the never-polled one was not")?;
        assert_eq!(
            (abandoned.id(), live.id()),
            (1, 2),
            "ids are minted when a request is recorded: the never-awaited one got none"
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
        Ok(())
    }

    /// Asked one way round, polled the other: ids follow the polls, so the
    /// host sees them in increasing order whatever order they were asked in.
    struct AskedThenPolledBackwards(Outbox<Effect>);

    impl Step for AskedThenPolledBackwards {
        async fn step(&mut self) -> ControlFlow<()> {
            let first_asked = self.0.ask(Effect::Ask);
            let second_asked = self.0.ask(Effect::Ask);
            let (b, a) = crate::join::join(second_asked, first_asked).await;
            self.0.tell(Effect::Say(alloc::format!("{a}{b}")));
            ControlFlow::Break(())
        }
    }

    #[test]
    fn ids_follow_poll_order_not_ask_order() {
        let mut driver = Driver::new(|outbox| AskedThenPolledBackwards(outbox).run());
        let (_, handles) = split(driver.resume());
        let ids: alloc::vec::Vec<u64> = handles.iter().map(ReplyHandle::id).collect();
        assert_eq!(
            ids,
            [1, 2],
            "the second-asked request was polled first, so it is 1"
        );
    }

    /// Two requests polled in one step record two effects in one batch; the
    /// host replies in the *other* order, and each reply yields only what the
    /// routine did on it. This is what the ids are for.
    struct FanOut(Outbox<Effect>);

    impl Step for FanOut {
        async fn step(&mut self) -> ControlFlow<()> {
            let (a, b) = crate::join::join(self.0.ask(Effect::Ask), self.0.ask(Effect::Ask)).await;
            self.0.tell(Effect::Say(alloc::format!("{a}+{b}")));
            ControlFlow::Break(())
        }
    }

    /// Whatever order the host replies in, the routine's output is the same.
    #[test]
    fn fan_out_is_order_independent() {
        bolero::check!()
            .with_type::<bool>()
            .for_each(|first_first| {
                let Ok(()) = fan_out_replied(*first_first);
            });
    }

    fn fan_out_replied(first_first: bool) -> TestResult {
        let mut driver = Driver::new(|outbox| FanOut(outbox).run());
        let (_, handles) = split(driver.resume());
        let [a, b]: [ReplyHandle<String>; 2] = handles
            .try_into()
            .map_err(|_| "both requests in the first batch")?;

        // Each handle keeps its own value; only the order of delivery varies.
        let replies = if first_first {
            [(a, "a"), (b, "b")]
        } else {
            [(b, "b"), (a, "a")]
        };
        let [(one, one_value), (other, other_value)] = replies;

        let (said, _) = split(driver.reply(one, String::from(one_value)));
        assert!(said.is_empty(), "one of two replied: nothing to say yet");
        assert_eq!(driver.status(), Status::Awaiting);

        let (said, _) = split(driver.reply(other, String::from(other_value)));
        assert_eq!(said, ["a+b"]);
        assert_eq!(driver.status(), Status::Complete);
        Ok(())
    }

    /// A handle names the driver that minted it. Replying to another driver
    /// with it is a programming error, and is reported as one rather than
    /// landing in whichever of that driver's slots shares the id.
    #[test]
    #[should_panic(expected = "minted by driver")]
    fn a_handle_replies_only_to_its_own_driver() {
        let mut a = echo();
        let mut b = echo();
        let (_, handles) = split(a.resume());
        drop(b.resume());

        // Both drivers minted id 1; the handle knows whose it is.
        let Ok(handle) = one(handles);
        drop(b.reply(handle, String::from("misrouted")));
    }

    #[test]
    fn a_wait_with_no_request_outstanding_is_idle() {
        struct Never;

        impl Future for Never {
            type Output = ();

            fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
                Poll::Pending
            }
        }

        let mut driver = Driver::<Effect>::new(|_| Never);
        assert!(driver.resume().is_empty());
        assert_eq!(driver.status(), Status::Idle);
    }

    /// A queue two routines share: the smallest in-process channel. It stores
    /// no waker; the host resumes idle machines.
    type Queue = std::sync::Arc<std::sync::Mutex<alloc::collections::VecDeque<String>>>;

    fn recv(queue: &Queue) -> impl Future<Output = String> + '_ {
        core::future::poll_fn(|_| {
            queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front()
                .map_or(Poll::Pending, Poll::Ready)
        })
    }

    /// Waits for a message, then says it.
    struct Listener {
        outbox: Outbox<Effect>,
        queue: Queue,
    }

    impl Step for Listener {
        async fn step(&mut self) -> ControlFlow<()> {
            let message = recv(&self.queue).await;
            self.outbox.tell(Effect::Say(message));
            ControlFlow::Break(())
        }
    }

    /// Asks the host for a message, then sends it.
    struct Relay {
        outbox: Outbox<Effect>,
        queue: Queue,
    }

    impl Step for Relay {
        async fn step(&mut self) -> ControlFlow<()> {
            let message = self.outbox.ask(Effect::Ask).await;
            self.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(message);
            ControlFlow::Break(())
        }
    }

    #[test]
    fn an_idle_machine_resumes_once_another_has_sent() -> TestResult {
        let queue = Queue::default();
        let mut listener = Driver::new({
            let queue = Queue::clone(&queue);
            move |outbox| Listener { outbox, queue }.run()
        });
        let mut relay = Driver::new({
            let queue = Queue::clone(&queue);
            move |outbox| Relay { outbox, queue }.run()
        });

        assert!(listener.resume().is_empty());
        assert_eq!(listener.status(), Status::Idle);
        assert!(
            listener.resume().is_empty(),
            "nothing sent yet: a spurious resume changes nothing"
        );
        assert_eq!(listener.status(), Status::Idle);

        let (_, handles) = split(relay.resume());
        assert!(relay.reply(one(handles)?, String::from("hi")).is_empty());
        assert!(relay.is_finished());

        let (said, _) = split(listener.resume());
        assert_eq!(said, ["hi"]);
        assert!(listener.is_finished());
        Ok(())
    }

    /// Holds an `Rc` across an `.await`, so its future is not `Send`: only a
    /// local driver can step it.
    struct Counted {
        outbox: Outbox<Effect>,
        seen: alloc::rc::Rc<core::cell::Cell<u32>>,
    }

    impl Step for Counted {
        async fn step(&mut self) -> ControlFlow<()> {
            let seen = alloc::rc::Rc::clone(&self.seen);
            let answer = self.outbox.ask(Effect::Ask).await;
            seen.set(seen.get() + 1);
            self.outbox
                .tell(Effect::Say(alloc::format!("{answer} {}", seen.get())));
            ControlFlow::Break(())
        }
    }

    #[test]
    fn a_local_driver_steps_a_future_that_is_not_send() -> TestResult {
        let seen = alloc::rc::Rc::new(core::cell::Cell::new(0));
        let mut driver = Driver::local({
            let seen = alloc::rc::Rc::clone(&seen);
            move |outbox| Counted { outbox, seen }.run()
        });

        let (_, handles) = split(driver.resume());
        let (said, _) = split(driver.reply(one(handles)?, String::from("hi")));
        assert_eq!(said, ["hi 1"]);
        assert!(driver.is_finished());
        assert_eq!(seen.get(), 1);
        Ok(())
    }

    /// A queue that stores the waiter's waker when empty and calls it on the
    /// next send — the part of a real channel that matters here.
    #[derive(Clone, Default)]
    struct Signal(
        std::sync::Arc<std::sync::Mutex<(alloc::collections::VecDeque<String>, Option<Waker>)>>,
    );

    impl Signal {
        fn send(&self, message: &str) {
            let waiter = {
                let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                inner.0.push_back(String::from(message));
                inner.1.take()
            };
            if let Some(waker) = waiter {
                waker.wake();
            }
        }

        fn waiter(&self) -> Option<Waker> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .1
                .clone()
        }

        fn recv(&self) -> impl Future<Output = String> + '_ {
            core::future::poll_fn(|cx| {
                let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                inner.0.pop_front().map_or_else(
                    || {
                        inner.1 = Some(cx.waker().clone());
                        Poll::Pending
                    },
                    Poll::Ready,
                )
            })
        }
    }

    /// Says every message it receives, `n` times.
    struct Hearer {
        outbox: Outbox<Effect>,
        signal: Signal,
        n: u32,
    }

    impl Step for Hearer {
        async fn step(&mut self) -> ControlFlow<()> {
            let message = self.signal.recv().await;
            self.outbox.tell(Effect::Say(message));
            self.n -= 1;
            if self.n == 0 {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }

    fn counting() -> (
        std::sync::Arc<core::sync::atomic::AtomicUsize>,
        impl Fn() + Send + Sync + 'static,
    ) {
        let count = std::sync::Arc::new(core::sync::atomic::AtomicUsize::new(0));
        let hook = {
            let count = std::sync::Arc::clone(&count);
            move || {
                count.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
            }
        };
        (count, hook)
    }

    fn woken(count: &core::sync::atomic::AtomicUsize) -> usize {
        count.load(core::sync::atomic::Ordering::SeqCst)
    }

    #[test]
    fn a_wake_is_reported_once_per_wait() -> TestResult {
        let signal = Signal::default();
        let mut hearer = Driver::new({
            let signal = signal.clone();
            move |outbox| {
                Hearer {
                    outbox,
                    signal,
                    n: 2,
                }
                .run()
            }
        });
        let (count, hook) = counting();
        hearer.on_wake(hook);

        assert!(hearer.resume().is_empty());
        assert_eq!(hearer.status(), Status::Idle);
        let waker = signal.waiter().ok_or("the hearer is waiting")?;
        waker.wake_by_ref();
        waker.wake_by_ref();
        assert_eq!(
            woken(&count),
            1,
            "the second wake before a poll is not news"
        );

        assert!(
            hearer.resume().is_empty(),
            "woken, but nothing sent: harmless"
        );
        signal.send("once");
        assert_eq!(woken(&count), 2, "after a poll, a wake is news again");
        let (said, _) = split(hearer.resume());
        assert_eq!(said, ["once"]);
        Ok(())
    }

    /// Relays what the host tells it to `signal`, during its own poll.
    struct Teller {
        outbox: Outbox<Effect>,
        signal: Signal,
    }

    impl Step for Teller {
        async fn step(&mut self) -> ControlFlow<()> {
            let message = self.outbox.ask(Effect::Ask).await;
            self.signal.send(&message);
            ControlFlow::Break(())
        }
    }

    #[test]
    fn a_send_during_another_machines_poll_wakes_the_receiver() -> TestResult {
        let signal = Signal::default();
        let mut hearer = Driver::new({
            let signal = signal.clone();
            move |outbox| {
                Hearer {
                    outbox,
                    signal,
                    n: 1,
                }
                .run()
            }
        });
        let mut teller = Driver::new({
            let signal = signal.clone();
            move |outbox| Teller { outbox, signal }.run()
        });
        let (count, hook) = counting();
        hearer.on_wake(hook);

        assert!(hearer.resume().is_empty());
        let (_, handles) = split(teller.resume());
        assert_eq!(woken(&count), 0);
        drop(teller.reply(one(handles)?, String::from("hi")));
        assert_eq!(woken(&count), 1, "woken inside the teller's poll");

        let (said, _) = split(hearer.resume());
        assert_eq!(said, ["hi"]);
        assert!(hearer.is_finished());
        Ok(())
    }

    /// Wakes itself once and suspends — a yield — then finishes.
    struct Yielder(Outbox<Effect>);

    impl Step for Yielder {
        async fn step(&mut self) -> ControlFlow<()> {
            let mut yielded = false;
            core::future::poll_fn(|cx| {
                if yielded {
                    Poll::Ready(())
                } else {
                    yielded = true;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            })
            .await;
            self.0.tell(Effect::Say(String::from("again")));
            ControlFlow::Break(())
        }
    }

    #[test]
    fn a_routine_that_wakes_itself_reports_itself() {
        let mut driver = Driver::new(|outbox| Yielder(outbox).run());
        let (count, hook) = counting();
        driver.on_wake(hook);

        assert!(driver.resume().is_empty());
        assert_eq!(driver.status(), Status::Idle);
        assert_eq!(woken(&count), 1, "a wake during its own poll is not lost");

        let (said, _) = split(driver.resume());
        assert_eq!(said, ["again"]);
        assert!(driver.is_finished());
    }
}
