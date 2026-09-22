//! Replies: what a routine's wait resolves to, and the handle that answers it.
//!
//! A reply can be one of four things — bytes, a string, a `u64`, or nothing
//! ([`value::Value`]) — and [`Reply`] is the sealed trait a request's result
//! type implements to be one of them. The menu is small on purpose: it is
//! what every foreign host can carry across an ABI, and the mailbox stores
//! replies as `Value`, not `Box<dyn Any>`, so there is no downcast and no
//! `'static` bound on request types. A richer reply crosses as bytes and is
//! decoded in the routine's context.
//!
//! [`handle::ReplyHandle<T>`] is the typed, single-use capability to answer
//! one wait with a `T: Reply`.

pub mod handle;
pub mod kind;
pub mod value;

use self::{handle::ReplyHandle, value::Value};
use crate::boundary::pending::Pending;
use alloc::{string::String, vec::Vec};

/// A type a request may ask for. Sealed: the impls below are the menu, and
/// nothing outside this crate can add to it — which is what lets a driver
/// treat a kind mismatch as unreachable rather than as an error.
pub trait Reply: private::Sealed + Sized {
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
