//! What crosses a boundary: the reply menu, the traits an effect type
//! implements to be shown to a host that cannot hold a Rust value, and the
//! byte codec.
//!
//! This is the `no_std` half of the boundary. The handle table, panic
//! isolation, and input decoding are `effect_routine_host`, which is `std`; a
//! routine's own crate depends on this module without pulling that in — which
//! an effects-style routine, whose enum _is_ the wire, must.
//!
//! # The menu
//!
//! [`Value`] is every type a reply can have: bytes, a string, a `u64`, or
//! nothing. That is the ABI's menu and the mailbox's too — replies are stored
//! as `Value`, not as `Box<dyn Any>`, so there is no downcast and no
//! `'static` bound on request types. A richer reply crosses as bytes and is
//! decoded on the routine's side. [`Reply`] is the trait a request's result
//! type implements to be carried; it is implemented for exactly the menu, and
//! sealed.
//!
//! # Showing an effect to a host
//!
//! A foreign host answers by request id, not by [`ReplyHandle`]. So before an
//! effect crosses, [`HostEffect::split`] separates the host's
//! [`View`](HostEffect::View) of it — the effect with its handle replaced by
//! the id — from the [`Pending`] handle the host layer keeps to type-check
//! the reply. [`Encode`] then writes a view as bytes: tag, fields, and, for an
//! awaiting effect, its id.

use crate::reply::ReplyHandle;
use alloc::{string::String, vec::Vec};

/// A reply value as the mailbox and the ABI carry it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// Anything else, encoded by the host and decoded on the routine's side.
    Bytes(Vec<u8>),
    /// Text.
    Str(String),
    /// Counts, ids, timestamps, lengths.
    U64(u64),
    /// An acknowledgement: a sleep, an ack.
    Unit,
}

/// Which entry of the menu a slot expects. Recorded when a request opens,
/// checked when a value is delivered, so a mismatch is caught at the mailbox
/// and never reaches the routine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// [`Value::Bytes`].
    Bytes,
    /// [`Value::Str`].
    Str,
    /// [`Value::U64`].
    U64,
    /// [`Value::Unit`].
    Unit,
}

impl Value {
    /// Which entry of the menu this is.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Value::Bytes(_) => Kind::Bytes,
            Value::Str(_) => Kind::Str,
            Value::U64(_) => Kind::U64,
            Value::Unit => Kind::Unit,
        }
    }
}

/// A type a request may ask for. Sealed: the impls below are the menu, and
/// nothing outside this crate can add to it — which is what lets a driver
/// treat a kind mismatch as unreachable rather than as an error.
pub trait Reply: private::Sealed + Sized {
    /// The menu entry this type is.
    const KIND: Kind;

    /// Wrap for the mailbox.
    fn into_value(self) -> Value;

    /// `None` if the value is of another kind — which a
    /// [`ReplyHandle<T>`] minted for a `T` slot makes unreachable through the
    /// public API.
    fn from_value(value: Value) -> Option<Self>;

    /// Wrap a handle for the host layer's table.
    fn pending(reply: ReplyHandle<Self>) -> Pending;

    /// Unwrap a pending handle of this kind, or hand it back untouched.
    ///
    /// # Errors
    ///
    /// The same `Pending`, if it awaits another kind.
    fn from_pending(pending: Pending) -> Result<ReplyHandle<Self>, Pending>;
}

mod private {
    pub trait Sealed {}

    impl Sealed for alloc::vec::Vec<u8> {}
    impl Sealed for alloc::string::String {}
    impl Sealed for u64 {}
    impl Sealed for () {}
}

impl Reply for Vec<u8> {
    const KIND: Kind = Kind::Bytes;

