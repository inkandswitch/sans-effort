//! C ABI over the greeter: `new`, `start`, `reply`, `free`.
//!
//! The vocabulary and the reifying context are `greeter_boundary`; the handle
//! table and the type check on replies are `sans_effort_host`. This crate
//! is the `extern "C"` skin over both: one wrapper per function, each a line
//! plus the `unsafe` needed to touch foreign memory — building a slice from a
//! host pointer, writing the out-pointers, reclaiming a buffer. Three blocks,
//! and no mechanism.
//!
//! `ABI.md` at the repository root is the contract; `../python/main.py` is a
//! host that speaks it with a byte buffer and no library.

use greeter_boundary::{Ctx, Full, Quiet};
use routines::{fanout::Fanout, greeter::Greeter, ticker::Ticker};
use sans_effort::run::Run;
use sans_effort_host::{code::code_of, error::Error, status::Status, table};

/// Create a greeter. Returns its handle (never 0), valid on any thread; two
/// threads driving it at once get `BUSY`.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new() -> u64 {
    table::new(|outbox| Greeter::new(Ctx::<Full>::new(outbox)).run())
}

/// Create the fan-out greeter: two requests per batch, replied to in any
/// order. Same handle type, same calls, same codec.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_fanout() -> u64 {
    table::new(|outbox| Fanout::new(Ctx::<Full>::new(outbox)).run())
}

/// Create a three-tick ticker under the `Quiet` vocabulary. It only ever
/// emits tags 4 and 5, and a host can know that from the type alone.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_ticker() -> u64 {
    table::new(|outbox| Ticker::new(Ctx::<Quiet>::new(outbox), 3).run())
}

/// Run the routine to its first wait and receive the effects it recorded plus
/// a status code. Valid once per handle. On error nothing is written.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid for writes. Free the buffer with
/// [`greeter_buf_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_start(
    handle: u64,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    // SAFETY: caller contract.
    unsafe { deliver(table::start(handle), out_ptr, out_len) }
}

/// Deliver one reply record — `kind · id · payload` — and receive the effects
/// recorded before the next wait plus a status code. On error nothing is
/// written.
///
/// # Safety
///
/// `in_ptr` must be valid for `in_len` bytes, or null with `in_len == 0`;
/// `out_ptr` and `out_len` must be valid for writes. Free the buffer with
/// [`greeter_buf_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_reply(
    handle: u64,
    in_ptr: *const u8,
    in_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    let record = if in_ptr.is_null() {
        &[][..]
    } else {
        // SAFETY: caller contract. `from_raw_parts` needs a non-null pointer
        // even for zero bytes, and a host may pass `(NULL, 0)` for empty
        // input, so that case is taken above.
        unsafe { std::slice::from_raw_parts(in_ptr, in_len) }
    };

    // SAFETY: caller contract.
    unsafe { deliver(table::reply(handle, record), out_ptr, out_len) }
}

/// Drop a greeter, including any request it had outstanding.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_free(handle: u64) -> i32 {
    code_of(table::free(handle))
}

/// Free a buffer previously returned by [`greeter_start`] or
/// [`greeter_reply`].
///
/// # Safety
///
/// `ptr` and `len` must be exactly what a previous call handed out, used
/// once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_buf_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }

    // SAFETY: reconstructing the `Box<[u8]>` leaked in `deliver`.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// Hand a result to the host: on success, the buffer through the two
/// out-pointers and the status as the return code; on error, the error's
/// code and nothing written.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid for writes. Free with
/// [`greeter_buf_free`].
unsafe fn deliver(
    result: Result<(Vec<u8>, Status), Error>,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    match result {
        Ok((bytes, status)) => {
            let boxed = bytes.into_boxed_slice();
            // SAFETY: caller guarantees both out-pointers are writable.
            unsafe {
                *out_len = boxed.len();
                *out_ptr = Box::into_raw(boxed).cast::<u8>();
            }
            status.code()
        }
        Err(e) => e.code(),
    }
}
