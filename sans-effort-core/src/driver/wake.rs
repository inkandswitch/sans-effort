//! The driver's waker: how a machine learns it can make progress without a
//! reply.
//!
//! A future that cannot finish stores the waker it was polled with; whatever
//! it waits on calls it when that may have changed — a channel on a send, a
//! lock on a release. Under tokio the waker requeues the task. Here it sets a
//! flag and calls a hook the host layer installs, which tells the host "this
//! machine can run". The flag makes that once per wait: further wakes before
//! the next poll are already known, and the hook is not called again.
//!
//! Built once per driver and reused for every poll, so stepping still
//! allocates nothing.

use super::sync::{Arc, AtomicBool, Mutex, Wake};
use alloc::boxed::Box;
use core::{sync::atomic::Ordering, task::Waker};

/// What a driver's waker calls: the host layer's "this machine woke".
pub(super) type Hook = Box<dyn Fn() + Send + Sync>;

/// A driver's wake state: whether it has been woken since its last poll
/// began, and whom to tell.
pub(super) struct Wakeup {
    woken: AtomicBool,
    hook: Mutex<Option<Hook>>,
}

impl Wakeup {
    /// The state, and a waker over it.
    pub(super) fn new() -> (Arc<Self>, Waker) {
        let wakeup = Arc::new(Self {
            woken: AtomicBool::new(false),
            hook: Mutex::new(None),
        });
        let waker = Waker::from(Arc::clone(&wakeup));
        (wakeup, waker)
    }

    /// Who to tell. Replaces any earlier hook.
    pub(super) fn set_hook(&self, hook: Hook) {
        *self.hook.lock() = Some(hook);
    }

    /// A poll is about to begin: anything that wakes the routine from now on
    /// is news again.
    pub(super) fn clear(&self) {
        self.woken.store(false, Ordering::Release);
    }

    /// Record the wake; tell the hook if this is the first since the last
    /// poll began.
    fn wake_up(&self) {
        if !self.woken.swap(true, Ordering::AcqRel)
            && let Some(hook) = self.hook.lock().as_ref()
        {
            hook();
        }
    }
}

#[cfg(not(feature = "portable-atomic"))]
impl Wake for Wakeup {
    fn wake(self: Arc<Self>) {
        self.wake_up();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wake_up();
    }
}

/// `portable-atomic-util`'s `Wake` names the receiver `this`, not `self`.
#[cfg(feature = "portable-atomic")]
impl Wake for Wakeup {
    fn wake(this: Arc<Self>) {
        this.wake_up();
    }

    fn wake_by_ref(this: &Arc<Self>) {
        this.wake_up();
    }
}