    fn into_value(self) -> Value {
        Value::Bytes(self)
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Bytes(v) => Some(v),
            Value::Str(_) | Value::U64(_) | Value::Unit => None,
        }
    }

    fn pending(reply: ReplyHandle<Self>) -> Pending {
        Pending::Bytes(reply)
    }

    fn from_pending(pending: Pending) -> Result<ReplyHandle<Self>, Pending> {
        match pending {
            Pending::Bytes(r) => Ok(r),
            other @ (Pending::Str(_) | Pending::U64(_) | Pending::Unit(_)) => Err(other),
        }
    }
}

impl Reply for String {
    const KIND: Kind = Kind::Str;

    fn into_value(self) -> Value {
        Value::Str(self)
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Str(v) => Some(v),
            Value::Bytes(_) | Value::U64(_) | Value::Unit => None,
        }
    }

    fn pending(reply: ReplyHandle<Self>) -> Pending {
        Pending::Str(reply)
    }

    fn from_pending(pending: Pending) -> Result<ReplyHandle<Self>, Pending> {
        match pending {
            Pending::Str(r) => Ok(r),
            other @ (Pending::Bytes(_) | Pending::U64(_) | Pending::Unit(_)) => Err(other),
        }
    }
}

impl Reply for u64 {
    const KIND: Kind = Kind::U64;

    fn into_value(self) -> Value {
        Value::U64(self)
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::U64(v) => Some(v),
            Value::Bytes(_) | Value::Str(_) | Value::Unit => None,
        }
    }

    fn pending(reply: ReplyHandle<Self>) -> Pending {
        Pending::U64(reply)
    }

    fn from_pending(pending: Pending) -> Result<ReplyHandle<Self>, Pending> {
        match pending {
            Pending::U64(r) => Ok(r),
            other @ (Pending::Bytes(_) | Pending::Str(_) | Pending::Unit(_)) => Err(other),
        }
    }
}

impl Reply for () {
    const KIND: Kind = Kind::Unit;

    fn into_value(self) -> Value {
        Value::Unit
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Unit => Some(()),
            Value::Bytes(_) | Value::Str(_) | Value::U64(_) => None,
        }
    }

    fn pending(reply: ReplyHandle<Self>) -> Pending {
        Pending::Unit(reply)
    }

    fn from_pending(pending: Pending) -> Result<ReplyHandle<Self>, Pending> {
        match pending {
            Pending::Unit(r) => Ok(r),
            other @ (Pending::Bytes(_) | Pending::Str(_) | Pending::U64(_)) => Err(other),
        }
    }
}

/// An outstanding request, by the kind of reply it takes. One variant per
/// entry of the menu; a host replies with the matching `reply_*`.
#[derive(Debug, PartialEq, Eq)]
pub enum Pending {
    /// Awaits [`Value::Bytes`].
    Bytes(ReplyHandle<Vec<u8>>),
    /// Awaits [`Value::Str`].
    Str(ReplyHandle<String>),
    /// Awaits [`Value::U64`].
    U64(ReplyHandle<u64>),
    /// Awaits [`Value::Unit`].
    Unit(ReplyHandle<()>),
}

impl Pending {
    /// The request id of the handle inside.
    #[must_use]
    pub const fn id(&self) -> u64 {
        match self {
            Pending::Bytes(r) => r.id(),
            Pending::Str(r) => r.id(),
            Pending::U64(r) => r.id(),
            Pending::Unit(r) => r.id(),
        }
    }

    /// Which entry of the menu the request awaits.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Pending::Bytes(_) => Kind::Bytes,
            Pending::Str(_) => Kind::Str,
            Pending::U64(_) => Kind::U64,
            Pending::Unit(_) => Kind::Unit,
        }
    }
}

/// An effect a host can receive.
///
/// A host that cannot hold a Rust value — anything across an ABI — sees the
/// effect with its [`ReplyHandle`] replaced by the request id. The handle
/// stays on this side as a [`Pending`], so the reply is still typed.
/// [`View`](Self::View) is what every skin hands on: as Python objects, as
/// Erlang terms, or, through [`Encode`], as bytes.
///
/// A Rust host that holds the effect itself never needs this trait.
pub trait HostEffect {
    /// The handle-free view: an awaiting effect carries its `u64` id instead.
    type View;

