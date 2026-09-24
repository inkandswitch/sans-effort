//! The handle table: machines behind `u64`s.
//!
//! Most machines _migrate_: their drivers are `Send`, so they live behind a
//! `Mutex` in a `static` and a handle may be driven from any thread, one at a
//! time; two threads colliding on one handle get [`Error::Busy`], not a race.
//! Hosts may pool, and a machine's calls migrate between threads.
//!
//! A _pinned_ machine's future need not be `Send`, so it must stay on one
//! thread. It is parked unstarted by [`park_pinned`]; the thread that calls
//! [`start`] builds it and keeps it in its own table, and every later call for
//! it must come from that thread. From any other, a call is
//! [`Error::WrongThread`]: nothing changes, and the host routes the call where
//! it belongs.
//!
//! A machine waiting on something inside the process — a channel another
//! routine sends on — is `IDLE`. When whatever it waits on wakes it, the
//! table reports its handle as a [`FRAME_WOKE`] frame: at the end of the
//! output of the call whose poll caused the wake — the sender's, typically —
//! or, for a wake outside any call (a sender dropped by [`free`], a panic),
//! in the output of [`wakes`]. The host resumes that machine when it chooses.
//!
//! A panicking routine is caught, removed, and reported as
//! [`Error::Panicked`]; the host must not touch that handle again. Handles
//! are never `0` and never reused, so a stale one is [`Error::BadHandle`]
//! rather than a fault.

use crate::{
    contract::FRAME_WOKE,
    encoded::Encoded,
    error::Error,
    machine::{Drive, Machine},
    status::Status,
};
use sans_effort::{
    boundary::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
    },
    driver::{BoxedRoutine, LocalBoxedRoutine, LocalDriver, outbox::Outbox},
};
use std::{
    cell::RefCell,
    collections::HashMap,
    future::Future,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
    sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, TryLockError},
    thread::{self, ThreadId},
};

/// What the tables hold: a machine of any effect type, behind its byte
/// layer. The tables hold every routine every binding registers, so the
/// effect type is erased here.
trait Stepped {
    fn start(&mut self) -> Result<(Vec<u8>, Status), Error>;
    fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error>;
    fn resume(&mut self) -> Result<(Vec<u8>, Status), Error>;
    fn on_wake(&self, hook: Box<dyn Fn() + Send + Sync>);
}

impl<E: HostEffect, D: Drive<E>> Stepped for Encoded<E, D>
where
    E::View: Encode,
{
    fn start(&mut self) -> Result<(Vec<u8>, Status), Error> {
        Encoded::start(self)
    }

    fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
        Encoded::reply(self, record)
    }

    fn resume(&mut self) -> Result<(Vec<u8>, Status), Error> {
        Encoded::resume(self)
    }

    fn on_wake(&self, hook: Box<dyn Fn() + Send + Sync>) {
        Encoded::on_wake(self, hook);
    }
}

/// A pinned child not yet started: its closure is `Send`, its future will not
/// be, so it is built on the thread that starts it.
trait Park {
    fn build(self: Box<Self>) -> Box<dyn Stepped>;
}

struct Parked<E, M> {
    make: M,
    _effect: PhantomData<fn() -> E>,
}

impl<E: HostEffect + 'static, M: FnOnce(Outbox<E>) -> LocalBoxedRoutine> Park for Parked<E, M>
where
    E::View: Encode,
{
    fn build(self: Box<Self>) -> Box<dyn Stepped> {
        Box::new(Encoded::new(Machine::new(LocalDriver::from_boxed(
            self.make,
        ))))
    }
}

/// A migrating machine behind its own lock, shared between the table and
/// whoever is driving it right now.
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

/// A started pinned machine, in its thread's table.
type LocalEntry = Rc<RefCell<Box<dyn Stepped>>>;

/// The process-wide table: migrating machines, parked pinned children, which
/// thread owns each started pinned machine, and the next handle to issue.
/// One lock covers them all, so a handle is never issued twice.
struct Table {
    machines: HashMap<u64, Entry>,
    parked: HashMap<u64, Box<dyn Park + Send>>,
    pinned: HashMap<u64, ThreadId>,
    next: u64,
}

impl Table {
    fn new() -> Self {
        Self {
            machines: HashMap::new(),
            parked: HashMap::new(),
            pinned: HashMap::new(),
            next: 1,
        }
    }

