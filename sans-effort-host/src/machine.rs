//! The typed layer over a driver. The byte layer over that is
//! [`Encoded`](crate::encoded::Encoded).

use crate::{error::Error, status::Status};
use alloc::{string::String, vec::Vec};
use core::{future::Future, marker::PhantomData};
use sans_effort::{
    boundary::{host_effect::HostEffect, pending::Pending},
    driver::{
        BoxedRoutine, Driver, LocalDriver, Yield, outbox::Outbox, status::Status as DriveStatus,
    },
    reply::{Reply, handle::ReplyHandle},
};

/// What a [`Machine`] steps: a [`Driver`], or a [`LocalDriver`] for a routine
/// whose future is not `Send`. Sealed: the two are the only drivers.
pub trait Drive<E>: sealed::Sealed {
    /// Deliver a reply and poll.
    fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Yield<E>;
    /// Poll without delivering anything.
    fn resume(&mut self) -> Yield<E>;
    /// What the last poll reported.
    fn status(&self) -> DriveStatus;
    /// `true` once the routine has returned.
    fn is_finished(&self) -> bool;
    /// Call `hook` when the routine may be able to progress without a reply.
    fn on_wake<H: Fn() + Send + Sync + 'static>(&self, hook: H);
}

mod sealed {
    pub trait Sealed {}
    impl<E> Sealed for super::Driver<E> {}
    impl<E> Sealed for super::LocalDriver<E> {}
}

impl<E> Drive<E> for Driver<E> {
    fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Yield<E> {
        Driver::reply(self, reply, value)
    }

    fn resume(&mut self) -> Yield<E> {
        Driver::resume(self)
    }

    fn status(&self) -> DriveStatus {
        Driver::status(self)
    }

    fn is_finished(&self) -> bool {
        Driver::is_finished(self)
    }

    fn on_wake<H: Fn() + Send + Sync + 'static>(&self, hook: H) {
        Driver::on_wake(self, hook);
    }
}

impl<E> Drive<E> for LocalDriver<E> {
    fn reply<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Yield<E> {
        LocalDriver::reply(self, reply, value)
    }

    fn resume(&mut self) -> Yield<E> {
        LocalDriver::resume(self)
    }

    fn status(&self) -> DriveStatus {
        LocalDriver::status(self)
    }

    fn is_finished(&self) -> bool {
        LocalDriver::is_finished(self)
    }

    fn on_wake<H: Fn() + Send + Sync + 'static>(&self, hook: H) {
        LocalDriver::on_wake(self, hook);
    }
}

/// A [`Machine`] over a [`LocalDriver`]: for a routine whose future is not
/// `Send`, polled only on the thread that built it.
pub type LocalMachine<E> = Machine<E, LocalDriver<E>>;

/// A driver plus the requests it has outstanding, keyed by id: the typed
/// layer.
///
/// The type check a Rust host gets from [`ReplyHandle<T>`] at compile time
/// happens here at run time, against the kind the foreign host chose. The
/// `pending` table is this layer's wallet of capabilities: it holds the
/// handles the effects carried out, so that an `(id, value)` from across the
/// boundary can be turned back into the typed, infallible
/// [`Driver::reply`].
pub struct Machine<E, D = Driver<E>> {
    driver: D,
    /// Outstanding requests by id. A `Vec` scanned linearly, not a map: a
    /// routine has one or two requests in flight, and hashing a `u64` costs
    /// more than looking at two entries. Wide fan-out would want a sorted
    /// `Vec` with binary search.
    pending: Vec<(u64, Pending)>,
    /// The highest request id shown to the host. Ids are issued per driver,
    /// increasing, so an unmatched id at or below this was issued and is
    /// stale; one above it never existed.
    highest_shown: u64,
    _effect: PhantomData<E>,
}

impl<E: HostEffect> Machine<E> {
    /// Build a driver around `make` and wrap it: `Machine::new(Driver::new(make))`.
    /// Nothing runs until the first [`resume`](Self::resume).
    ///
    /// `make` receives the outbox the routine's context should write into and
    /// returns the routine's _future_ — `|outbox| Greeter::new(Ctx::new(outbox)).run()`
    /// — not the routine. The `.run()` cannot be hidden here: `Driver::new`
    /// requires the future to be `Send`, and only where the routine's type is
    /// concrete can the compiler decide that. A constructor generic over
    /// `P: Step` would have no way to state `P::run(): Send` on stable Rust.
    pub fn from_routine<F: Future<Output = ()> + Send + 'static, M: FnOnce(Outbox<E>) -> F>(
        make: M,
    ) -> Self {
        Self::new(Driver::new(make))
    }

    /// As [`from_routine`](Self::from_routine), for a routine already boxed.
    pub fn from_boxed<M: FnOnce(Outbox<E>) -> BoxedRoutine>(make: M) -> Self {
        Self::new(Driver::from_boxed(make))
    }
}

