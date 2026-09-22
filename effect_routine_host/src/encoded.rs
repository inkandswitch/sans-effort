//! The byte layer: a [`Machine`] whose calls take and return bytes.
//!
//! Same two operations as the typed machine — [`start`](Encoded::start) and
//! [`reply`](Encoded::reply) — with one reply record in and the effects
//! encoded out, one record each, by the routine's [`Encode`] impl. This is
//! what a C-ABI or `erl_nif` skin calls, and what the [`table`](crate::table)
//! holds.

mod input;

use self::input::Input;
use crate::{error::Error, machine::Machine, status::Status};
use effect_routine::boundary::{codec::Encode, codec::Writer, host_effect::HostEffect};

/// A machine seen through bytes.
pub struct Encoded<E>(Machine<E>);

impl<E: HostEffect> Encoded<E>
where
    E::View: Encode,
{
    /// The byte layer over `machine`.
    #[must_use]
    pub const fn new(machine: Machine<E>) -> Self {
        Self(machine)
    }

    /// The typed machine underneath.
    #[must_use]
    pub const fn machine(&self) -> &Machine<E> {
        &self.0
    }

    /// Run to the first wait: the effects it recorded, encoded, and its status.
    ///
    /// # Errors
    ///
    /// As [`Machine::start`].
    pub fn start(&mut self) -> Result<(Vec<u8>, Status), Error> {
        let views = self.0.start()?;
        Ok((encode(&views), self.0.status()))
    }

    /// Deliver one reply record — `1 id str`, `2 id u64`, `3 id`, or
    /// `4 id bytes`, and nothing after it — and run to the next wait: the
    /// effects it recorded, encoded, and its status.
    ///
    /// # Errors
    ///
    /// [`Error::BadInput`] on a malformed record or trailing bytes, plus
    /// whatever [`Machine::reply`] returns.
    pub fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
        let views = match Input::decode(record)? {
            Input::Bytes(id, v) => self.0.reply(id, v)?,
            Input::Str(id, v) => self.0.reply(id, v)?,
            Input::U64(id, v) => self.0.reply(id, v)?,
            Input::Unit(id) => self.0.reply(id, ())?,
        };
        Ok((encode(&views), self.0.status()))
    }
}

impl<E> std::fmt::Debug for Encoded<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Encoded").field(&self.0).finish()
    }
}

fn encode<V: Encode>(views: &[V]) -> Vec<u8> {
    let mut w = Writer::new();
    for view in views {
        view.encode(&mut w);
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::fixtures::{Both, Echo, reply_str_record};
    use effect_routine::{boundary::codec::Writer, run::Run};

    fn encoded<F: core::future::Future<Output = ()> + Send + 'static>(
        make: impl FnOnce(effect_routine::driver::outbox::Outbox<crate::fixtures::Effect>) -> F,
    ) -> Encoded<crate::fixtures::Effect> {
        Encoded::new(Machine::from_routine(make))
    }

    /// Two requests in one batch, replied to through the byte layer in the
    /// other order: ids 1 and 2 out, `2 replied → []`, `1 replied → Say`.
    #[test]
    fn fan_out_through_the_byte_layer() {
        let mut m = encoded(|outbox| Both(outbox).run());
        let (bytes, status) = m.start().expect("start");
        assert_eq!(status, Status::Awaiting);
        let mut want = Writer::new();
        want.u8(1);
        want.u64(1);
        want.u8(1);
        want.u64(2);
        assert_eq!(bytes, want.finish(), "Ask·1, Ask·2");

        let (bytes, status) = m.reply(&reply_str_record(2, "b")).expect("reply 2");
        assert!(bytes.is_empty(), "one of two replied: no new effects");
        assert_eq!(status, Status::Awaiting);

        let (bytes, status) = m.reply(&reply_str_record(1, "a")).expect("reply 1");
        assert_eq!(status, Status::Complete);

        let mut want = Writer::new();
        want.u8(2);
        want.str("a+b");
        assert_eq!(bytes, want.finish());
    }

    #[test]
    fn malformed_input_is_bad_input_never_a_panic() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
            let mut m = encoded(|outbox| Echo(outbox).run());
            m.start().expect("start");
            // Any input either decodes to a well-formed record or is
            // `BadInput`; a well-formed record for id 1 of kind str succeeds.
            match m.reply(bytes) {
                Ok((_, status)) => assert_eq!(status, Status::Complete),
                Err(e) => assert_eq!(e, Error::BadInput),
            }
        });
    }

    #[test]
    fn trailing_bytes_are_bad_input() {
        let mut m = encoded(|outbox| Echo(outbox).run());
        assert_eq!(m.start().expect("start").1, Status::Awaiting);
        let mut record = reply_str_record(1, "hi");
        record.push(0);
        assert_eq!(m.reply(&record), Err(Error::BadInput), "trailing byte");
        assert_eq!(
            m.machine().status(),
            Status::Awaiting,
            "nothing was delivered"
        );
    }

    #[test]
    fn start_encoded_is_the_first_batch() {
        let mut m = encoded(|outbox| Echo(outbox).run());
        let (bytes, status) = m.start().expect("start");
        assert_eq!(status, Status::Awaiting);
        let mut want = Writer::new();
        want.u8(1);
        want.u64(1);
        assert_eq!(bytes, want.finish());
        assert_eq!(m.start(), Err(Error::BadInput), "start twice");
    }
}
