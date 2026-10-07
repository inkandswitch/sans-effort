//! C ABI over the greeter: `abi_version`, `new`, `resume`, `reply`, `wakes`,
//! `free` — and, for host checks, `record` and `record_finish`, so a run a
//! foreign host made can be replayed in Rust (`src/bin/replay.rs`).
//!
//! The vocabulary and the reifying context are `greeter_boundary`; the handle
//! table and the type check on replies are `sans-effort-host`. This crate
//! is the `extern "C"` binding over both: one wrapper per function, each a line
//! plus the `unsafe` needed to touch foreign memory — building a slice from a
//! host pointer, writing the out-pointers, reclaiming a buffer. Three blocks,
//! and no mechanism.
//!
//! `ABI.md` at the repository root is the contract; `../python/main.py` is a
//! host that speaks it with a byte buffer and no library.

use greeter_boundary::{Full, Quiet};
use routines::{
    deadline::Deadline, fanout::Fanout, faults::deadlock::Deadlock, front_desk::FrontDesk,
    greeter::Greeter, journal::Journal, ping_pong::PingPong, ring::Ring, ticker::Ticker,
};
use sans_effort_core::{boundary::codec::Encode, driver::status::Status, step::Step};
use sans_effort_effects::ctx::Ctx;
use sans_effort_host::{
    contract::{BAD_HANDLE, OK, REVISION, code_of, status_code},
    error::Error,
    record::{Recorder, record},
    table,
};
use std::{
    collections::HashMap,
    sync::{
        LazyLock, Mutex, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
};

/// The revision of `ABI.md` this binding speaks. Check it before `new`.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_abi_version() -> u8 {
    REVISION
}

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

/// Create three rounds of ping-pong. Its first batch spawns a child (tag 6):
/// resume it to begin it, then resume whichever of the two a `woke` frame names
/// until both complete.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_ping_pong() -> u64 {
    table::new(|outbox| PingPong::new(Ctx::<Full>::new(outbox), 3).run())
}

/// Create a ring of 16 routines passing a counter 250 times around: 4000 hops,
/// every one a message between two machines that no effect reports — only
/// `woke` frames tell the host whom to resume.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_ring() -> u64 {
    table::new(|outbox| Ring::new(Ctx::<Full>::new(outbox), 16, 250).run())
}

/// Create a deadline: two workers, each raced against a sleep. The quick
/// one wins, so the sleep's request closes — a closed frame — and the host
/// should stop its timer; the slow one loses, then answers late.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_deadline() -> u64 {
    table::new(|outbox| Deadline::new(Ctx::<Full>::new(outbox)).run())
}

/// Create a deadlock: two machines, each waiting for the other to go first.
/// Nothing will ever wake either; a host should report it, not hang.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_deadlock() -> u64 {
    table::new(|outbox| Deadlock::new(Ctx::<Full>::new(outbox)).run())
}

/// Create a front desk: it reads names and spawns a pinned clerk per name
/// (tag 7). Resume each clerk first on the thread that will drive it from then
/// on.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_front_desk() -> u64 {
    table::new(|outbox| FrontDesk::new(Ctx::<Full>::new(outbox)).run())
}

/// Create a journal that appends three entries: it asks for a variable, a
/// file, the time, and random bytes (tags 8–12), and writes the file back.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_new_journal() -> u64 {
    table::new(|outbox| Journal::new(Ctx::<Full>::new(outbox), 3).run())
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

/// Run the routine to its next wait without delivering anything, and receive
/// the effects it recorded plus a status code. The first call begins it — a
/// pinned child is built on the calling thread, which then owns it. After
/// that, for an `IDLE` routine a `woke` frame named, once something it waits on
/// may have changed; harmless when nothing has. On error nothing is written.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid for writes. Free the buffer with
/// [`greeter_buf_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_resume(
    handle: u64,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    // SAFETY: caller contract.
    unsafe { deliver(table::resume(handle), out_ptr, out_len) }
}

/// Receive the machines woken outside any call — a receiver whose sender was
/// dropped by `free`, say — as `woke` frames. Call it when there is nothing
/// else to do. Always `OK`.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid for writes. Free the buffer with
/// [`greeter_buf_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_wakes(out_ptr: *mut *mut u8, out_len: *mut usize) -> i32 {
    // SAFETY: caller contract.
    unsafe { write_out(table::wakes(), out_ptr, out_len) };
    OK
}

/// Begin recording `handle` and every machine it creates: each later
/// `resume`, `reply`, and `free` on them, with its outcome. Returns the
/// recording's number, never 0, for [`greeter_record_finish`].
#[unsafe(no_mangle)]
pub extern "C" fn greeter_record(handle: u64) -> u32 {
    let recording = NEXT_RECORDING.fetch_add(1, Ordering::Relaxed);
    RECORDINGS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(recording, record(handle));
    recording
}

/// Stop a recording and hand the host its log, encoded: `OK`, or
/// `BAD_HANDLE` for a number that is not a recording in progress. Save the
/// bytes to a file and replay them with
/// `cargo run -p greeter_cdylib --bin replay -- FILE MODE`.
///
/// # Safety
///
/// `out_ptr` and `out_len` must be valid for writes. Free the buffer with
/// [`greeter_buf_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn greeter_record_finish(
    recording: u32,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    let recorder = RECORDINGS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&recording);
    let Some(recorder) = recorder else {
        return BAD_HANDLE;
    };
    // SAFETY: caller contract.
    unsafe { write_out(recorder.finish().to_bytes(), out_ptr, out_len) };
    OK
}

/// Drop a greeter, including any request it had outstanding.
#[unsafe(no_mangle)]
pub extern "C" fn greeter_free(handle: u64) -> i32 {
    code_of(table::free(handle))
}

/// Free a buffer previously returned by [`greeter_resume`] or
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

/// Recordings begun and not finished, by number.
static RECORDINGS: LazyLock<Mutex<HashMap<u32, Recorder>>> = LazyLock::new(Mutex::default);

/// The next recording's number.
static NEXT_RECORDING: AtomicU32 = AtomicU32::new(1);

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
            // SAFETY: caller guarantees both out-pointers are writable.
            unsafe { write_out(bytes, out_ptr, out_len) };
            status_code(status)
        }
        Err(e) => e.code(),
    }
}

/// Hand `bytes` to the host through the out-pointers.
///
/// # Safety
///
/// Both out-pointers must be valid for writes.
unsafe fn write_out(bytes: Vec<u8>, out_ptr: *mut *mut u8, out_len: *mut usize) {
    let boxed = bytes.into_boxed_slice();
    // SAFETY: caller guarantees both out-pointers are writable.
    unsafe {
        *out_len = boxed.len();
        *out_ptr = Box::into_raw(boxed).cast::<u8>();
    }
}
