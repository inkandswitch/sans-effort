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

use crate::{encoded::Encoded, error::Error, machine::Machine, status::Status};
use effect_routine::{
    boundary::{codec::Encode, host_effect::HostEffect},
    driver::outbox::Outbox,
};
use std::{
    collections::HashMap,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, TryLockError},
};

/// What the table holds: a machine of any effect type, behind its byte
/// layer. The table is one `static` for every routine every skin registers,
/// so the effect type is erased here.
trait Stepped {
    fn start(&mut self) -> Result<(Vec<u8>, Status), Error>;
    fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error>;
}

impl<E: HostEffect> Stepped for Encoded<E>
where
    E::View: Encode,
{
    fn start(&mut self) -> Result<(Vec<u8>, Status), Error> {
        Encoded::start(self)
    }

    fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
        Encoded::reply(self, record)
    }
}

/// A machine behind its own lock, shared between the table and whoever is
/// driving it right now. Any effect type: the table is one `static` holding
/// every routine every skin registers, so the machine is type-erased here.
#[derive(Clone)]
struct Entry(Arc<Mutex<Box<dyn Stepped + Send>>>);

impl Entry {
    fn new(machine: Box<dyn Stepped + Send>) -> Self {
        Self(Arc::new(Mutex::new(machine)))
    }

    /// Take the machine's lock without waiting: a concurrent call is
    /// [`Error::Busy`] rather than a wait, and a lock poisoned by an earlier
    /// panic is [`Error::Panicked`].
    fn try_lock(&self) -> Result<MutexGuard<'_, Box<dyn Stepped + Send>>, Error> {
        self.0.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => Error::Busy,
            TryLockError::Poisoned(_) => Error::Panicked,
        })
    }
}

/// The process-wide table: machines by handle, and the next handle to issue.
/// One lock covers both, so a handle is never issued twice.
struct Table {
    machines: HashMap<u64, Entry>,
    next: u64,
}

impl Table {
    fn new() -> Self {
        Self {
            machines: HashMap::new(),
            next: 1,
        }
    }

    /// Store a machine under a fresh handle, never `0`, never reused.
    fn insert(&mut self, machine: Box<dyn Stepped + Send>) -> u64 {
        let handle = self.next;
        self.next += 1;
        self.machines.insert(handle, Entry::new(machine));
        handle
    }

    fn get(&self, handle: u64) -> Option<Entry> {
        self.machines.get(&handle).cloned()
    }

    /// `true` if there was a machine to remove.
    fn remove(&mut self, handle: u64) -> bool {
        self.machines.remove(&handle).is_some()
    }
}

static TABLE: LazyLock<Mutex<Table>> = LazyLock::new(|| Mutex::new(Table::new()));

/// The table, with a poisoned lock recovered: a panic while holding it can
/// only have been in `HashMap` itself, and the routines behind it are
/// untouched.
fn table() -> MutexGuard<'static, Table> {
    TABLE.lock().unwrap_or_else(PoisonError::into_inner)
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
    table().insert(Box::new(Encoded::new(Machine::from_routine(make))))
}

/// Run the routine to its first wait: the effects it recorded, encoded, plus
/// its status. Valid once per handle.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`], or whatever
/// [`Encoded::start`] returns.
pub fn start(handle: u64) -> Result<(Vec<u8>, Status), Error> {
    guarded(handle, |m| m.start())
}

/// Deliver one reply record and run the routine to its next wait: the
/// effects it recorded, encoded, plus its status.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`], or whatever
/// [`Encoded::reply`] returns.
pub fn reply(handle: u64, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
    guarded(handle, |m| m.reply(record))
}

/// Drop a routine, including any request it had outstanding.
///
/// # Errors
///
/// [`Error::BadHandle`] if there is no such routine.
pub fn free(handle: u64) -> Result<(), Error> {
    if table().remove(handle) {
        Ok(())
    } else {
        Err(Error::BadHandle)
    }
}

/// Look the handle up, take its lock without waiting, and run `f` with the
/// routine's panics caught.
///
/// The table lock is not held while the routine runs. A panicking routine is removed and reported as [`Error::Panicked`]; the
/// removal happens before its lock is released, so no other thread can
/// observe the poisoned machine in between.
fn guarded<T>(
    handle: u64,
    f: impl FnOnce(&mut MutexGuard<'_, Box<dyn Stepped + Send>>) -> Result<T, Error>,
) -> Result<T, Error> {
    let machine = table().get(handle).ok_or(Error::BadHandle)?;

    let mut guard = machine.try_lock()?;

    if let Ok(result) = catch_unwind(AssertUnwindSafe(|| f(&mut guard))) {
        return result;
    }

    table().remove(handle);
    drop(guard);
    Err(Error::Panicked)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert their preconditions"
    )]

    use super::*;
    use crate::fixtures::{Echo, Effect, reply_str_record};
    use effect_routine::{boundary::codec::Writer, run::Run};

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
