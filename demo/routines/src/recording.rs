//! A mock of all five capabilities, for the routines' tests.
//!
//! The capabilities test story: a mock implements the traits, every future
//! is ready at once, and one poll runs the routine to completion. No driver,
//! no runtime, no vocabulary — and what the mock recorded is the assertion.
//!
//! The capability traits declare their futures `Send`, and each future
//! borrows the context, so the mock is `Sync`: an `Arc`, locks, and an
//! atomic, though every test runs it on one thread.

use crate::traits::{Count, Lookup};
use alloc::{collections::VecDeque, format, string::String, sync::Arc, vec::Vec};
use core::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use sans_effort::{
    console::{ReadLine, ReadLineError, WriteLine},
    time::Sleep,
};
use sans_effort::{step::Step, testing::run_now};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// What a routine did to its context, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Call {
    Count,
    Lookup(String),
    ReadLine,
    Sleep(Duration),
    Write(String),
}

/// The mock directory's answer for any name.
pub(crate) fn greeting_for(name: &str) -> String {
    format!("Hello to {name}")
}

/// Scripted input, a fixed directory, an instant clock, a running counter,
/// and a log of every call. `Clone` shares the log, so a test keeps a handle
/// after moving one into the routine.
#[derive(Clone)]
pub(crate) struct Recording(Arc<Inner>);

struct Inner {
    calls: Mutex<Vec<Call>>,
    count: AtomicU64,
    lines: Mutex<VecDeque<String>>,
}

impl Recording {
    pub(crate) fn new(script: &[String]) -> Self {
        Self(Arc::new(Inner {
            calls: Mutex::new(Vec::new()),
            count: AtomicU64::new(0),
            lines: Mutex::new(script.iter().cloned().collect()),
        }))
    }

    fn log(&self, call: Call) {
        lock(&self.0.calls).push(call);
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        lock(&self.0.calls).clone()
    }
}

/// A poisoned lock means a test already panicked; the data is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Sleep for Recording {
    async fn sleep(&self, duration: Duration) {
        self.log(Call::Sleep(duration));
    }
}

impl Count for Recording {
    async fn count(&self) -> u64 {
        self.log(Call::Count);
        self.0.count.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl Lookup for Recording {
    async fn lookup(&self, name: String) -> String {
        self.log(Call::Lookup(name.clone()));
        greeting_for(&name)
    }
}

impl ReadLine for Recording {
    /// The next scripted line, or `Closed` once the script runs out.
    async fn read_line(&self) -> Result<String, ReadLineError> {
        self.log(Call::ReadLine);
        lock(&self.0.lines).pop_front().ok_or(ReadLineError::Closed)
    }
}

impl WriteLine for Recording {
    fn write_line(&self, line: String) {
        self.log(Call::Write(line));
    }
}

/// Run a routine against a recording of the script; return what it did.
pub(crate) fn transcript<P: Step>(
    routine: impl FnOnce(Recording) -> P,
    script: &[String],
) -> Vec<Call> {
    let recording = Recording::new(script);
    run_now(routine(recording.clone()).run());
    recording.calls()
}
