//! The waker under `portable-atomic`, whose `Wake` names its receiver
//! `this`, not `self`. In a file of its own so mutation testing can leave it
//! out: the tests run without the feature, so they never build it.

use super::Wakeup;
use crate::driver::sync::{Arc, Wake};

impl Wake for Wakeup {
    fn wake(this: Arc<Self>) {
        this.wake_up();
    }

    fn wake_by_ref(this: &Arc<Self>) {
        this.wake_up();
    }
}
