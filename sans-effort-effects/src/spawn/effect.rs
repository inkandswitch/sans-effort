//! What [`Spawn`](super::Spawn) records under a reifying context.
//!
//! Each effect carries the child itself, unstarted: a closure from the
//! child's own outbox to its boxed future. The host's vocabulary decides what
//! spawning means — typically registering the child as a machine of its own
//! and telling the host its handle. None of it crosses an FFI boundary; only
//! that handle does.

use alloc::boxed::Box;
use core::fmt;
use sans_effort_core::driver::{BoxedRoutine, LocalBoxedRoutine, outbox::Outbox};

/// Start a child that may move between threads: a tell.
#[derive(Debug)]
pub struct Spawn<E>(pub Child<E>);

/// Start a child that stays on the thread that starts it: a tell.
#[derive(Debug)]
pub struct SpawnPinned<E>(pub PinnedChild<E>);

/// A child routine, unstarted; its future is `Send`.
///
/// Only [`Spawn`](super::Spawn) builds one, so a child always gets a context
/// built around its own outbox.
pub struct Child<E> {
    make: Box<dyn FnOnce(Outbox<E>) -> BoxedRoutine + Send>,
}

impl<E> Child<E> {
    pub(super) fn new<M: FnOnce(Outbox<E>) -> BoxedRoutine + Send + 'static>(make: M) -> Self {
        Self {
            make: Box::new(make),
        }
    }

    /// Build the child's future around `outbox`, the outbox of the machine
    /// that will run it — `Driver::from_boxed(|outbox| child.start(outbox))`.
    #[must_use]
    pub fn start(self, outbox: Outbox<E>) -> BoxedRoutine {
        (self.make)(outbox)
    }
}

impl<E> fmt::Debug for Child<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Child").finish_non_exhaustive()
    }
}

/// A child routine, unstarted; its future need not be `Send`, so it is built
/// and run on one thread. The closure that builds it is `Send`, so the child
/// can be handed to that thread first.
pub struct PinnedChild<E> {
    make: Box<dyn FnOnce(Outbox<E>) -> LocalBoxedRoutine + Send>,
}

impl<E> PinnedChild<E> {
    pub(super) fn new<M: FnOnce(Outbox<E>) -> LocalBoxedRoutine + Send + 'static>(make: M) -> Self {
        Self {
            make: Box::new(make),
        }
    }

    /// Build the child's future around `outbox`, on the thread that will run
    /// it — `LocalDriver::from_boxed(|outbox| child.start(outbox))`.
    #[must_use]
    pub fn start(self, outbox: Outbox<E>) -> LocalBoxedRoutine {
        (self.make)(outbox)
    }
}

impl<E> fmt::Debug for PinnedChild<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinnedChild").finish_non_exhaustive()
    }
}
