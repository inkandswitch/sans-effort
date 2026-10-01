//! Replies: what a routine's wait resolves to, and the handle that answers it.
//!
//! On the wire a reply is one of four things — bytes, a string, a `u64`, or
//! nothing ([`value::Value`]) — and [`Reply`] is the sealed trait for those
//! four. The menu is small on purpose: it is what every foreign host can
//! carry across an ABI, and the mailbox stores replies as `Value`, not
//! `Box<dyn Any>`, so there is no downcast and no `'static` bound on request
//! types.
//!
//! A routine waits for an [`Answer`]: a type that crosses as one of the four.
//! The wire kinds answer as themselves; `Result` and `Option` cross as bytes,
//! encoded with this crate's codec; any other type can say how it crosses. A
//! host replies with the answer (in Rust) or with its wire form (over an
//! ABI), and a wire form that does not decode as the awaited answer is
//! refused where it is delivered, so the routine never sees it.
//!
//! [`handle::ReplyHandle<A>`] is the typed, single-use capability to answer
//! one wait with an `A: Answer`.

pub mod handle;
pub mod kind;
pub mod value;

use self::{handle::ReplyHandle, value::Value};
use crate::boundary::{
    codec::{Decode, DecodeError, Encode},
    pending::Pending,
};
use alloc::{string::String, vec::Vec};

/// A reply's form on the wire. Sealed: the impls below are the menu, and
/// nothing outside this crate can add to it — which is what lets a driver
/// treat a kind mismatch as unreachable rather than as an error.
///
/// Each kind is also an [`Answer`] that crosses as itself.
pub trait Reply: private::Sealed + Answer<Wire = Self> + Clone {
    /// Wrap for the mailbox.
    fn into_value(self) -> Value;

    /// `None` if the value is of another kind — which a
    /// [`ReplyHandle`] minted for a slot of this kind makes unreachable
    /// through the public API.
    fn from_value(value: Value) -> Option<Self>;

    /// Borrow the value as this kind, or `None` if it is of another.
    fn from_value_ref(value: &Value) -> Option<&Self>;

    /// Wrap a handle for the host layer's table.
    fn into_pending(reply: ReplyHandle<Self>) -> Pending;

    /// Unwrap a pending handle of this kind, or hand it back untouched.
    ///
    /// # Errors
    ///
    /// The same `Pending`, if it awaits another kind.
    fn from_pending(pending: Pending) -> Result<ReplyHandle<Self>, Pending>;
}

/// What a routine's wait resolves to, and how it crosses: as one of the four
/// [`Reply`] kinds.
///
/// The wire kinds answer as themselves. `Result<T, E>` and `Option<T>` cross
/// as bytes in this crate's [codec](crate::boundary::codec), so a fallible
/// request can say what it returns — `Result<String, ReadLineError>` — and a
/// host that replies in Rust is checked against exactly that. Any other type
/// implements this trait to say how it crosses.
///
/// ```
/// use sans_effort_core::{boundary::codec::DecodeError, reply::Answer};
///
/// /// A temperature, crossing as a `u64` of millikelvin.
/// struct Kelvin(f64);
///
/// impl Answer for Kelvin {
///     type Wire = u64;
///
///     fn into_wire(self) -> u64 {
///         (self.0 * 1000.0) as u64
///     }
///
///     fn from_wire(millis: u64) -> Result<Self, DecodeError> {
///         Ok(Kelvin(millis as f64 / 1000.0))
///     }
/// }
/// ```
pub trait Answer: Sized {
    /// The kind this answer crosses as.
    type Wire: Reply;

    /// This answer in its wire form.
    fn into_wire(self) -> Self::Wire;

    /// Read an answer back from its wire form.
    ///
    /// # Errors
    ///
    /// If `wire` does not hold a valid answer. A host's reply is checked
    /// with [`check`](Self::check) where it is delivered, so a routine never
    /// sees this error.
    fn from_wire(wire: Self::Wire) -> Result<Self, DecodeError>;

    /// Whether `wire` holds a valid answer, without keeping it. The default
    /// decodes a copy.
    ///
    /// # Errors
    ///
    /// As [`from_wire`](Self::from_wire).
    fn check(wire: &Self::Wire) -> Result<(), DecodeError> {
        Self::from_wire(wire.clone()).map(drop)
    }

    /// Wrap a handle for the host layer's table, under its wire kind.
    #[must_use]
    fn pending(reply: ReplyHandle<Self>) -> Pending {
        Self::Wire::into_pending(reply.retype())
    }
}

mod private {
    pub trait Sealed {}

    impl Sealed for alloc::vec::Vec<u8> {}
    impl Sealed for alloc::string::String {}
    impl Sealed for u64 {}
    impl Sealed for () {}
}