impl<E: HostEffect, D: Drive<E>> Machine<E, D> {
    /// Wrap a driver. Nothing has been polled yet.
    #[must_use]
    pub const fn new(driver: D) -> Self {
        Self {
            driver,
            pending: Vec::new(),
            highest_shown: 0,
            _effect: PhantomData,
        }
    }

    /// Reply to the request with this id: the effects recorded before the
    /// next wait.
    ///
    /// `T` is the kind the host chose; the check that it matches what the
    /// routine asked for happens here, at run time, and is the one place in
    /// the stack where a reply can be refused.
    ///
    /// # Errors
    ///
    /// [`Error::Finished`] if the routine has completed; [`Error::BadInput`]
    /// if nothing awaits `id`; [`Error::WrongKind`] if `id` awaits another
    /// kind — the handle is kept, so the host may retry with the right one.
    pub fn reply<T: Reply>(&mut self, id: u64, value: T) -> Result<Yield<E::View>, Error> {
        self.reply_shown(id, value).map(views)
    }

    /// As [`reply`](Self::reply), marking which effects await a reply.
    pub(crate) fn reply_shown<T: Reply>(
        &mut self,
        id: u64,
        value: T,
    ) -> Result<Yield<Shown<E::View>>, Error> {
        match T::from_pending(self.take(id)?) {
            Ok(reply) => Ok(self.deliver(reply, value)),
            Err(pending) => {
                let expected = pending.kind();
                self.pending.push((id, pending));
                Err(Error::WrongKind {
                    id,
                    expected,
                    got: value.into_value().kind(),
                })
            }
        }
    }

    /// Run the routine to its next wait without delivering anything: what it
    /// yielded. The first call begins it; after that, for an
    /// [`Idle`](Status::Idle) routine, once something it waits on in the
    /// process may have changed — harmless when nothing has.
    ///
    /// # Errors
    ///
    /// [`Error::Finished`] if the routine has completed.
    pub fn resume(&mut self) -> Result<Yield<E::View>, Error> {
        self.resume_shown().map(views)
    }

    /// As [`resume`](Self::resume), marking which effects await a reply.
    pub(crate) fn resume_shown(&mut self) -> Result<Yield<Shown<E::View>>, Error> {
        if self.driver.is_finished() {
            return Err(Error::Finished);
        }

        let effects = self.driver.resume();
        Ok(self.show(effects))
    }

    /// [`reply`](Self::reply) with a `str`, for bindings that cannot call a
    /// generic method (`PyO3`, Rustler).
    ///
    /// # Errors
    ///
    /// As [`reply`](Self::reply).
    pub fn reply_str(&mut self, id: u64, value: String) -> Result<Yield<E::View>, Error> {
        self.reply(id, value)
    }

    /// [`reply`](Self::reply) with a `u64`, for bindings that cannot call a
    /// generic method.
    ///
    /// # Errors
    ///
    /// As [`reply`](Self::reply).
    pub fn reply_u64(&mut self, id: u64, value: u64) -> Result<Yield<E::View>, Error> {
        self.reply(id, value)
    }

    /// [`reply`](Self::reply) with `unit`, for bindings that cannot call a
    /// generic method.
    ///
    /// # Errors
    ///
    /// As [`reply`](Self::reply).
    pub fn reply_unit(&mut self, id: u64) -> Result<Yield<E::View>, Error> {
        self.reply(id, ())
    }

    /// [`reply`](Self::reply) with `bytes`, for bindings that cannot call a
    /// generic method.
    ///
    /// # Errors
    ///
    /// As [`reply`](Self::reply).
    pub fn reply_bytes(&mut self, id: u64, value: Vec<u8>) -> Result<Yield<E::View>, Error> {
        self.reply(id, value)
    }

    /// Call `hook` when the routine, `Idle`, may be able to progress: once
    /// per wait, possibly from another thread, possibly during another
    /// machine's poll. See [`Driver::on_wake`].
    pub fn on_wake<H: Fn() + Send + Sync + 'static>(&self, hook: H) {
        self.driver.on_wake(hook);
    }

    /// What the last poll reported.
    #[must_use]
    pub fn status(&self) -> Status {
        self.driver.status().into()
    }

    /// `true` once the routine has returned.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.driver.is_finished()
    }

    /// Split a batch into what the host sees and the handles we keep — and
    /// forget the handles of requests the routine dropped unanswered, so this
    /// table tracks the driver's rather than growing past it.
    fn show(&mut self, step: Yield<E>) -> Yield<Shown<E::View>> {
        let (effects, closed) = step.into_parts();
        let shown = effects
            .into_iter()
            .map(|effect| {
                let (view, pending) = effect.split();
                let awaits = pending.is_some();
                if let Some(p) = pending {
                    self.highest_shown = self.highest_shown.max(p.id());
                    self.pending.push((p.id(), p));
                }
                Shown { view, awaits }
            })
            .collect();

        // After inserting: a request polled and dropped in the same step is
        // both in this batch and already closed.
        for id in &closed {
            self.pending.retain(|(i, _)| i != id);
        }

        Yield::new(shown, closed)
    }

    fn take(&mut self, id: u64) -> Result<Pending, Error> {
        if self.driver.is_finished() {
            return Err(Error::Finished);
        }

        match self.pending.iter().position(|(i, _)| *i == id) {
            Some(at) => Ok(self.pending.swap_remove(at).1),
            None if id != 0 && id <= self.highest_shown => Err(Error::Stale { id }),
            None => Err(Error::BadInput),
        }
    }

    fn deliver<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Yield<Shown<E::View>> {
        let effects = self.driver.reply(reply, value);
        self.show(effects)
    }
}

