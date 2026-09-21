//! The reply menu: every type a reply can have.
//!
//! [`Value`] is bytes, a string, a `u64`, or nothing. That is the ABI's menu
//! and the mailbox's too — replies are stored as `Value`, not as
//! `Box<dyn Any>`, so there is no downcast and no `'static` bound on request
//! types. A richer reply crosses as bytes and is decoded on the routine's
//! side. [`Reply`] is the trait a request's result type implements to be
//! carried; it is implemented for exactly the menu, and sealed.

use super::pending::Pending;
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
}