    /// Separate the host's view from the handle the host layer keeps.
    fn split(self) -> (Self::View, Option<Pending>);
}

/// How a [`HostEffect::View`] is written for the byte layer: tag, fields,
/// and, for an awaiting effect, its id.
pub trait Encode {
    /// Append this value's records to `w`.
    fn encode(&self, w: &mut Writer);
}

/// Appends little-endian records to a buffer.
#[derive(Debug, Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    /// An empty buffer.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// One byte.
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }

    /// Four bytes, little-endian.
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    /// Eight bytes, little-endian.
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }

    /// A `u32` length, then the bytes.
    ///
    /// # Panics
    ///
    /// If `b` is 4 GiB or longer: the wire carries a `u32` length, and a
    /// payload that size is a bug, not a case.
    #[expect(
        clippy::expect_used,
        reason = "a >4 GiB record is a programming error, not a runtime case"
    )]
    pub fn bytes(&mut self, b: &[u8]) {
        self.u32(u32::try_from(b.len()).expect("payload shorter than 4 GiB"));
        self.0.extend_from_slice(b);
    }

    /// A `u32` length, then UTF-8.
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }

    /// The buffer.
    #[must_use]
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// Reads little-endian records; every method returns `None` on short or
/// malformed input.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Read from the start of `bytes`.
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.bytes.split_at_checked(n)?;
        self.bytes = tail;
        Some(head)
    }

    /// One byte.
    pub fn u8(&mut self) -> Option<u8> {
        self.take(1).and_then(|b| b.first().copied())
    }

    /// Four bytes, little-endian.
    pub fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
    }

    /// Eight bytes, little-endian.
    pub fn u64(&mut self) -> Option<u64> {
        self.take(8)
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes)
    }

    /// A `u32` length, then that many bytes.
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = usize::try_from(self.u32()?).ok()?;
        self.take(len)
    }

    /// A `u32` length, then UTF-8.
    pub fn str(&mut self) -> Option<String> {
        String::from_utf8(self.bytes()?.to_vec()).ok()
    }

    /// `true` when every byte has been consumed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use alloc::vec;

    #[test]
    fn menu_round_trips_through_value() {
        assert_eq!(
            <Vec<u8>>::from_value(vec![1u8, 2].into_value()),
            Some(vec![1, 2])
        );
        assert_eq!(
            String::from_value(String::from("x").into_value()),
            Some(String::from("x"))
        );
        assert_eq!(u64::from_value(7u64.into_value()), Some(7));
        assert_eq!(<()>::from_value(().into_value()), Some(()));
        assert_eq!(u64::from_value(Value::Unit), None);
    }

    #[test]
    fn codec_round_trips() {
        bolero::check!()
            .with_type::<(u8, u32, u64, Vec<u8>, String)>()
            .for_each(|(a, b, c, d, e)| {
                let mut w = Writer::new();
                w.u8(*a);
                w.u32(*b);
                w.u64(*c);
                w.bytes(d);
                w.str(e);

                let bytes = w.finish();
                let mut r = Reader::new(&bytes);
                assert_eq!(r.u8(), Some(*a));
                assert_eq!(r.u32(), Some(*b));
                assert_eq!(r.u64(), Some(*c));
                assert_eq!(r.bytes(), Some(d.as_slice()));
                assert_eq!(r.str().as_deref(), Some(e.as_str()));
                assert!(r.is_empty());
            });
    }

    #[test]
    fn reader_rejects_short_input() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|bytes| {
            let mut r = Reader::new(bytes);
            // Whatever the prefix decodes to, a truncated tail must be `None`,
            // never a panic.
            let _ = r.u8();
            let _ = r.u32();
            let _ = r.u64();
            let _ = r.bytes();
            drop(r.str());
        });
    }
}
