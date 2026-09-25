//! An `Rc`-based context cannot implement a capability: the future `sleep`
//! returns borrows it, and a capability's future must be `Send`.

use core::{cell::Cell, time::Duration};
use sans_effort::time::Sleep;
use std::rc::Rc;

struct Local(Rc<Cell<u32>>);

impl Sleep for Local {
    async fn sleep(&self, _: Duration) {
        self.0.set(self.0.get() + 1);
    }
}

fn main() {}
