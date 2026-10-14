//! Why a call failed.

use crate::contract;
use sans_effort_core::{boundary::codec::DecodeError, reply::kind::Kind};

/// Why a call failed. [`code`](Self::code) is its wire form.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// Unknown or freed handle.
    #[error("unknown or freed handle")]
    BadHandle,
    /// A reply to an id that was never issued.
    #[error("a reply to an id that was never issued")]
    BadInput,
    /// A reply record that does not parse: a bug in the host's encoder.
    #[error("malformed reply record")]
    Malformed(#[from] DecodeError),
    /// A reply to an id that was issued but is no longer awaited: already
    /// answered, or abandoned by the routine. Nothing changed.
    #[error("request {id} is no longer awaited")]
    Stale {
        /// The request replied to.
        id: u64,
    },
    /// The reply's kind is not what request `id` asked for. The request is
    /// still outstanding.
    #[error("request {id} awaits a {expected:?} reply; a {got:?} was sent")]
    WrongKind {
        /// The request replied to.
        id: u64,
        /// What it asked for.
        expected: Kind,
        /// What arrived.
        got: Kind,
    },
    /// Another thread is inside `resume` or `reply` for this
    /// handle.
    #[error("another thread is driving this routine")]
    Busy,
    /// The handle names a pinned machine first resumed on another thread; every
    /// call for it must come from that thread. Nothing changed.
    #[error("this routine is pinned to another thread")]
    WrongThread,
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
            Error::BadHandle => contract::BAD_HANDLE,
            Error::BadInput => contract::BAD_INPUT,
            Error::Malformed(_) => contract::MALFORMED,
            Error::Stale { .. } => contract::STALE,
            Error::Busy => contract::BUSY,
            Error::Finished => contract::FINISHED,
            Error::Panicked => contract::PANICKED,
            Error::WrongKind { .. } => contract::WRONG_KIND,
            Error::WrongThread => contract::WRONG_THREAD,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use core::error::Error as _;

    /// No error's message repeats its cause: a report that walks `source()`
    /// prints each cause once.
    #[test]
    fn no_error_repeats_its_source() {
        let causes = [
            DecodeError::Truncated {
                needed: 8,
                available: 2,
            },
            DecodeError::TooLong { len: 7 },
            DecodeError::InvalidUtf8 { valid_up_to: 3 },
            DecodeError::UnknownTag { tag: 9 },
            DecodeError::TrailingBytes { remaining: 1 },
        ];
        let errors = causes.map(Error::Malformed).into_iter().chain([
            Error::BadHandle,
            Error::BadInput,
            Error::Stale { id: 1 },
            Error::WrongKind {
                id: 1,
                expected: Kind::Unit,
                got: Kind::Str,
            },
            Error::Busy,
            Error::WrongThread,
            Error::Finished,
            Error::Panicked,
        ]);
        for error in errors {
            if let Some(source) = error.source() {
                let (message, cause) = (error.to_string(), source.to_string());
                assert!(!message.contains(&cause), "{message:?} repeats {cause:?}");
            }
        }
    }
}
