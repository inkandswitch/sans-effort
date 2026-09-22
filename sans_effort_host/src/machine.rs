//! The typed layer over a driver. The byte layer over that is
//! [`Encoded`](crate::encoded::Encoded).

use crate::{error::Error, status::Status};
use sans_effort::{
    boundary::{host_effect::HostEffect, pending::Pending},
    driver::{Driver, outbox::Outbox},
    reply::{Reply, handle::ReplyHandle},
};
use alloc::vec::Vec;
use core::{future::Future, marker::PhantomData};

/// A driver plus the requests it has outstanding, keyed by id: the typed
/// layer.
///
/// The type check a Rust host gets from [`ReplyHandle<T>`] at compile time
/// happens here at run time, against the kind the foreign host chose. The
/// `pending` table is this layer's wallet of capabilities: it holds the
/// handles the effects carried out, so that an `(id, value)` from across the
/// boundary can be turned back into the typed, infallible
/// [`Driver::reply`].
pub struct Machine<E> {
    driver: Driver<E>,
    /// Outstanding requests by id. A `Vec` scanned linearly, not a map: a
    /// routine has one or two requests in flight, and hashing a `u64` costs
    /// more than looking at two entries. Wide fan-out would want a sorted
    /// `Vec` with binary search.
    pending: Vec<(u64, Pending)>,
    started: bool,
    _effect: PhantomData<E>,
}

impl<E: HostEffect> Machine<E> {
    /// Wrap a driver. Nothing has been polled yet.
    #[must_use]
    pub const fn new(driver: Driver<E>) -> Self {
        Self {
            driver,
            pending: Vec::new(),
            started: false,
            _effect: PhantomData,
        }
    }

    /// Build a driver around `make` and wrap it: `Machine::new(Driver::new(make))`.
    /// Nothing runs until [`start`](Self::start).
    ///
    /// `make` receives the outbox the routine's context should write into and
    /// returns the routine's _future_ — `|outbox| Greeter::new(Ctx::new(outbox)).run()`
    /// — not the routine. The `.run()` cannot be hidden here: `Driver::new`
    /// requires the future to be `Send`, and only where the routine's type is
    /// concrete can the compiler decide that. A constructor generic over
    /// `P: Run` would have no way to state `P::run(): Send` on stable Rust.
    pub fn from_routine<F: Future<Output = ()> + Send + 'static, M: FnOnce(Outbox<E>) -> F>(
        make: M,
    ) -> Self {
        Self::new(Driver::new(make))
    }

    /// Begin: the effects recorded before the first wait. Valid once.
    ///
    /// # Errors
    ///
    /// [`Error::Finished`] if the routine has completed;
    /// [`Error::BadInput`] if already started.
    pub fn start(&mut self) -> Result<Vec<E::View>, Error> {
        if self.driver.is_finished() {
            return Err(Error::Finished);
        }

        if self.started {
            return Err(Error::BadInput);
        }

        self.started = true;
        let effects = self.driver.start();
        Ok(self.present(effects))
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
    pub fn reply<T: Reply>(&mut self, id: u64, value: T) -> Result<Vec<E::View>, Error> {
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

    /// What the last poll reported.
    #[must_use]
    pub fn status(&self) -> Status {
        self.driver.status().into()
    }

    /// `true` once the routine has returned.
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.driver.is_finished()
    }

    /// Split a batch into what the host sees and the handles we keep — and
    /// forget the handles of requests the routine dropped unanswered, so this
    /// table tracks the driver's rather than growing past it.
    fn present(&mut self, effects: Vec<E>) -> Vec<E::View> {
        let views = effects
            .into_iter()
            .map(|effect| {
                let (view, pending) = effect.split();
                if let Some(p) = pending {
                    self.pending.push((p.id(), p));
                }
                view
            })
            .collect();

        // After inserting: a request polled and dropped in the same step is
        // both in this batch and already closed.
        for id in self.driver.closed() {
            self.pending.retain(|(i, _)| *i != id);
        }

        views
    }

    fn take(&mut self, id: u64) -> Result<Pending, Error> {
        if self.driver.is_finished() {
            return Err(Error::Finished);
        }

        let at = self
            .pending
            .iter()
            .position(|(i, _)| *i == id)
            .ok_or(Error::BadInput)?;
        Ok(self.pending.swap_remove(at).1)
    }

    fn deliver<T: Reply>(&mut self, reply: ReplyHandle<T>, value: T) -> Vec<E::View> {
        let effects = self.driver.reply(reply, value);
        self.present(effects)
    }
}

impl<E> core::fmt::Debug for Machine<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Machine")
            .field("started", &self.started)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::fixtures::{Echo, Impatient, View};
    use sans_effort::{reply::kind::Kind, run::Run};

    #[test]
    fn typed_layer() {
        let mut m = Machine::from_routine(|outbox| Echo(outbox).run());
        assert_eq!(m.start().expect("start"), [View::Ask(1)]);
        assert_eq!(m.start(), Err(Error::BadInput), "start twice");
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
            m.reply(1, String::from("hi")).expect("reply"),
            [View::Say("hi".into())]
        );
        assert_eq!(m.status(), Status::Complete);
        assert_eq!(m.reply(1, String::from("again")), Err(Error::Finished));
    }

    #[test]
    fn dropped_requests_are_forgotten() {
        let mut m = Machine::from_routine(|outbox| Impatient(outbox).run());
        let views = m.start().expect("start");
        assert_eq!(views, [View::Ask(1), View::Ask(2)], "abandoned, then live");
        assert_eq!(
            m.pending.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [2],
            "the abandoned request's handle was dropped with the request"
        );
        assert_eq!(m.reply(1, String::from("late")), Err(Error::BadInput));
        assert_eq!(
            m.reply(2, String::from("kept")).expect("reply"),
            [View::Say("kept".into())]
        );
        assert_eq!(m.status(), Status::Complete);
    }
}
