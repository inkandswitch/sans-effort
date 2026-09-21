//! The handle table: machines behind `u64`s, drivable from any thread.
//!
//! Drivers are `Send`, so they live behind a `Mutex` in a `static` and a
//! handle may be driven from any thread, one at a time; two threads colliding
//! on one handle get [`Error::Busy`], not a race. Hosts may pool, and a
//! machine's calls migrate between threads.
//!
//! A panicking routine is caught, removed, and reported as
//! [`Error::Panicked`]; the host must not touch that handle again. Handles
//! are never `0` and never reused, so a stale one is [`Error::BadHandle`]
//! rather than a fault.

use crate::{Error, Machine, Status};
use effect_routine::{
    driver::{Driver, outbox::Outbox},
    wire::{Encode, HostEffect},
};
use std::{
    collections::HashMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex, MutexGuard, PoisonError, TryLockError,
        atomic::{AtomicU64, Ordering},
    },
};

/// What the table holds: a machine of any effect type, behind its byte
/// layer.
trait Encoded {
    fn start(&mut self) -> Result<(Vec<u8>, Status), Error>;
    fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error>;
}

impl<E: HostEffect> Encoded for Machine<E>
where
    E::View: Encode,
{
    fn start(&mut self) -> Result<(Vec<u8>, Status), Error> {
        Machine::start_encoded(self)
    }

    fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
        Machine::reply_encoded(self, record)
    }
}

type Boxed = Box<dyn Encoded + Send>;
type Table = HashMap<u64, Arc<Mutex<Boxed>>>;

static MACHINES: Mutex<Option<Table>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn with_table<T>(f: impl FnOnce(&mut Table) -> T) -> T {
    let mut guard = MACHINES.lock().unwrap_or_else(PoisonError::into_inner);
    f(guard.get_or_insert_with(HashMap::new))
}

/// Register a routine. `make` receives the outbox the routine's context
/// should write into and returns the routine's future, which must be `Send`.
/// Returns a handle, never `0` and never reused, valid on any thread.
pub fn new<E, F, M>(make: M) -> u64
where
    E: HostEffect + Send + 'static,
    E::View: Encode,
    F: Future<Output = ()> + Send + 'static,
    M: FnOnce(Outbox<E>) -> F,
{
    let handle = NEXT.fetch_add(1, Ordering::Relaxed);
    let machine: Boxed = Box::new(Machine::new(Driver::new(make)));

    with_table(|t| t.insert(handle, Arc::new(Mutex::new(machine))));
    handle
}

/// Run the routine to its first wait: the effects it recorded, encoded, plus
/// its status. Valid once per handle.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`], or whatever
/// [`Machine::start_encoded`] returns.
pub fn start(handle: u64) -> Result<(Vec<u8>, Status), Error> {
    guarded(handle, |m| m.start())
}

/// Deliver one reply record and run the routine to its next wait: the
/// effects it recorded, encoded, plus its status.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`], or whatever
/// [`Machine::reply_encoded`] returns.
pub fn reply(handle: u64, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
    guarded(handle, |m| m.reply(record))
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

/// Look the handle up, take its lock without waiting, and run `f` with the
/// routine's panics caught.
///
/// The table lock is not held while the routine runs; the machine's own lock
/// is `try_lock`ed so a concurrent call is [`Error::Busy`] rather than a wait.
/// A panicking routine is removed and reported as [`Error::Panicked`]; the
/// removal happens before its lock is released, so no other thread can
/// observe the poisoned machine in between.
fn guarded<T>(
    handle: u64,
    f: impl FnOnce(&mut MutexGuard<'_, Boxed>) -> Result<T, Error>,
) -> Result<T, Error> {
    let machine = with_table(|t| t.get(&handle).cloned()).ok_or(Error::BadHandle)?;

    let mut guard = match machine.try_lock() {
        Ok(g) => g,
        Err(TryLockError::WouldBlock) => return Err(Error::Busy),
        Err(TryLockError::Poisoned(_)) => return Err(Error::Panicked),
    };

    if let Ok(result) = catch_unwind(AssertUnwindSafe(|| f(&mut guard))) {
        return result;
    }

    with_table(|t| t.remove(&handle));
    drop(guard);
    Err(Error::Panicked)
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

        let (bytes, status) = start(h).expect("start");
        assert_eq!(status, Status::Awaiting);
        let mut want = Writer::new();
        want.u8(1);
        want.u64(1);
        assert_eq!(bytes, want.finish());

        let record = reply_str_record(1, "far");
        let elsewhere = std::thread::spawn(move || reply(h, &record))
            .join()
            .expect("thread");
        let (bytes, status) = elsewhere.expect("replied on another thread");
        assert_eq!(status, Status::Complete);
        let mut want = Writer::new();
        want.u8(2);
        want.str("far");
        assert_eq!(bytes, want.finish());

        free(h).expect("free");
        assert_eq!(free(h), Err(Error::BadHandle));
        assert_eq!(start(h), Err(Error::BadHandle));
        assert_eq!(reply(h, &reply_str_record(1, "x")), Err(Error::BadHandle));
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
        let result = start(h);
        std::panic::set_hook(hook);

        assert_eq!(result, Err(Error::Panicked));
        assert_eq!(start(h), Err(Error::BadHandle), "removed from the table");
    }
}
