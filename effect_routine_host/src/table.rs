//! The handle table: machines behind `u64`s, steppable from any thread.
//!
//! Drivers are `Send`, so they live behind a `Mutex` in a `static` and a
//! handle may be stepped from any thread, one at a time; two threads
//! colliding on one handle get [`Error::Busy`], not a race. Hosts may pool,
//! and a machine's steps migrate between threads.
//!
//! A panicking routine is caught, removed, and reported as
//! [`Error::Panicked`]; the host must not step that handle again. Handles are
//! never `0` and never reused, so a stale one is [`Error::BadHandle`] rather
//! than a fault.

use crate::{Error, Machine, Status};
use effect_routine::{
    driver::{Drive, Driver, Outbox},
    wire::{Encode, HostEffect},
};
use std::{
    collections::HashMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, PoisonError, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
};

/// What the table holds: a machine of any effect type, behind its byte
/// layer.
trait Stepper {
    fn step(&mut self, input: &[u8]) -> Result<(Vec<u8>, Status), Error>;
}

impl<E: HostEffect, D: Drive<E>> Stepper for Machine<D, E>
where
    E::View: Encode,
{
    fn step(&mut self, input: &[u8]) -> Result<(Vec<u8>, Status), Error> {
        Machine::step(self, input)
    }
}

type Table = HashMap<u64, Arc<Mutex<Box<dyn Stepper + Send>>>>;

static MACHINES: Mutex<Option<Table>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn with_table<T>(f: impl FnOnce(&mut Table) -> T) -> T {
    let mut guard = MACHINES.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// Register a routine. `make` receives the outbox the routine should write
/// into and returns its future, which must be `Send`. Returns a handle, never
/// `0` and never reused, valid on any thread.
pub fn new<E, F, M>(make: M) -> u64
where
    E: HostEffect + Send + 'static,
    E::View: Encode,
    F: Future<Output = ()> + Send + 'static,
    M: FnOnce(Outbox<E>) -> F,
{
    let handle = NEXT.fetch_add(1, Ordering::Relaxed);
    let machine: Box<dyn Stepper + Send> = Box::new(Machine::new(Driver::new(make)));

    with_table(|t| t.insert(handle, Arc::new(Mutex::new(machine))));
    handle
}

/// Feed one input — `Start`, or a reply — and return the effects recorded
/// before the next wait, encoded, plus the routine's status.
///
/// The table lock is not held while the routine runs; the machine's own lock
/// is `try_lock`ed so a concurrent step is [`Error::Busy`] rather than a
/// wait. A panicking routine is removed and reported as [`Error::Panicked`];
/// the removal happens before its lock is released, so no other thread can
/// observe the poisoned machine in between.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`], or whatever
/// [`Machine::step`] returns.
pub fn step(handle: u64, input: &[u8]) -> Result<(Vec<u8>, Status), Error> {
    let machine = with_table(|t| t.get(&handle).cloned()).ok_or(Error::BadHandle)?;

    let mut guard = match machine.try_lock() {
        Ok(g) => g,
        Err(TryLockError::WouldBlock) => return Err(Error::Busy),
        Err(TryLockError::Poisoned(_)) => return Err(Error::Panicked),
    };

    if let Ok(result) = catch_unwind(AssertUnwindSafe(|| guard.step(input))) {
        return result;
    }

    with_table(|t| t.remove(&handle));
    drop(guard);
    Err(Error::Panicked)
}

/// Drop a routine, including any request it had outstanding.
///
/// # Errors
///
/// [`Error::BadHandle`] if there is no such routine.
pub fn free(handle: u64) -> Result<(), Error> {
    with_table(|t| t.remove(&handle))
        .map(drop)
        .ok_or(Error::BadHandle)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::tests::{Echo, Effect, reply_str_record};
    use effect_routine::{run::Run, wire::Writer};

    #[test]
    fn round_trip_across_threads() {
        let h = new(|outbox| Echo(outbox).run());

        let (bytes, status) = step(h, &[0]).expect("start");
        assert_eq!(status, Status::Awaiting);
        let mut want = Writer::new();
        want.u8(1);
        want.u64(1);
        assert_eq!(bytes, want.finish());

        let input = reply_str_record(1, "far");
        let elsewhere = std::thread::spawn(move || step(h, &input))
            .join()
            .expect("thread");
        let (bytes, status) = elsewhere.expect("stepped on another thread");
        assert_eq!(status, Status::Complete);
        let mut want = Writer::new();
        want.u8(2);
        want.str("far");
        assert_eq!(bytes, want.finish());

        free(h).expect("free");
        assert_eq!(free(h), Err(Error::BadHandle));
        assert_eq!(step(h, &[0]), Err(Error::BadHandle));
    }

    #[test]
    fn a_panicking_routine_is_removed() {
        let h = new(|outbox: Outbox<Effect>| async move {
            drop(outbox);
            panic!("routine bug");
        });

        // Silence the panic message the default hook would print.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = step(h, &[0]);
        std::panic::set_hook(hook);

        assert_eq!(result, Err(Error::Panicked));
        assert_eq!(
            step(h, &[0]),
            Err(Error::BadHandle),
            "removed from the table"
        );
    }
}