    /// A fresh handle, never `0`, never reused.
    const fn issue(&mut self) -> u64 {
        let handle = self.next;
        self.next += 1;
        handle
    }
}

static TABLE: LazyLock<Mutex<Table>> = LazyLock::new(|| Mutex::new(Table::new()));

thread_local! {
    /// This thread's started pinned machines.
    static LOCAL: RefCell<HashMap<u64, LocalEntry>> = RefCell::new(HashMap::new());

    /// The machines woken during the call this thread is making, if it is
    /// making one: they are reported in that call's output.
    static CALL: RefCell<Option<Vec<u64>>> = const { RefCell::new(None) };
}

/// Machines woken outside any call, until [`wakes`] reports them.
static WAKES: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// The hook every machine's driver calls when it wakes: note `handle` for the
/// current call, or for [`wakes`] if there is none.
fn woke(handle: u64) {
    let noted = CALL.with(|call| {
        call.borrow_mut()
            .as_mut()
            .map(|woken| woken.push(handle))
            .is_some()
    });
    if !noted {
        WAKES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(handle);
    }
}

/// Run one call, collecting the wakes its poll causes: appended to its output
/// as [`FRAME_WOKE`] frames, or — if it fails, and so has no output — queued
/// for [`wakes`].
fn reporting_wakes(
    call: impl FnOnce() -> Result<(Vec<u8>, Status), Error>,
) -> Result<(Vec<u8>, Status), Error> {
    let outer = CALL.with(|woken| woken.replace(Some(Vec::new())));
    let result = call();
    let woken = CALL.with(|woken| woken.replace(outer)).unwrap_or_default();

    match result {
        Ok((mut bytes, status)) => {
            bytes.extend(woke_frames(&woken));
            Ok((bytes, status))
        }
        Err(e) => {
            WAKES
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend(woken);
            Err(e)
        }
    }
}

fn woke_frames(handles: &[u64]) -> Vec<u8> {
    let mut w = Writer::new();
    for handle in handles {
        w.u8(FRAME_WOKE);
        w.bytes(&handle.to_le_bytes());
    }
    w.finish()
}

/// The machines woken outside any call since the last time — a receiver whose
/// sender was dropped by [`free`], say — as [`FRAME_WOKE`] frames. Call it
/// when there is nothing else to do; a host that finds no calls to make, no
/// requests outstanding, and nothing here has reached the end, or a stall.
#[must_use]
pub fn wakes() -> Vec<u8> {
    let woken = core::mem::take(&mut *WAKES.lock().unwrap_or_else(PoisonError::into_inner));
    woke_frames(&woken)
}

/// The table, with a poisoned lock recovered: a panic while holding it can
/// only have been in `HashMap` itself, and the routines behind it are
/// untouched.
fn table() -> MutexGuard<'static, Table> {
    TABLE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Register a migrating routine. `make` receives the outbox the routine's
/// context should write into and returns the routine's future, which must be
/// `Send`. Returns a handle, never `0` and never reused, valid on any thread.
pub fn new<
    E: HostEffect + Send + 'static,
    F: Future<Output = ()> + Send + 'static,
    M: FnOnce(Outbox<E>) -> F,
>(
    make: M,
) -> u64
where
    E::View: Encode,
{
    insert(Box::new(Encoded::new(Machine::from_routine(make))))
}

/// As [`new`], for a routine already boxed — a spawned child, say — so it is
/// not boxed twice.
pub fn new_boxed<E: HostEffect + Send + 'static, M: FnOnce(Outbox<E>) -> BoxedRoutine>(
    make: M,
) -> u64
where
    E::View: Encode,
{
    insert(Box::new(Encoded::new(Machine::from_boxed(make))))
}

/// Park a pinned routine, unstarted. Its future is built by whichever thread
/// calls [`start`] for the returned handle, and stays on that thread. Until
/// then, [`free`] from any thread drops it.
pub fn park_pinned<
    E: HostEffect + 'static,
    M: FnOnce(Outbox<E>) -> LocalBoxedRoutine + Send + 'static,
>(
    make: M,
) -> u64
where
    E::View: Encode,
{
    let mut table = table();
    let handle = table.issue();
    table.parked.insert(
        handle,
        Box::new(Parked {
            make,
            _effect: PhantomData,
        }),
    );
    handle
}

