//! The handle table: machines behind `u64`s.
//!
//! Most machines _migrate_: their drivers are `Send`, so they live behind a
//! `Mutex` in a `static` and a handle may be driven from any thread, one at a
//! time; two threads colliding on one handle get [`Error::Busy`], not a race.
//! Hosts may pool, and a machine's calls migrate between threads.
//!
//! A _pinned_ machine's future need not be `Send`, so it must stay on one
//! thread. It is parked unstarted by [`park_pinned`]; the thread that first
//! calls [`resume`] for it builds it and keeps it in its own table, and every
//! later call for it must come from that thread. From any other, a call is
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
    contract::{FRAME_WOKE, OK},
    encoded::Encoded,
    error::Error,
    machine::Machine,
    record::{Event, Log, Outcome, outcome},
};
use sans_effort_core::{
    boundary::{
        codec::{Encode, Writer},
        host_effect::HostEffect,
    },
    driver::status::Status,
    driver::{BoxedRoutine, Driver, LocalBoxedRoutine, outbox::Outbox},
};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
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
    fn reply(&mut self, record: &[u8]) -> Result<(Vec<u8>, Status), Error>;
    fn resume(&mut self) -> Result<(Vec<u8>, Status), Error>;
    fn on_wake(&self, hook: Box<dyn Fn() + Send + Sync>);
}

impl<E: HostEffect, F: Future<Output = ()> + ?Sized> Stepped for Encoded<E, F>
where
    E::View: Encode,
{
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
        Box::new(Encoded::new(Machine::new(Driver::local_boxed(self.make))))
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

    /// Machines being recorded, each with its session and its number there.
    recorded: HashMap<u64, Tap>,

    /// Recording sessions, by number: the log each is building.
    sessions: HashMap<u32, Log>,
    next_session: u32,
}

/// Where a recorded machine's calls go: its session, and its number in it
/// (creation order, the root first).
#[derive(Clone, Copy)]
struct Tap {
    session: u32,
    machine: u32,
}

impl Table {
    fn new() -> Self {
        Self {
            machines: HashMap::new(),
            parked: HashMap::new(),
            pinned: HashMap::new(),
            next: 1,
            recorded: HashMap::new(),
            sessions: HashMap::new(),
            next_session: 1,
        }
    }

    /// A fresh handle, never `0`, never reused — unless this thread is
    /// replaying, when it is the next handle the recording issued, if that
    /// is free again.
    fn issue(&mut self) -> u64 {
        if let Some(handle) = REISSUE.with(|reissue| reissue.borrow_mut().pop_front())
            && !self.in_use(handle)
        {
            self.next = self.next.max(handle + 1);
            return handle;
        }
        while self.in_use(self.next) {
            self.next += 1;
        }
        let handle = self.next;
        self.next += 1;
        handle
    }

    fn in_use(&self, handle: u64) -> bool {
        self.machines.contains_key(&handle)
            || self.parked.contains_key(&handle)
            || self.pinned.contains_key(&handle)
    }

    /// If the call this thread is making is on a recorded machine, record
    /// `child` — just created by it — in the same session.
    fn adopt(&mut self, child: u64) {
        let Some(parent) = CALL.with(|call| call.borrow().as_ref().map(|call| call.handle)) else {
            return;
        };
        let Some(Tap { session, .. }) = self.recorded.get(&parent).copied() else {
            return;
        };
        if let Some(log) = self.sessions.get_mut(&session) {
            let machine = log.adopt(child);
            self.recorded.insert(child, Tap { session, machine });
        }
    }

    /// Log `event` for `handle`, if it is being recorded.
    fn note(&mut self, handle: u64, event: impl FnOnce(u32) -> Event) {
        if let Some(Tap { session, machine }) = self.recorded.get(&handle).copied()
            && let Some(log) = self.sessions.get_mut(&session)
        {
            log.push(event(machine));
        }
    }
}

static TABLE: LazyLock<Mutex<Table>> = LazyLock::new(|| Mutex::new(Table::new()));

