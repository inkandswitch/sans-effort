//! Resume a routine one wait at a time, with no runtime.
//!
//! In Rust you hand a routine to an executor and it polls. A Java or Python
//! host cannot poll a Rust future, so a [`Driver`] turns a routine into a
//! _steppable_ thing with sans-io's host API: [`start`](Driver::start) returns
//! the effects recorded before the first wait; each
//! [`reply`](Driver::reply) delivers one answer and returns the effects
//! recorded before the next. Both return a [`Step`], which also carries the
//! ids of any requests the routine abandoned.
//!
//! An [`Outbox`] serves every wait the same way: mint a [`ReplyHandle`], build
//! the effect around it, return an [`Awaiting`](awaiting::Awaiting) future that _holds_ the
//! effect. Nothing has happened yet — like every Rust future, it is lazy. Its
//! first poll records the effect, opens a mailbox slot, and returns `Pending`;
//! the routine suspends; the driver hands the recorded effects to the host.
//! The host replies through the handle, which puts the value in the slot; on
//! the next poll the future finds it and the routine continues. A future
//! dropped before its first poll emitted nothing; one dropped after closes its
//! slot, its id is reported in the step's [`closed`](Step::closed), and a late
//! reply is discarded.
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

pub mod awaiting;
pub mod outbox;
pub mod status;
pub mod step;

mod mail;
mod sync;

use self::{outbox::Outbox, status::Status, step::Step};
use crate::reply::{Reply, handle::ReplyHandle};
use alloc::boxed::Box;
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Waker},
};

/// One suspended routine plus the outbox its context writes into.
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
    /// the routine's context should write into and returns the routine's
    /// future — typically `|outbox| Routine::new(Ctx::new(outbox)).run()`.
    ///
    /// `Send` is checked here, once, on the concrete future: this is where a
    /// context holding an `Rc` across an `.await` is rejected. This is also
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
    /// wait. Valid once; a second call is a no-op returning an empty step.
    pub fn start(&mut self) -> Step<E> {
        self.poll()
    }

    /// Deliver the value a handle asked for and advance: the effects
    /// recorded before the next wait.
    ///
    /// Infallible. The handle is proof that this driver minted a request of
    /// this type; if the routine has since dropped that request — the losing
    /// arm of a [`select`](crate::select::select), say — the value is
    /// discarded and the step is empty, because the routine moved on without
    /// it.
    ///
    /// # Panics
    ///
    /// If the handle was minted by another driver. That is a host bug, and it
    /// is reported as one rather than delivered to whichever slot of this
    /// driver shares the id.
    pub fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Step<E> {
        if self.outbox.deliver(reply, value.into_value()) {
            self.poll()
        } else {
            Step::default()
        }
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

    fn poll(&mut self) -> Step<E> {
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

impl<E> core::fmt::Debug for Driver<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Driver")
            .field("status", &self.status)
            .field("finished", &self.is_finished())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::{run::Run, testing::poll_once};
    use alloc::string::String;
    use core::{ops::ControlFlow, task::Poll};

    #[derive(Debug)]
    enum Effect {
        Ask(ReplyHandle<String>),
        Say(String),
    }

    struct Echo {
        outbox: Outbox<Effect>,
        steps: u32,
    }

    impl Run for Echo {
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
    fn split(effects: Step<Effect>) -> (Vec<String>, Vec<ReplyHandle<String>>) {
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

    /// Three requests: one never polled (nothing recorded), one polled then
    /// dropped (recorded, then its late reply discarded), then a live one.
    struct Impatient(Outbox<Effect>);

    impl Run for Impatient {
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
    fn unpolled_requests_emit_nothing_and_abandoned_ones_discard_late_replies() {
        let mut driver = Driver::new(|outbox| Impatient(outbox).run());

        let step = driver.start();
        assert_eq!(
            step.closed(),
            [2],
            "the abandoned request is reported closed, in the step that dropped it"
        );
        let (_, handles) = split(step);
        let [abandoned, live]: [ReplyHandle<String>; 2] = handles
            .try_into()
            .expect("the polled-then-dropped and the live request were recorded; the never-polled one was not");
        assert_eq!(
            (abandoned.id(), live.id()),
            (2, 3),
            "ids are minted at ask time, even for the unpolled one"
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

    /// Two requests polled in one step record two effects in one batch; the
    /// host replies in the *other* order, and each reply yields only what the
    /// routine did on it. This is what the ids are for.
    struct FanOut(Outbox<Effect>);

    impl Run for FanOut {
        async fn step(&mut self) -> ControlFlow<()> {
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
        drop(b.reply(one(handles), String::from("misrouted")));
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
