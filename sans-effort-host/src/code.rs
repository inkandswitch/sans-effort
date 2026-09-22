//! Status and error codes as they cross the ABI.
//!
//! Non-negative is a status, negative is an error. Plain constants so a host
//! can copy them; `ABI.md` at the repository root is the table in prose.

use crate::error::Error;

/// The revision of `ABI.md` this crate implements. A binding returns it from
/// `<prefix>_abi_version()`; a host checks it once, before `new`. One number,
/// bumped whenever a host written against the previous text could misbehave
/// against a binding written against the new one. Tag tables are outside it.
pub const ABI_VERSION: u8 = 0;

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
/// The reply's kind is not what the request asked for. The request is
/// still outstanding; reply again with the right kind.
pub const WRONG_KIND: i32 = -3;
/// The routine panicked and has been removed.
pub const PANICKED: i32 = -4;
/// Unknown or freed handle.
pub const BAD_HANDLE: i32 = -5;
/// Malformed input, a second `start`, or an id nothing awaits.
pub const BAD_INPUT: i32 = -6;

/// The wire code of a call that has no [`Status`](crate::status::Status): [`OK`] or the
/// error's code.
#[must_use]
pub fn code_of(result: Result<(), Error>) -> i32 {
    result.map_or_else(Error::code, |()| OK)
}
