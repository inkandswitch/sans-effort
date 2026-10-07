//! The synchronisation primitives the driver needs, and where they come from.
//!
//! Under the `std` feature the lock is `std::sync::Mutex`, with poisoning
//! recovered: a poisoned outbox means the routine panicked mid-poll, and the
//! driver has already dropped that routine, so the data behind the lock is
//! consistent. Under `spin` alone it is `spin::Mutex`, which needs CAS
//! atomics on the target unless the `portable-atomic` feature supplies them.
//! With neither feature there is no lock to build, and the crate says so at
//! compile time rather than failing on a missing type somewhere inside.
//!
//! Either way the lock is never contended — the host polls one driver at a
//! time — and exists to give the outbox a `Sync` impl and a happens-before
//! edge between a `reply` on one thread and the next poll on another.
//!
//! The atomics, `Arc`, and `Wake` are `core`'s and `alloc`'s unless the
//! `portable-atomic` feature is on, for targets without native CAS or 64-bit
//! atomics — where `alloc::sync` does not exist at all.
//!
//! Under `--cfg sans_effort_loom` (`test:loom`) the lock and the waker's flag are loom's,
//! so its models (`tests/loom.rs`) explore their interleavings. `Arc` stays
//! `alloc`'s — `Waker::from` takes no other — and so does the driver-id
//! counter, a `static` loom's atomics cannot be.

#[cfg(all(
    sans_effort_loom,
    any(not(feature = "std"), feature = "portable-atomic")
))]
compile_error!("loom models need the `std` feature, and native atomics (no `portable-atomic`)");

#[cfg(not(any(feature = "std", feature = "spin")))]
compile_error!(
    "sans-effort-core needs a lock: enable the `std` feature (default), or `spin` for no_std \
     (add `portable-atomic` or `critical-section` on targets without CAS atomics)"
);

#[cfg(all(feature = "std", not(sans_effort_loom)))]
pub(crate) struct Mutex<T>(std::sync::Mutex<T>);

#[cfg(all(feature = "std", not(sans_effort_loom)))]
impl<T> Mutex<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }

    pub(crate) fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(sans_effort_loom)]
pub(crate) struct Mutex<T>(loom::sync::Mutex<T>);

#[cfg(sans_effort_loom)]
impl<T> Mutex<T> {
    pub(crate) fn new(value: T) -> Self {
        Self(loom::sync::Mutex::new(value))
    }

    pub(crate) fn lock(&self) -> loom::sync::MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(all(not(feature = "std"), feature = "spin"))]
pub(crate) type Mutex<T> = spin::Mutex<T>;

#[cfg(feature = "portable-atomic")]
pub(super) use portable_atomic::{AtomicBool, AtomicU64};

#[cfg(feature = "portable-atomic")]
pub(crate) use portable_atomic_util::Arc;
#[cfg(feature = "portable-atomic")]
pub(super) use portable_atomic_util::task::Wake;

#[cfg(not(any(feature = "portable-atomic", sans_effort_loom)))]
pub(super) use core::sync::atomic::AtomicBool;
#[cfg(not(feature = "portable-atomic"))]
pub(super) use core::sync::atomic::AtomicU64;
#[cfg(sans_effort_loom)]
pub(super) use loom::sync::atomic::AtomicBool;

#[cfg(not(feature = "portable-atomic"))]
pub(crate) use alloc::sync::Arc;
#[cfg(not(feature = "portable-atomic"))]
pub(super) use alloc::task::Wake;
