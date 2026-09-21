//! The host side of an effect routine, for hosts that cannot hold a Rust
//! value — minus the C ABI.
//!
//! `new(routine) → handle`, `start(handle) → effects`, `reply(handle, record)
//! → effects`, `free(handle)`. A reply record is `kind · id · payload`, where
//! the id came out on the wire with the effect and the kind is one of the
//! reply menu's ([`Pending`]).
//!
//! Everything a foreign host needs — the handle table, the type check on
//! replies, the encoding — over owned Rust types, in two layers:
//!
//! - [`Machine`] is _typed_: [`start`](Machine::start) and
//!   [`reply`](Machine::reply) return `Vec<E::View>`, the effects with their
//!   handles replaced by ids. It owns the outstanding-request table and the
//!   kind check. Skins that speak the host language's own types —
//!   wasm-bindgen, `PyO3`, Rustler — hold one directly.
//! - [`start_encoded`](Machine::start_encoded) and
//!   [`reply_encoded`](Machine::reply_encoded) are the _byte_ layer over it:
//!   decode one reply record, call the typed method, encode the views. The
//!   [`table`] holds machines behind this, and a C-ABI or `erl_nif` skin
//!   calls it.
//!
//! The application adds the skin: one `#[no_mangle]` wrapper per function in
//! [`table`], each a line plus the `unsafe` needed to touch foreign memory.
//! That keeps this crate under `unsafe_code = "forbid"`.
//!
//! ```text
//!   app cdylib                                 effect_routine_host
//!   ────────────────────────────────           ──────────────────────────────────────────
//!   enum Effect { … }  impl HostEffect
//!   #[no_mangle] new()             ─────────▶  table::new(|outbox| Greeter::new(Ctx::new(outbox)).run())
//!   #[no_mangle] start(h, out*)    ─────────▶  table::start(h)          -> Result<(Vec<u8>, Status), Error>
//!   #[no_mangle] reply(h, in*, out*) ───────▶  table::reply(h, &[u8])   -> Result<(Vec<u8>, Status), Error>
//!                                  ◀─────────  (bytes, status)   — app writes the out-pointers
//! ```
//!
//! # Wire
//!
//! The codec, the reply menu ([`Value`](effect_routine::wire::menu::Value)), and
//! the [`HostEffect`]/[`Encode`] traits live in [`effect_routine::wire`] — the
//! `no_std` half of the boundary, so a routine's wire crate may implement
//! them. This crate decodes reply records (`1 id str`; `2 id u64`; `3 id`;
//! `4 id bytes`) and keeps the table. `ABI.md` at the repository root is the
//! contract in full.

use effect_routine::{
    driver::{Driver, status::Status as DriveStatus},
    reply::ReplyHandle,
    wire::{
        codec::{Encode, Reader, Writer},
        host_effect::HostEffect,
        menu::Reply,
        pending::Pending,
    },
};
use std::marker::PhantomData;

pub mod table;

/// Status and error codes as they cross the ABI. Non-negative is a status,
/// negative is an error. Plain constants so a host can copy them.
pub mod code {
    /// Success, for calls that carry no status (`free`).
    pub const OK: i32 = 0;
    /// Suspended; at least one effect in the batch awaits a reply. Shares
    /// `0` with [`OK`].
    pub const AWAITING: i32 = 0;
    /// Ran to completion.
    pub const COMPLETE: i32 = 1;
    /// Suspended on something the driver cannot wake.
    pub const STALLED: i32 = 2;

    /// Another thread is inside `start` or `reply` for this handle.
    pub const BUSY: i32 = -1;
    /// The routine already completed.
    pub const FINISHED: i32 = -2;
    /// The routine panicked and has been removed.
    pub const PANICKED: i32 = -4;
    /// Unknown or freed handle.
    pub const BAD_HANDLE: i32 = -5;
    /// Malformed input, a reply of the wrong kind, or an id nothing awaits.
    pub const BAD_INPUT: i32 = -6;
}

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
    /// if nothing awaits `id` or it awaits another kind (the handle is kept,
    /// so the host may retry with the right kind).
    pub fn reply<T: Reply>(&mut self, id: u64, value: T) -> Result<Vec<E::View>, Error> {
        match T::from_pending(self.take(id)?) {
            Ok(reply) => Ok(self.deliver(reply, value)),
            Err(other) => Err(self.wrong_kind(id, other)),
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

    /// Put the handle back after a kind mismatch so the host can retry.
    fn wrong_kind(&mut self, id: u64, pending: Pending) -> Error {
        self.pending.push((id, pending));
        Error::BadInput
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

/// What one `start` or `reply` reports. [`code`](Self::code) is its wire form.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Status {
    /// At least one request awaits a reply.
    Awaiting,
    /// The routine returned.
    Complete,
    /// Pending with no request outstanding; will never progress.
    Stalled,
}

impl Status {
    /// The wire code.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Status::Awaiting => code::AWAITING,
            Status::Complete => code::COMPLETE,
            Status::Stalled => code::STALLED,
        }
    }
}

impl From<DriveStatus> for Status {
    fn from(status: DriveStatus) -> Self {
        match status {
            DriveStatus::Awaiting => Status::Awaiting,
            DriveStatus::Complete => Status::Complete,
            DriveStatus::Stalled => Status::Stalled,
        }
    }
}