impl Reply for Vec<u8> {
    fn into_value(self) -> Value {
        Value::Bytes(self)
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Bytes(v) => Some(v),
            Value::Str(_) | Value::U64(_) | Value::Unit => None,
        }
    }

    fn from_value_ref(value: &Value) -> Option<&Self> {
        match value {
            Value::Bytes(v) => Some(v),
            Value::Str(_) | Value::U64(_) | Value::Unit => None,
        }
    }

    fn into_pending(reply: ReplyHandle<Self>) -> Pending {
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
    fn into_value(self) -> Value {
        Value::Str(self)
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Str(v) => Some(v),
            Value::Bytes(_) | Value::U64(_) | Value::Unit => None,
        }
    }

    fn from_value_ref(value: &Value) -> Option<&Self> {
        match value {
            Value::Str(v) => Some(v),
            Value::Bytes(_) | Value::U64(_) | Value::Unit => None,
        }
    }

    fn into_pending(reply: ReplyHandle<Self>) -> Pending {
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
    fn into_value(self) -> Value {
        Value::U64(self)
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::U64(v) => Some(v),
            Value::Bytes(_) | Value::Str(_) | Value::Unit => None,
        }
    }

    fn from_value_ref(value: &Value) -> Option<&Self> {
        match value {
            Value::U64(v) => Some(v),
            Value::Bytes(_) | Value::Str(_) | Value::Unit => None,
        }
    }

    fn into_pending(reply: ReplyHandle<Self>) -> Pending {
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
    fn into_value(self) -> Value {
        Value::Unit
    }

    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Unit => Some(()),
            Value::Bytes(_) | Value::Str(_) | Value::U64(_) => None,
        }
    }

    fn from_value_ref(value: &Value) -> Option<&Self> {
        match value {
            Value::Unit => Some(&()),
            Value::Bytes(_) | Value::Str(_) | Value::U64(_) => None,
        }
    }

    fn into_pending(reply: ReplyHandle<Self>) -> Pending {
        Pending::Unit(reply)
    }

    fn from_pending(pending: Pending) -> Result<ReplyHandle<Self>, Pending> {
        match pending {
            Pending::Unit(r) => Ok(r),
            other @ (Pending::Bytes(_) | Pending::Str(_) | Pending::U64(_)) => Err(other),
        }
    }
}

/// The wire kinds answer as themselves; checking one is free.
macro_rules! answers_as_itself {
    ($($t:ty),*) => {$(
        impl Answer for $t {
            type Wire = Self;

            fn into_wire(self) -> Self {
                self
            }

            fn from_wire(wire: Self) -> Result<Self, DecodeError> {
                Ok(wire)
            }

            fn check(_: &Self) -> Result<(), DecodeError> {
                Ok(())
            }
        }
    )*};
}

answers_as_itself!(Vec<u8>, String, u64, ());

impl<T: Encode + Decode, E: Encode + Decode> Answer for Result<T, E> {
    type Wire = Vec<u8>;

    fn into_wire(self) -> Vec<u8> {
        self.to_bytes()
    }

    fn from_wire(wire: Vec<u8>) -> Result<Self, DecodeError> {
        Self::from_bytes(&wire)
    }

    fn check(wire: &Vec<u8>) -> Result<(), DecodeError> {
        Self::from_bytes(wire).map(drop)
    }
}

impl<T: Encode + Decode> Answer for Option<T> {
    type Wire = Vec<u8>;

    fn into_wire(self) -> Vec<u8> {
        self.to_bytes()
    }

    fn from_wire(wire: Vec<u8>) -> Result<Self, DecodeError> {
        Self::from_bytes(&wire)
    }

    fn check(wire: &Vec<u8>) -> Result<(), DecodeError> {
        Self::from_bytes(wire).map(drop)
    }
}

/// Whether a delivered value is a valid `A`: the check a mailbox slot runs
/// before it accepts a reply.
pub(crate) fn check<A: Answer>(value: &Value) -> Result<(), DecodeError> {
    match A::Wire::from_value_ref(value) {
        Some(wire) => A::check(wire),
        None => unreachable!("a ReplyHandle is only minted for a slot of its kind"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wire value comes back as itself from its own kind, and as nothing
    /// from any other — borrowed or taken.
    #[test]
    fn menu_round_trips_through_value() {
        bolero::check!()
            .with_type::<(Vec<u8>, String, u64)>()
            .for_each(|(bytes, text, n)| {
                let values = [
                    bytes.clone().into_value(),
                    text.clone().into_value(),
                    n.into_value(),
                    ().into_value(),
                ];
                for (at, value) in values.into_iter().enumerate() {
                    // Borrowing sees what taking sees.
                    assert_eq!(
                        <Vec<u8>>::from_value_ref(&value),
                        (at == 0).then_some(bytes)
                    );
                    assert_eq!(String::from_value_ref(&value), (at == 1).then_some(text));
                    assert_eq!(u64::from_value_ref(&value), (at == 2).then_some(n));
                    assert_eq!(<()>::from_value_ref(&value), (at == 3).then_some(&()));

                    assert_eq!(
                        <Vec<u8>>::from_value(value.clone()),
                        (at == 0).then(|| bytes.clone())
                    );
                    assert_eq!(
                        String::from_value(value.clone()),
                        (at == 1).then(|| text.clone())
                    );
                    assert_eq!(u64::from_value(value.clone()), (at == 2).then_some(*n));
                    assert_eq!(<()>::from_value(value), (at == 3).then_some(()));
                }
            });
    }

    /// `Result` and `Option` answers survive the trip through their wire form.
    #[test]
    fn compound_answers_round_trip() {
        bolero::check!()
            .with_type::<(bool, String, u64, Option<String>)>()
            .for_each(|(ok, text, n, maybe)| {
                let result: Result<String, u64> = if *ok { Ok(text.clone()) } else { Err(*n) };
                assert_eq!(
                    <Result<String, u64>>::from_wire(result.clone().into_wire()),
                    Ok(result)
                );
                assert_eq!(
                    <Option<String>>::from_wire(maybe.clone().into_wire()),
                    Ok(maybe.clone())
                );
            });
    }

    /// The check a mailbox slot runs agrees with decoding, on any bytes: the
    /// invariant that makes an undecodable reply unreachable for a routine.
    #[test]
    fn check_agrees_with_decoding() {
        bolero::check!().with_type::<Vec<u8>>().for_each(|wire| {
            assert_eq!(
                <Result<String, u64>>::check(wire).is_ok(),
                <Result<String, u64>>::from_wire(wire.clone()).is_ok()
            );
            assert_eq!(
                <Option<String>>::check(wire).is_ok(),
                <Option<String>>::from_wire(wire.clone()).is_ok()
            );
        });
    }
}
