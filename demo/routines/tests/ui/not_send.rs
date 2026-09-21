//! A routine over an `Rc`-based context is `!Send`, and the compiler says so
//! at the one place that asks — not on the traits.

use core::{cell::Cell, time::Duration};
use effect_routine::run::Run;
use routines::{
    ticker::Ticker,
    traits::{Clock, Output},
};
use std::rc::Rc;

struct Local(Rc<Cell<u32>>);

impl Clock for Local {
    async fn sleep(&self, _: Duration) {}
}

impl Output for Local {
    fn write(&self, _: String) {}
}

fn spawn<F: core::future::Future + Send + 'static>(_: F) {}

fn main() {
    spawn(Ticker::new(Local(Rc::new(Cell::new(0))), 3).run());
}
