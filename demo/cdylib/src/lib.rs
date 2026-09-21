//! C ABI over the greeter, in sans-io's shape: `new`, `step`, `free`.
//!
//! The vocabulary and the encoding are the `greeter` crate; the stepping,
//! the handle table, and the type check on replies are `effect_routine_host`.
//! This crate is the `extern "C"` skin over both: one wrapper per function,
//! each a line plus the `unsafe` needed to touch foreign memory — building a
//! slice from a host pointer, writing the out-pointers, reclaiming a buffer.
//! Three blocks, and no mechanism.
//!
//! `ABI.md` at the repository root is the contract; `../python/main.py` is a
//! host that speaks it with a byte buffer and no library.

use effect_routine_host::{code_of, table};

/// Create a greeter. Returns its handle (never 0), valid on any thread; two
/// threads stepping it at once get `BUSY`.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new() -> u64 {
    table::new(greeter::greeter)
}

/// Create the fan-out greeter: two requests per batch, replied to in any
/// order. Same handle type, same `step`, same codec.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_fanout() -> u64 {
    table::new(greeter::fanout)
}

/// Feed one input — `0` to start, then one reply per awaited effect — and
/// receive the effects recorded before the next wait plus a status code. On
/// error nothing is written.
///
/// # Safety
///
/// `in_ptr` must be valid for `in_len` bytes, or null with `in_len == 0`;
/// `out_ptr` and `out_len` must be valid for writes. Free the buffer with
/// [`greeter_buf_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_step(
    handle: u64,
    in_ptr: *const u8,
    in_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    let input = if in_ptr.is_null() {
        &[][..]
    } else {
        // SAFETY: caller contract. `from_raw_parts` needs a non-null pointer
        // even for zero bytes, and a host may pass `(NULL, 0)` for empty
        // input, so that case is taken above.
        unsafe { std::slice::from_raw_parts(in_ptr, in_len) }
    };

    match table::step(handle, input) {
        Ok((bytes, status)) => {
            // SAFETY: caller contract.
            unsafe { give(bytes, out_ptr, out_len) };
            status.code()
        }
        Err(e) => e.code(),
    }
}

/// Drop a greeter, including any request it had outstanding.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_free(handle: u64) -> i32 {
    code_of(table::free(handle))
}

/// Free a buffer previously returned by [`greeter_step`].
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

    // SAFETY: reconstructing the `Box<[u8]>` leaked in `give`.
    drop(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) });
}

/// Hand a Rust-allocated buffer to the host through two out-pointers.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid for writes. Free with
/// [`greeter_buf_free`].
unsafe fn give(bytes: Vec<u8>, out_ptr: *mut *mut u8, out_len: *mut usize) {
    let boxed = bytes.into_boxed_slice();
    // SAFETY: caller guarantees both out-pointers are writable.
    unsafe {
        *out_len = boxed.len();
        *out_ptr = Box::into_raw(boxed).cast::<u8>();
    }
}