thread_local! {
    /// This thread's started pinned machines.
    static LOCAL: RefCell<HashMap<u64, LocalEntry>> = RefCell::new(HashMap::new());

    /// The call this thread is making, if it is making one: which machine,
    /// and the machines woken during it, which are reported in its output.
    static CALL: RefCell<Option<Call>> = const { RefCell::new(None) };

    /// While this thread replays a recording: the handles it issued, in
    /// order, for the table to issue again.
    static REISSUE: RefCell<VecDeque<u64>> = const { RefCell::new(VecDeque::new()) };
}

/// A call in progress on this thread.
struct Call {
    handle: u64,
    woken: Vec<u64>,
}

/// Machines woken outside any call, until [`wakes`] reports them.
static WAKES: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// The hook every machine's driver calls when it wakes: note `handle` for the
/// current call, or for [`wakes`] if there is none.
fn woke(handle: u64) {
    let noted = CALL.with(|call| {
        call.borrow_mut()
            .as_mut()
            .map(|call| call.woken.push(handle))
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
    handle: u64,
    call: impl FnOnce() -> Result<(Vec<u8>, Status), Error>,
) -> Result<(Vec<u8>, Status), Error> {
    let this = Call {
        handle,
        woken: Vec::new(),
    };
    let outer = CALL.with(|current| current.replace(Some(this)));
    let result = call();
    let woken = CALL
        .with(|current| current.replace(outer))
        .map(|call| call.woken)
        .unwrap_or_default();

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
/// first calls [`resume`] for the returned handle, and stays on that thread. Until
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
    table.adopt(handle);
    handle
}

fn insert(machine: Box<dyn Stepped + Send>) -> u64 {
    let mut table = table();
    let handle = table.issue();
    machine.on_wake(Box::new(move || woke(handle)));
    table.machines.insert(handle, Entry::new(machine));
    table.adopt(handle);
    handle
}

/// Deliver one reply record and run the routine to its next wait: the
/// effects it recorded, encoded, plus its status.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`],
/// [`Error::WrongThread`], [`Error::BadInput`] for a parked routine not yet
/// resumed (it has issued no ids), or whatever [`Encoded::reply`] returns.
pub fn reply(handle: u64, record: &[u8]) -> Result<(Vec<u8>, Status), Error> {
    let result = reporting_wakes(handle, || guarded(handle, |m| m.reply(record)));
    table().note(handle, |machine| Event::Reply {
        machine,
        record: record.to_vec(),
        outcome: outcome(&result),
    });
    result
}

/// Run the routine to its next wait without delivering anything: the effects
/// it recorded, encoded, plus its status. The first call begins it — and, for
/// a parked pinned routine, builds it on the calling thread, which then owns
/// it. After that, for an [`Status::Idle`] routine — one a [`FRAME_WOKE`]
/// frame named — once something it waits on may have changed; harmless when
/// nothing has.
///
/// # Errors
///
/// [`Error::BadHandle`], [`Error::Busy`], [`Error::Panicked`],
/// [`Error::WrongThread`], or whatever [`Encoded::resume`] returns.
pub fn resume(handle: u64) -> Result<(Vec<u8>, Status), Error> {
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

    if let Some(parked) = parked {
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
    }

    let result = reporting_wakes(handle, || guarded(handle, |m| m.resume()));
    table().note(handle, |machine| Event::Resume {
        machine,
        outcome: outcome(&result),
    });
    result
}

/// Drop a routine, including any request it had outstanding. A parked pinned
/// routine may be freed from any thread; a started one only from its own.
///
/// # Errors
///
/// [`Error::BadHandle`] if there is no such routine; [`Error::WrongThread`]
/// for a pinned routine started on another thread.
pub fn free(handle: u64) -> Result<(), Error> {
    let result = free_unrecorded(handle);
    let code = result.map_or_else(Error::code, |()| OK);
    table().note(handle, |machine| Event::Free {
        machine,
        outcome: Outcome {
            code,
            bytes: Vec::new(),
        },
    });
    result
}

fn free_unrecorded(handle: u64) -> Result<(), Error> {
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

/// Begin recording `root` and every machine it creates: the session's
/// number.
pub(crate) fn start_recording(root: u64) -> u32 {
    let mut table = table();
    let session = table.next_session;
    table.next_session += 1;
    let mut log = Log::default();
    let machine = log.adopt(root);
    table.sessions.insert(session, log);
    table.recorded.insert(root, Tap { session, machine });
    session
}

/// Stop recording a session: its log.
pub(crate) fn finish_recording(session: u32) -> Log {
    let mut table = table();
    table.recorded.retain(|_, tap| tap.session != session);
    table.sessions.remove(&session).unwrap_or_default()
}

/// The handle a session gave its `machine`th machine, if it has one yet.
pub(crate) fn recorded_handle(session: u32, machine: u32) -> Option<u64> {
    table()
        .sessions
        .get(&session)
        .and_then(|log| log.handle(machine))
}

/// Have this thread issue `handles` again, in order, until the returned
/// guard is dropped.
pub(crate) fn reissue(handles: &[u64]) -> Reissue {
    REISSUE.with(|reissue| *reissue.borrow_mut() = handles.iter().copied().collect());
    Reissue(PhantomData)
}

/// Stops re-issuing handles when dropped. Not `Send`: it is this thread's.
pub(crate) struct Reissue(PhantomData<*const ()>);

impl Drop for Reissue {
    fn drop(&mut self) {
        REISSUE.with(|reissue| reissue.borrow_mut().clear());
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
    use sans_effort_core::step::Step;

    #[test]
    fn round_trip_across_threads() {
        let h = new(|outbox| Echo(outbox).run());

        let (bytes, status) = resume(h).expect("resume");
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
        assert_eq!(resume(h), Err(Error::BadHandle));
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
        let result = resume(h);
        std::panic::set_hook(hook);

        assert_eq!(result, Err(Error::Panicked));
        assert_eq!(resume(h), Err(Error::BadHandle), "removed from the table");
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

        assert_eq!(resume(listener), Ok((Vec::new(), Status::Idle)));
        assert_eq!(resume(relay).expect("resume").1, Status::Awaiting);
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
            "parked: not resumed yet, so no id has been issued"
        );

        let (owner_start, record) = (
            thread::spawn(move || resume(h)),
            reply_str_record(1, "from afar"),
        );
        let (bytes, status) = owner_start.join().expect("thread").expect("resume");
        assert_eq!((bytes, status), (framed(&[View::Ask(1)]), Status::Awaiting));

        assert_eq!(reply(h, &record), Err(Error::WrongThread));
        assert_eq!(resume(h), Err(Error::WrongThread));
        assert_eq!(free(h), Err(Error::WrongThread), "only its owner frees it");
        assert_eq!(resume(h), Err(Error::WrongThread));
    }

    #[test]
    fn a_pinned_machine_runs_and_frees_on_its_own_thread() {
        let h = park_pinned(pinned_echo);
        thread::spawn(move || {
            assert_eq!(resume(h).expect("resume").1, Status::Awaiting);
            assert_eq!(
                resume(h),
                Ok((Vec::new(), Status::Awaiting)),
                "again before the reply: nothing new"
            );
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
        assert_eq!(resume(h), Err(Error::BadHandle));
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
        let result = resume(h);
        std::panic::set_hook(hook);

        assert_eq!(result, Err(Error::Panicked));
        assert_eq!(resume(h), Err(Error::BadHandle), "removed from both tables");
    }

    fn frames_of(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut r = sans_effort_core::boundary::codec::Reader::new(bytes);
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

        assert_eq!(resume(listener), Ok((Vec::new(), Status::Idle)));
        drop(resume(relay).expect("resume"));
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

        assert_eq!(resume(listener), Ok((Vec::new(), Status::Idle)));
        drop(resume(holder).expect("resume"));
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