/// Why a call failed. [`code`](Self::code) is its wire form.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// Unknown or freed handle.
    #[error("unknown or freed handle")]
    BadHandle,
    /// Malformed input, a reply of the wrong kind, or an id nothing awaits.
    #[error("malformed input, wrong reply kind, or unknown request id")]
    BadInput,
    /// Another thread is inside `start` or `reply` for this handle.
    #[error("another thread is driving this routine")]
    Busy,
    /// The routine already completed.
    #[error("routine already completed")]
    Finished,
    /// The routine panicked and has been removed.
    #[error("routine panicked and was removed")]
    Panicked,
}

impl Error {
    /// The wire code.
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Error::BadHandle => code::BAD_HANDLE,
            Error::BadInput => code::BAD_INPUT,
            Error::Busy => code::BUSY,
            Error::Finished => code::FINISHED,
            Error::Panicked => code::PANICKED,
        }
    }
}

/// The wire code of a call that has no [`Status`]: [`code::OK`] or the
/// error's code.
#[must_use]
pub fn code_of(result: Result<(), Error>) -> i32 {
    result.map_or_else(Error::code, |()| code::OK)
}

/// One decoded reply record: what the host is telling the routine.
enum Input {
    Bytes(u64, Vec<u8>),
    Str(u64, String),
    U64(u64, u64),
    Unit(u64),
}

impl Input {
    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let bad = Error::BadInput;

        let input = match r.u8().ok_or(bad)? {
            1 => Input::Str(r.u64().ok_or(bad)?, r.str().ok_or(bad)?),
            2 => Input::U64(r.u64().ok_or(bad)?, r.u64().ok_or(bad)?),
            3 => Input::Unit(r.u64().ok_or(bad)?),
            4 => Input::Bytes(r.u64().ok_or(bad)?, r.bytes().ok_or(bad)?.to_vec()),
            _ => return Err(bad),
        };

        if r.is_empty() { Ok(input) } else { Err(bad) }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use core::ops::ControlFlow;
    use effect_routine::{
        driver::{Driver, outbox::Outbox},
        run::Run,
    };

    /// Asks once (tag 1), says the answer (tag 2), finishes.
    pub(crate) enum Effect {
        Ask(ReplyHandle<String>),
        Say(String),
    }

    #[derive(Debug, PartialEq)]
    pub(crate) enum View {
        Ask(u64),
        Say(String),
    }

    impl HostEffect for Effect {
        type View = View;

        fn split(self) -> (View, Option<Pending>) {
            match self {
                Effect::Ask(reply) => (View::Ask(reply.id()), Some(Pending::Str(reply))),
                Effect::Say(text) => (View::Say(text), None),
            }
        }
    }

    impl Encode for View {
        fn encode(&self, w: &mut Writer) {
            match self {
                View::Ask(id) => {
                    w.u8(1);
                    w.u64(*id);
                }
                View::Say(text) => {
                    w.u8(2);
                    w.str(text);
                }
            }
        }
    }

    pub(crate) struct Echo(pub(crate) Outbox<Effect>);

    impl Run for Echo {
        async fn step(&mut self) -> ControlFlow<()> {
            let answer = self.0.ask(Effect::Ask).await;
            self.0.tell(Effect::Say(answer));
            ControlFlow::Break(())
        }
    }

    pub(crate) struct Both(pub(crate) Outbox<Effect>);

    impl Run for Both {
        async fn step(&mut self) -> ControlFlow<()> {
            let (a, b) =
                effect_routine::join::join(self.0.ask(Effect::Ask), self.0.ask(Effect::Ask)).await;
            self.0.tell(Effect::Say(format!("{a}+{b}")));
            ControlFlow::Break(())
        }
    }

    /// Polls a future exactly once, then hands it back — enough to make a
    /// request record itself without waiting for its reply.
    struct PollOnce<F>(Option<F>);

    impl<F: Future + Unpin> Future for PollOnce<F> {
        type Output = F;

        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<F> {
            let mut inner = self.0.take().expect("polled once");
            drop(std::pin::Pin::new(&mut inner).poll(cx));
            std::task::Poll::Ready(inner)
        }
    }

    struct Impatient(Outbox<Effect>);

    impl Run for Impatient {
        async fn step(&mut self) -> ControlFlow<()> {
            let abandoned = PollOnce(Some(self.0.ask(Effect::Ask))).await;
            drop(abandoned);

            let kept = self.0.ask(Effect::Ask).await;
            self.0.tell(Effect::Say(kept));
            ControlFlow::Break(())
        }
    }

    pub(crate) fn reply_str_record(id: u64, s: &str) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(1);
        w.u64(id);
        w.str(s);
        w.finish()
    }

    #[test]
    fn typed_layer() {
        let mut m = Machine::new(Driver::new(|outbox| Echo(outbox).run()));
        assert_eq!(m.start().expect("start"), [View::Ask(1)]);
        assert_eq!(m.start(), Err(Error::BadInput), "start twice");
        assert_eq!(
            m.reply(1, 7u64),
            Err(Error::BadInput),
            "wrong kind, handle kept"
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
        let mut m = Machine::new(Driver::new(|outbox| Both(outbox).run()));
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
        let mut m = Machine::new(Driver::new(|outbox| Impatient(outbox).run()));
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
            let mut m = Machine::new(Driver::new(|outbox| Echo(outbox).run()));
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
        let mut m = Machine::new(Driver::new(|outbox| Echo(outbox).run()));
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
        let mut m = Machine::new(Driver::new(|outbox| Echo(outbox).run()));
        let (bytes, status) = m.start_encoded().expect("start");
        assert_eq!(status, Status::Awaiting);
        let mut want = Writer::new();
        want.u8(1);
        want.u64(1);
        assert_eq!(bytes, want.finish());
        assert_eq!(m.start_encoded(), Err(Error::BadInput), "start twice");
    }
}
