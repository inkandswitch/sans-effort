//! A mock of all five capabilities, for the routines' tests.
//!
//! The capabilities test story: a mock implements the traits, every future
//! is ready at once, and one poll runs the routine to completion. No driver,
//! no runtime, no vocabulary — and what the mock recorded is the assertion.
//!
//! `Recording` is `Rc`-based and so `!Send`, and this is fine: nothing here
//! spawns, so nothing asks. Had the traits demanded `Send`, this mock could
//! not exist.

use crate::traits::{Count, Lookup, ReadLine, Sleep, WriteLine};
use alloc::{
    collections::VecDeque,
    format,
    rc::Rc,
    string::{String, ToString},
    vec::Vec,
};
use core::{
    cell::{Cell, RefCell},
    time::Duration,
};
use effect_routine::{run::Run, testing::run_now};

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
pub(crate) struct Recording(Rc<Inner>);

struct Inner {
    calls: RefCell<Vec<Call>>,
    count: Cell<u64>,
    lines: RefCell<VecDeque<String>>,
}

impl Recording {
    pub(crate) fn new(script: &[String]) -> Self {
        Self(Rc::new(Inner {
            calls: RefCell::new(Vec::new()),
            count: Cell::new(0),
            lines: RefCell::new(script.iter().cloned().collect()),
        }))
    }

    fn log(&self, call: Call) {
        self.0.calls.borrow_mut().push(call);
    }

    pub(crate) fn calls(&self) -> Vec<Call> {
        self.0.calls.borrow().clone()
    }
}

impl Sleep for Recording {
    async fn sleep(&self, duration: Duration) {
        self.log(Call::Sleep(duration));
    }
}

impl Count for Recording {
    async fn count(&self) -> u64 {
        self.log(Call::Count);
        self.0.count.set(self.0.count.get() + 1);
        self.0.count.get()
    }
}

impl Lookup for Recording {
    async fn lookup(&self, name: String) -> String {
        self.log(Call::Lookup(name.clone()));
        greeting_for(&name)
    }
}

impl ReadLine for Recording {
    async fn read_line(&self) -> String {
        self.log(Call::ReadLine);
        self.0
            .lines
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| "quit".to_string())
    }
}

impl WriteLine for Recording {
    fn write(&self, line: String) {
        self.log(Call::Write(line));
    }
}

/// Run a routine against a recording of the script; return what it did.
pub(crate) fn transcript<P: Run>(
    routine: impl FnOnce(Recording) -> P,
    script: &[String],
) -> Vec<Call> {
    let recording = Recording::new(script);
    run_now(routine(recording.clone()).run());
    recording.calls()
}
