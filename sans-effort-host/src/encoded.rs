//! The byte layer: a [`Machine`] whose calls take and return bytes.
//!
//! Same operations as the typed machine — [`start`](Encoded::start),
//! [`reply`](Encoded::reply), and [`resume`](Encoded::resume) — with one
//! reply record in and the effects encoded out. Each effect is one _frame_: a kind
//! ([`FRAME_TELL`] or [`FRAME_ASK`]), a `u32` length, and the view's
//! bytes as the routine's [`Encode`] impl writes them. The length lets a host
//! skip a tell it does not understand and check that it parsed each record
//! exactly. After the effects, one [`FRAME_CLOSED`] per request the routine
//! abandoned, holding its id. This is what a C-ABI or `erl_nif` binding calls, and what the
//! [`table`](crate::table) holds.

mod input;

use self::input::Input;
use crate::{
    contract::{FRAME_ASK, FRAME_CLOSED, FRAME_TELL},
    error::Error,
    machine::{Drive, Machine, Shown},
    status::Status,
};
use alloc::vec::Vec;
use sans_effort::{
    boundary::{codec::Encode, codec::Writer, host_effect::HostEffect},
    driver::{Driver, step::Step},
};

/// A machine seen through bytes.
pub struct Encoded<E, D = Driver<E>>(Machine<E, D>);

impl<E: HostEffect, D: Drive<E>> Encoded<E, D>
where
    E::View: Encode,
{
    /// The byte layer over `machine`.
    #[must_use]
    pub const fn new(machine: Machine<E, D>) -> Self {
        Self(machine)
    }

    /// The typed machine underneath.
    #[must_use]
    pub const fn machine(&self) -> &Machine<E, D> {
        &self.0
    }

    /// Run to the first wait: the effects it recorded, encoded, and its status.
    ///
    /// # Errors
    ///
    /// As [`Machine::start`].
    pub fn start(&mut self) -> Result<(Vec<u8>, Status), Error> {
        let shown = self.0.start_shown()?;
        Ok((encode(&shown), self.0.status()))
    }

    /// Deliver one reply record — `1 id str`, `2 id u64`, `3 id`, or
    /// `4 id bytes`, and nothing after it — and run to the next wait: the
    /// effects it recorded, encoded, and its status.
    ///
    /// # Errors
    ///
    /// [`Error::Malformed`] on a malformed record or trailing bytes, plus
    /// whatever [`Machine::reply`] returns.
    pub fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
        let shown = match Input::decode(record)? {
            Input::Bytes(id, v) => self.0.reply_shown(id, v)?,
            Input::Str(id, v) => self.0.reply_shown(id, v)?,
            Input::U64(id, v) => self.0.reply_shown(id, v)?,
            Input::Unit(id) => self.0.reply_shown(id, ())?,
        };
        Ok((encode(&shown), self.0.status()))
    }

    /// Poll again without delivering anything: the effects recorded before
    /// the next wait, encoded, and the status.
    ///
    /// # Errors
    ///
    /// As [`Machine::resume`].
    pub fn resume(&mut self) -> Result<(Vec<u8>, Status), Error> {
        let shown = self.0.resume_shown()?;
        Ok((encode(&shown), self.0.status()))
    }
}

impl<E, D> core::fmt::Debug for Encoded<E, D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("Encoded").field(&self.0).finish()
    }
}

