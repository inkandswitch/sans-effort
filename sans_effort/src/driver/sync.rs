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
//! `AtomicU64` and `Arc` are `core`'s and `alloc`'s unless the
//! `portable-atomic` feature is on, for targets without native CAS or 64-bit
//! atomics — where `alloc::sync` does not exist at all.

#[cfg(not(any(feature = "std", feature = "spin")))]
compile_error!(
    "sans_effort needs a lock: enable the `std` feature (default), or `spin` for no_std \
     (add `portable-atomic` or `critical-section` on targets without CAS atomics)"
);

#[cfg(feature = "std")]
pub(super) struct Mutex<T>(std::sync::Mutex<T>);

#[cfg(feature = "std")]
impl<T> Mutex<T> {
    pub(super) const fn new(value: T) -> Self {
        Self(std::sync::Mutex::new(value))
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(all(not(feature = "std"), feature = "spin"))]
pub(super) type Mutex<T> = spin::Mutex<T>;

#[cfg(feature = "portable-atomic")]
pub(super) use portable_atomic::AtomicU64;

#[cfg(feature = "portable-atomic")]
pub(super) use portable_atomic_util::Arc;

#[cfg(not(feature = "portable-atomic"))]
pub(super) use core::sync::atomic::AtomicU64;

#[cfg(not(feature = "portable-atomic"))]
pub(super) use alloc::sync::Arc;
