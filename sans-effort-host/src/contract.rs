//! The numbers in the contract a foreign host assumes (`ABI.md` at the
//! repository root): its revision, the kinds of effect frame, and the status
//! and error codes. The functions a host calls are the application's binding,
//! not this crate.
//!
//! For codes, non-negative is a status and negative is an error. Plain
//! constants so a host can copy them; `ABI.md` is the table in prose.

use crate::error::Error;

/// The revision of `ABI.md` this crate implements. A binding returns it from
/// `<prefix>_abi_version()`; a host checks it once, before `new`. One number,
/// bumped whenever a host written against the previous text could misbehave
/// against a binding written against the new one. Tag tables are outside it.
pub const REVISION: u8 = 0;

/// A frame holding an effect that awaits no reply. A host may skip one whose
/// tag it does not know.
pub const FRAME_TELL: u8 = 1;
/// A frame holding an effect that awaits a reply. A host must refuse to
/// continue on one whose tag it does not know: nobody else will answer it.
pub const FRAME_ASK: u8 = 2;
/// A frame holding one `u64` request id the routine no longer needs a reply
/// to. The host may stop that work; a late reply is refused as [`STALE`].
pub const FRAME_CLOSED: u8 = 3;

/// Success, for calls that carry no status (`free`).
pub const OK: i32 = 0;
/// Suspended; at least one effect in the batch awaits a reply. Shares
/// `0` with [`OK`].
pub const AWAITING: i32 = 0;
/// Ran to completion.
pub const COMPLETE: i32 = 1;
/// Suspended with no request outstanding: waiting on something inside the
/// process, such as a channel another routine sends on. `resume` it once that
/// may have changed; resuming early is harmless.
pub const IDLE: i32 = 2;

/// Another thread is inside `start`, `reply`, or `resume` for this handle.
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
/// A second `start`, or a reply to an id that was never issued.
pub const BAD_INPUT: i32 = -6;
/// A reply record that does not parse: a bug in the host's encoder.
pub const MALFORMED: i32 = -7;
/// A reply to an id that was issued but is no longer awaited: already
/// answered, or abandoned by the routine. Harmless; nothing changed.
pub const STALE: i32 = -8;
/// The handle names a pinned machine, started on another thread; every call
/// for it must come from that thread. Nothing changed: route the call there.
pub const WRONG_THREAD: i32 = -9;

/// The wire code of a call that has no [`Status`](crate::status::Status): [`OK`] or the
/// error's code.
#[must_use]
pub fn code_of(result: Result<(), Error>) -> i32 {
    result.map_or_else(Error::code, |()| OK)
}