/// One frame per effect — kind, `u32` length, the view's own bytes — then one
/// closed frame per abandoned id.
fn encode<V: Encode>(step: &Step<Shown<V>>) -> Vec<u8> {
    let mut w = Writer::new();
    for Shown { view, awaits } in step.effects() {
        w.u8(if *awaits { FRAME_ASK } else { FRAME_TELL });
        w.bytes(&view.to_bytes());
    }
    for id in step.closed() {
        w.u8(FRAME_CLOSED);
        w.bytes(&id.to_bytes());
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use crate::fixtures::{
        Both, Echo, Holds, Impatient, View, closed_frame, framed, reply_str_record,
    };
    use alloc::string::String;
    use sans_effort::{boundary::codec::DecodeError, run::Run};

    fn encoded<F: core::future::Future<Output = ()> + Send + 'static>(
        make: impl FnOnce(sans_effort::driver::outbox::Outbox<crate::fixtures::Effect>) -> F,
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
        assert_eq!(bytes, framed(&[View::Ask(1), View::Ask(2)]), "Ask·1, Ask·2");

        let (bytes, status) = m.reply(&reply_str_record(2, "b")).expect("reply 2");
        assert!(bytes.is_empty(), "one of two replied: no new effects");
        assert_eq!(status, Status::Awaiting);

        let (bytes, status) = m.reply(&reply_str_record(1, "a")).expect("reply 1");
        assert_eq!(status, Status::Complete);

        assert_eq!(bytes, framed(&[View::Say(String::from("a+b"))]));
    }

    #[test]
    fn arbitrary_input_is_refused_with_a_code_never_a_panic() {
        use crate::contract::{BAD_INPUT, MALFORMED, STALE, WRONG_KIND};

        bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
            let mut m = encoded(|outbox| Echo(outbox).run());
            m.start().expect("start");
            // A well-formed str record for id 1 completes the routine.
            // Anything else is refused: unparsable is `MALFORMED`, another
            // kind for id 1 is `WRONG_KIND`, id 0 or an id never issued is
            // `BAD_INPUT`. Nothing here was answered or abandoned, so never
            // `STALE`.
            match m.reply(bytes) {
                Ok((_, status)) => assert_eq!(status, Status::Complete),
                Err(e) => {
                    assert!(
                        [BAD_INPUT, MALFORMED, WRONG_KIND].contains(&e.code()),
                        "{e}"
                    );
                    assert_ne!(e.code(), STALE);
                }
            }
        });
    }

    #[test]
    fn trailing_bytes_are_bad_input() {
        let mut m = encoded(|outbox| Echo(outbox).run());
        assert_eq!(m.start().expect("start").1, Status::Awaiting);
        let mut record = reply_str_record(1, "hi");
        record.push(0);
        assert_eq!(
            m.reply(&record),
            Err(Error::Malformed(DecodeError::TrailingBytes {
                remaining: 1
            })),
            "trailing byte"
        );
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
        assert_eq!(bytes, framed(&[View::Ask(1)]));
        assert_eq!(m.start(), Err(Error::BadInput), "start twice");
    }

    #[test]
    fn abandoned_requests_follow_the_effects_as_closed_frames() {
        let mut m = encoded(|outbox| Impatient(outbox).run());
        let (bytes, status) = m.start().expect("start");
        assert_eq!(status, Status::Awaiting);
        let mut want = framed(&[View::Ask(1), View::Ask(2)]);
        want.extend(closed_frame(1));
        assert_eq!(bytes, want);

        assert_eq!(
            m.reply(&reply_str_record(1, "late")),
            Err(Error::Stale { id: 1 }),
            "the host was told; its late reply is stale"
        );
    }

    #[test]
    fn completion_closes_what_is_still_held() {
        let mut m = encoded(|outbox| Holds(outbox).run());
        let (bytes, status) = m.start().expect("start");
        assert_eq!(status, Status::Complete);
        let mut want = framed(&[View::Ask(1), View::Say(String::from("done"))]);
        want.extend(closed_frame(1));
        assert_eq!(bytes, want);
    }

    #[test]
    fn a_closed_frame_is_kind_length_id() {
        assert_eq!(
            closed_frame(5),
            [
                3, // FRAME_CLOSED
                8, 0, 0, 0, // payload length
                5, 0, 0, 0, 0, 0, 0, 0, // the id
            ]
        );
    }

    /// The frame format, spelled out byte by byte rather than through the
    /// code that writes it.
    #[test]
    fn a_frame_is_kind_length_payload() {
        let mut m = encoded(|outbox| Echo(outbox).run());
        let (ask, _) = m.start().expect("start");
        assert_eq!(
            ask,
            [
                2, // FRAME_ASK
                9, 0, 0, 0, // payload length
                1, // the view's tag: Ask
                1, 0, 0, 0, 0, 0, 0, 0, // its request id
            ]
        );

        let (tell, _) = m.reply(&reply_str_record(1, "hi")).expect("reply");
        assert_eq!(
            tell,
            [
                1, // FRAME_TELL
                7, 0, 0, 0, // payload length
                2, // the view's tag: Say
                2, 0, 0, 0, b'h', b'i', // its text
            ]
        );
    }
}