fn insert(machine: Box<dyn Stepped + Send>) -> u64 {
    let mut table = table();
    let handle = table.issue();
    machine.on_wake(Box::new(move || woke(handle)));
    table.machines.insert(handle, Entry::new(machine));
    handle
}

/// Run the routine to its first wait: the effects it recorded, encoded, plus
/// its status. Valid once per handle. For a parked pinned routine, this is
/// where it is built, on the calling thread, which then owns it.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`],
/// [`Error::WrongThread`], or whatever [`Encoded::start`] returns.
pub fn start(handle: u64) -> Result<(Vec<u8>, Status), Error> {
    let parked = {
        let mut table = table();
        match table.parked.remove(&handle) {
            Some(parked) => {
                table.pinned.insert(handle, thread::current().id());
                Some(parked)
            }
            None => None,
        }
    };

    match parked {
        Some(parked) => {
            let built = catch_unwind(AssertUnwindSafe(|| parked.build()));
            let Ok(machine) = built else {
                table().pinned.remove(&handle);
                return Err(Error::Panicked);
            };
            machine.on_wake(Box::new(move || woke(handle)));
            LOCAL.with(|local| {
                local
                    .borrow_mut()
                    .insert(handle, Rc::new(RefCell::new(machine)));
            });
            reporting_wakes(|| guarded(handle, |m| m.start()))
        }
        None => reporting_wakes(|| guarded(handle, |m| m.start())),
    }
}

/// Deliver one reply record and run the routine to its next wait: the
/// effects it recorded, encoded, plus its status.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`],
/// [`Error::WrongThread`], [`Error::BadInput`] for a parked routine not yet
/// started, or whatever [`Encoded::reply`] returns.
pub fn reply(handle: u64, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
    reporting_wakes(|| guarded(handle, |m| m.reply(record)))
}

/// Run the routine to its next wait without delivering anything: the effects
/// it recorded, encoded, plus its status. For an [`Status::Idle`] routine,
/// once something it waits on may have changed; harmless when nothing has.
///
/// # Errors
///
/// As [`reply`], except that the machine's own errors are those of
/// [`Encoded::resume`].
pub fn resume(handle: u64) -> Result<(Vec<u8>, Status), Error> {
    reporting_wakes(|| guarded(handle, |m| m.resume()))
}

/// Drop a routine, including any request it had outstanding. A parked pinned
/// routine may be freed from any thread; a started one only from its own.
///
/// # Errors
///
/// [`Error::BadHandle`] if there is no such routine; [`Error::WrongThread`]
/// for a pinned routine started on another thread.
pub fn free(handle: u64) -> Result<(), Error> {
    let mut table = table();

    if table.machines.remove(&handle).is_some() || table.parked.remove(&handle).is_some() {
        return Ok(());
    }

    match table.pinned.get(&handle) {
        Some(owner) if *owner == thread::current().id() => {
            table.pinned.remove(&handle);
            drop(table);
            LOCAL.with(|local| local.borrow_mut().remove(&handle));
            Ok(())
        }
        Some(_) => Err(Error::WrongThread),
        None => Err(Error::BadHandle),
    }
}

/// Where a handle's machine is, from the calling thread.
enum Found {
    Migrating(Entry),
    Here(LocalEntry),
}

fn find(handle: u64) -> Result<Found, Error> {
    let table = table();

    if let Some(entry) = table.machines.get(&handle) {
        return Ok(Found::Migrating(entry.clone()));
    }

    if table.parked.contains_key(&handle) {
        return Err(Error::BadInput);
    }

    match table.pinned.get(&handle) {
        Some(owner) if *owner == thread::current().id() => {
            drop(table);
            LOCAL
                .with(|local| local.borrow().get(&handle).cloned())
                .map(Found::Here)
                .ok_or(Error::BadHandle)
        }
        Some(_) => Err(Error::WrongThread),
        None => Err(Error::BadHandle),
    }
}