impl<E, D> core::fmt::Debug for Machine<E, D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Machine")
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

/// One effect as a host sees it, and whether it awaits a reply: what the byte
/// layer needs to frame it.
pub(crate) struct Shown<V> {
    pub(crate) view: V,
    pub(crate) awaits: bool,
}

fn views<V>(step: Yield<Shown<V>>) -> Yield<V> {
    step.map(|s| s.view)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::fixtures::{Both, Echo, Holds, Impatient, View};
    use sans_effort::{reply::kind::Kind, step::Step};

    #[test]
    fn typed_layer() {
        let mut m = Machine::from_routine(|outbox| Echo(outbox).run());
        assert_eq!(m.resume().expect("resume").effects(), [View::Ask(1)]);
        assert!(
            m.resume().expect("resume").is_empty(),
            "resuming again before the reply: nothing new, harmless"
        );
        assert_eq!(
            m.reply(1, 7u64),
            Err(Error::WrongKind {
                id: 1,
                expected: Kind::Str,
                got: Kind::U64,
            }),
            "wrong kind: named, and the handle kept"
        );
        assert_eq!(
            m.reply(1, String::from("hi")).expect("reply").effects(),
            [View::Say("hi".into())]
        );
        assert_eq!(m.status(), Status::Complete);
        assert_eq!(m.reply(1, String::from("again")), Err(Error::Finished));
    }

    #[test]
    fn dropped_requests_are_forgotten() {
        let mut m = Machine::from_routine(|outbox| Impatient(outbox).run());
        let step = m.resume().expect("resume");
        assert_eq!(
            step.effects(),
            [View::Ask(1), View::Ask(2)],
            "abandoned, then live"
        );
        assert_eq!(step.closed(), [1], "the abandoned one is reported closed");
        assert_eq!(
            m.pending.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [2],
            "the abandoned request's handle was dropped with the request"
        );
        assert_eq!(
            m.reply(1, String::from("late")),
            Err(Error::Stale { id: 1 }),
            "a late reply to an abandoned id is stale, not a host bug"
        );
        assert_eq!(
            m.reply(99, String::from("?")),
            Err(Error::BadInput),
            "an id never issued is a host bug"
        );
        assert_eq!(
            m.reply(2, String::from("kept")).expect("reply").effects(),
            [View::Say("kept".into())]
        );
        assert_eq!(m.status(), Status::Complete);
    }

    #[test]
    fn a_second_reply_to_an_answered_id_is_stale() {
        let mut m = Machine::from_routine(|outbox| Both(outbox).run());
        drop(m.resume().expect("resume"));
        drop(m.reply(1, String::from("a")).expect("reply 1"));
        assert_eq!(m.reply(1, String::from("a")), Err(Error::Stale { id: 1 }));
    }

    #[test]
    fn completion_closes_what_the_routine_still_held() {
        let mut m = Machine::from_routine(|outbox| Holds(outbox).run());
        let step = m.resume().expect("resume");
        assert_eq!(m.status(), Status::Complete);
        assert_eq!(step.effects(), [View::Ask(1), View::Say("done".into())]);
        assert_eq!(step.closed(), [1], "held until return, then closed");
    }

    #[test]
    fn resume_begins_a_machine_and_is_harmless_after() {
        let mut m = Machine::from_routine(|outbox| Echo(outbox).run());
        assert_eq!(
            m.reply(1, String::from("early")),
            Err(Error::BadInput),
            "before the first resume, no id has been issued"
        );
        assert_eq!(
            m.resume().expect("the first resume begins it").effects(),
            [View::Ask(1)]
        );
        assert!(
            m.resume().expect("resume").is_empty(),
            "nothing new: harmless"
        );
        assert_eq!(m.status(), Status::Awaiting, "still awaiting its reply");
        drop(m.reply(1, String::from("hi")).expect("reply"));
        assert_eq!(m.resume(), Err(Error::Finished));
    }
}
