//! `Send` is inferred at the spawn site, not declared on the traits.
//!
//! Both halves of that claim, pinned. A context built on `Rc` satisfies the
//! traits and runs a routine with `run_now`; the same routine cannot be
//! handed to anything that requires `Send`, because the compiler sees the
//! `Rc` in the concrete future (`ui/not_send.rs`, checked with `trybuild`).

use core::{cell::Cell, time::Duration};
use routines::{
    ticker::Ticker,
    traits::{Sleep, WriteLine},
};
use sans_effort::{run::Run, testing::run_now};
use std::rc::Rc;

/// A context nothing could ever send across a thread.
struct Local(Rc<Cell<u32>>);

impl Sleep for Local {
    async fn sleep(&self, _: Duration) {}
}

impl WriteLine for Local {
    fn write_line(&self, _: String) {
        self.0.set(self.0.get() + 1);
    }
}

#[test]
fn a_not_send_context_runs_where_nothing_asks_for_send() {
    let written = Rc::new(Cell::new(0));
    run_now(Ticker::new(Local(written.clone()), 3).run());
    assert_eq!(written.get(), 3);
}

#[test]
fn a_not_send_context_is_rejected_where_send_is_required() {
    trybuild::TestCases::new().compile_fail("tests/ui/not_send.rs");
}
