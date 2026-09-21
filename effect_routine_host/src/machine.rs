//! The typed layer over a driver, and the byte layer over that.

mod input;

use self::input::Input;
use crate::{error::Error, status::Status};
use effect_routine::{
    driver::{Driver, outbox::Outbox},
    reply::{Reply, handle::ReplyHandle},
    wire::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
        pending::Pending,
    },
};
use std::{future::Future, marker::PhantomData};

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

impl<E: HostEffect> Machine<E>
where
    E::View: Encode,
{
    /// The byte layer's `start`: the first batch, encoded by [`Encode`], one
    /// record per effect.
    ///
    /// # Errors
    ///
    /// As [`start`](Self::start).
    pub fn start_encoded(&mut self) -> Result<(Vec<u8>, Status), Error> {
        let views = self.start()?;
        Ok((Self::encode(&views), self.status()))
    }

    /// The byte layer's `reply`: one reply record in — `1 id str`, `2 id u64`,
    /// `3 id`, or `4 id bytes`, and nothing after it — the resulting effects
    /// out, encoded.
    ///
    /// # Errors
    ///
    /// [`Error::BadInput`] on a malformed record or trailing bytes, plus
    /// whatever [`reply`](Self::reply) returns.
    pub fn reply_encoded(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
        let views = match Input::decode(record)? {
            Input::Bytes(id, v) => self.reply(id, v)?,
            Input::Str(id, v) => self.reply(id, v)?,
            Input::U64(id, v) => self.reply(id, v)?,
            Input::Unit(id) => self.reply(id, ())?,
        };
        Ok((Self::encode(&views), self.status()))
    }

    fn encode(views: &[E::View]) -> Vec<u8> {
        let mut w = Writer::new();
        for view in views {
            view.encode(&mut w);
        }
        w.finish()
    }
}

impl<E> std::fmt::Debug for Machine<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    use crate::fixtures::{Both, Echo, Impatient, View, reply_str_record};
    use effect_routine::{reply::kind::Kind, run::Run, wire::codec::Writer};

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

    /// Two requests in one batch, replied to through the byte layer in the
    /// other order: ids 1 and 2 out, `2 replied → []`, `1 replied → Say`.
    #[test]
    fn fan_out_through_the_byte_layer() {
        let mut m = Machine::from_routine(|outbox| Both(outbox).run());
        assert_eq!(m.start().expect("start"), [View::Ask(1), View::Ask(2)]);

        let (bytes, status) = m.reply_encoded(&reply_str_record(2, "b")).expect("reply 2");
        assert!(bytes.is_empty(), "one of two replied: no new effects");
        assert_eq!(status, Status::Awaiting);

        let (bytes, status) = m.reply_encoded(&reply_str_record(1, "a")).expect("reply 1");
        assert_eq!(status, Status::Complete);

        let mut want = Writer::new();
        want.u8(2);
        want.str("a+b");
        assert_eq!(bytes, want.finish());
    }

    /// A request polled once then dropped leaves a slot in the driver only
    /// until the drop; the machine's own table must not keep it either.
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

    #[test]
    fn malformed_input_is_bad_input_never_a_panic() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
            let mut m = Machine::from_routine(|outbox| Echo(outbox).run());
            m.start().expect("start");
            // Any input either decodes to a well-formed record or is
            // `BadInput`; a well-formed record for id 1 of kind str succeeds.
            match m.reply_encoded(bytes) {
                Ok((_, status)) => assert_eq!(status, Status::Complete),
                Err(e) => assert_eq!(e, Error::BadInput),
            }
        });
    }

    #[test]
    fn trailing_bytes_are_bad_input() {
        let mut m = Machine::from_routine(|outbox| Echo(outbox).run());
        assert_eq!(m.start_encoded().expect("start").1, Status::Awaiting);
        let mut record = reply_str_record(1, "hi");
        record.push(0);
        assert_eq!(
            m.reply_encoded(&record),
            Err(Error::BadInput),
            "trailing byte"
        );
        assert_eq!(m.status(), Status::Awaiting, "nothing was delivered");
    }

    #[test]
    fn start_encoded_is_the_first_batch() {
        let mut m = Machine::from_routine(|outbox| Echo(outbox).run());
        let (bytes, status) = m.start_encoded().expect("start");
        assert_eq!(status, Status::Awaiting);
        let mut want = Writer::new();
        want.u8(1);
        want.u64(1);
        assert_eq!(bytes, want.finish());
        assert_eq!(m.start_encoded(), Err(Error::BadInput), "start twice");
    }
}