/// Look the handle up, take its machine's lock without waiting, and run `f`
/// with the routine's panics caught.
///
/// No table lock or borrow is held while the routine runs: a routine may
/// spawn, and spawning inserts into the tables. A panicking routine is
/// removed and reported as [`Error::Panicked`]; for a migrating one, the
/// removal happens before its lock is released, so no other thread can
/// observe the poisoned machine in between.
fn guarded<T>(
    handle: u64,
    f: impl FnOnce(&mut dyn Stepped) -> Result<T, Error>,
) -> Result<T, Error> {
    match find(handle)? {
        Found::Migrating(entry) => {
            let mut guard = entry.try_lock()?;

            if let Ok(result) = catch_unwind(AssertUnwindSafe(|| f(&mut **guard))) {
                return result;
            }

            table().machines.remove(&handle);
            drop(guard);
            Err(Error::Panicked)
        }
        Found::Here(entry) => {
            let mut machine = entry.try_borrow_mut().map_err(|_| Error::Busy)?;

            if let Ok(result) = catch_unwind(AssertUnwindSafe(|| f(&mut **machine))) {
                return result;
            }

            drop(machine);
            table().pinned.remove(&handle);
            LOCAL.with(|local| local.borrow_mut().remove(&handle));
            Err(Error::Panicked)
        }
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert their preconditions"
    )]

    use super::*;
    use crate::fixtures::{Echo, Effect, View, framed, reply_str_record};
    use sans_effort::run::Run;

    #[test]
    fn round_trip_across_threads() {
        let h = new(|outbox| Echo(outbox).run());

        let (bytes, status) = start(h).expect("start");
        assert_eq!(status, Status::Awaiting);
        assert_eq!(bytes, framed(&[View::Ask(1)]));

        let record = reply_str_record(1, "far");
        let elsewhere = std::thread::spawn(move || reply(h, &record))
            .join()
            .expect("thread");
        let (bytes, status) = elsewhere.expect("replied on another thread");
        assert_eq!(status, Status::Complete);
        assert_eq!(bytes, framed(&[View::Say(String::from("far"))]));

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

    /// A queue two routines share: the smallest in-process channel. It stores
    /// no waker; the host resumes idle machines.
    type Queue = Arc<Mutex<std::collections::VecDeque<String>>>;

    fn recv(queue: &Queue) -> impl Future<Output = String> + '_ {
        std::future::poll_fn(|_| {
            queue
                .lock()
                .expect("queue")
                .pop_front()
                .map_or(std::task::Poll::Pending, std::task::Poll::Ready)
        })
    }

    #[test]
    fn two_machines_exchange_a_message_through_resume() {
        let queue = Queue::default();
        let listener = new({
            let queue = Queue::clone(&queue);
            move |outbox: Outbox<Effect>| async move {
                let message = recv(&queue).await;
                outbox.tell(Effect::Say(message));
            }
        });
        let relay = new({
            let queue = Queue::clone(&queue);
            move |outbox: Outbox<Effect>| async move {
                let message = outbox.ask(Effect::Ask).await;
                queue.lock().expect("queue").push_back(message);
            }
        });

        assert_eq!(start(listener), Ok((Vec::new(), Status::Idle)));
        assert_eq!(start(relay).expect("start").1, Status::Awaiting);
        assert_eq!(
            reply(relay, &reply_str_record(1, "hi")),
            Ok((Vec::new(), Status::Complete))
        );
        assert_eq!(
            resume(listener),
            Ok((framed(&[View::Say(String::from("hi"))]), Status::Complete))
        );

        free(listener).expect("free");
        free(relay).expect("free");
    }

    /// Echo, holding an `Rc` across its wait: its future is not `Send`, so
    /// only a pinned machine can run it.
    fn pinned_echo(outbox: Outbox<Effect>) -> LocalBoxedRoutine {
        let local = Rc::new(());
        Box::pin(async move {
            let answer = outbox.ask(Effect::Ask).await;
            drop(local);
            outbox.tell(Effect::Say(answer));
        })
    }

    #[test]
    fn a_pinned_machine_stays_on_the_thread_that_started_it() {
        let h = park_pinned(pinned_echo);
        assert_eq!(
            reply(h, &reply_str_record(1, "early")),
            Err(Error::BadInput),
            "parked: not started yet"
        );

        let (owner_start, record) = (
            thread::spawn(move || start(h)),
            reply_str_record(1, "from afar"),
        );
        let (bytes, status) = owner_start.join().expect("thread").expect("start");
        assert_eq!((bytes, status), (framed(&[View::Ask(1)]), Status::Awaiting));

        assert_eq!(reply(h, &record), Err(Error::WrongThread));
        assert_eq!(resume(h), Err(Error::WrongThread));
        assert_eq!(free(h), Err(Error::WrongThread), "only its owner frees it");
        assert_eq!(start(h), Err(Error::WrongThread));
    }

    #[test]
    fn a_pinned_machine_runs_and_frees_on_its_own_thread() {
        let h = park_pinned(pinned_echo);
        thread::spawn(move || {
            assert_eq!(start(h).expect("start").1, Status::Awaiting);
            assert_eq!(start(h), Err(Error::BadInput), "start twice");
            assert_eq!(
                reply(h, &reply_str_record(1, "near")),
                Ok((framed(&[View::Say(String::from("near"))]), Status::Complete))
            );
            free(h).expect("free");
            assert_eq!(free(h), Err(Error::BadHandle));
        })
        .join()
        .expect("thread");
    }

    #[test]
    fn a_parked_machine_frees_from_any_thread() {
        let h = park_pinned(pinned_echo);
        thread::spawn(move || free(h))
            .join()
            .expect("thread")
            .expect("free");
        assert_eq!(start(h), Err(Error::BadHandle));
    }

    #[test]
    fn a_panicking_pinned_machine_is_removed() {
        let h = park_pinned(|outbox: Outbox<Effect>| -> LocalBoxedRoutine {
            Box::pin(async move {
                drop(outbox);
                panic!("routine bug");
            })
        });

        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = start(h);
        std::panic::set_hook(hook);

        assert_eq!(result, Err(Error::Panicked));
        assert_eq!(start(h), Err(Error::BadHandle), "removed from both tables");
    }

    fn frames_of(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut r = sans_effort::boundary::codec::Reader::new(bytes);
        let mut out = Vec::new();
        while !r.is_empty() {
            let kind = r.u8().expect("kind");
            out.push((kind, r.bytes().expect("payload").to_vec()));
        }
        out
    }

    fn woken_in(bytes: &[u8]) -> Vec<u64> {
        frames_of(bytes)
            .into_iter()
            .filter(|(kind, _)| *kind == FRAME_WOKE)
            .map(|(_, payload)| u64::from_le_bytes(payload.try_into().expect("a u64")))
            .collect()
    }

    #[test]
    fn the_sender_s_output_names_the_machine_it_woke() {
        let (tx, rx) = async_channel::unbounded::<String>();
        let listener = new(move |outbox: Outbox<Effect>| async move {
            if let Ok(message) = rx.recv().await {
                outbox.tell(Effect::Say(message));
            }
        });
        let relay = new(move |outbox: Outbox<Effect>| async move {
            let message = outbox.ask(Effect::Ask).await;
            drop(tx.send(message).await);
        });

        assert_eq!(start(listener), Ok((Vec::new(), Status::Idle)));
        drop(start(relay).expect("start"));
        let (bytes, status) = reply(relay, &reply_str_record(1, "hi")).expect("reply");
        assert_eq!(status, Status::Complete);
        assert_eq!(
            woken_in(&bytes),
            [listener],
            "the relay's own output says who it woke"
        );

        assert_eq!(
            resume(listener),
            Ok((framed(&[View::Say(String::from("hi"))]), Status::Complete))
        );
        free(listener).expect("free");
        free(relay).expect("free");
    }

    #[test]
    fn a_wake_outside_any_call_is_reported_by_wakes() {
        let (tx, rx) = async_channel::unbounded::<String>();
        let listener = new(move |outbox: Outbox<Effect>| async move {
            let closed = rx.recv().await.is_err();
            outbox.tell(Effect::Say(format!("closed: {closed}")));
        });
        // Holds the sender while it waits for a reply that never comes.
        let holder = new(move |outbox: Outbox<Effect>| async move {
            let _tx = tx;
            drop(outbox.ask(Effect::Ask).await);
        });

        assert_eq!(start(listener), Ok((Vec::new(), Status::Idle)));
        drop(start(holder).expect("start"));
        free(holder).expect("free: drops the sender, which wakes the listener");
        assert!(
            woken_in(&wakes()).contains(&listener),
            "no call was running, so the wake waited for `wakes`"
        );

        assert_eq!(
            resume(listener),
            Ok((
                framed(&[View::Say(String::from("closed: true"))]),
                Status::Complete
            ))
        );
        free(listener).expect("free");
    }

    #[test]
    fn a_woke_frame_is_kind_length_handle() {
        assert_eq!(
            woke_frames(&[5]),
            [
                4, // FRAME_WOKE
                8, 0, 0, 0, // payload length
                5, 0, 0, 0, 0, 0, 0, 0, // the handle
            ]
        );
    }
}
