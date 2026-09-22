//! Why a call failed.

use crate::code;
use sans_effort::{boundary::codec::DecodeError, reply::kind::Kind};

/// Why a call failed. [`code`](Self::code) is its wire form.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    /// Unknown or freed handle.
    #[error("unknown or freed handle")]
    BadHandle,
    /// A second `start`, or a reply to an id nothing awaits.
    #[error("a second start, or a reply to an id nothing awaits")]
    BadInput,
    /// A reply record that is not one well-formed record. Crosses the ABI as
    /// `BAD_INPUT`, like [`BadInput`](Self::BadInput); kept apart here so a
    /// Rust caller can see why.
    #[error("malformed reply record: {0}")]
    Malformed(#[from] DecodeError),
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
            Error::BadInput | Error::Malformed(_) => code::BAD_INPUT,
            Error::Busy => code::BUSY,
            Error::Finished => code::FINISHED,
            Error::Panicked => code::PANICKED,
            Error::WrongKind { .. } => code::WRONG_KIND,
        }
    }
}
