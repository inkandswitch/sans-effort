//! Spawning, on tokio's runtime.

use core::{fmt, future::Future};
use tokio_util::task::LocalPoolHandle;

/// Where a context's children run: `tokio::spawn` for a child that may move
/// between threads, and a pool of single-threaded workers
/// ([`LocalPoolHandle`]) for a child that must stay on one.
///
/// The pool is the application's: `LocalPoolHandle::new(n)` starts `n`
/// threads at once, so this crate never builds one on its own. Build one at
/// startup and hand clones of it to every context; clones share its workers.
#[derive(Clone)]
pub struct TokioSpawner {
    pinned: LocalPoolHandle,
}

impl TokioSpawner {
    /// A spawner whose pinned children run on `pool`.
    #[must_use]
    pub const fn new(pool: LocalPoolHandle) -> Self {
        Self { pinned: pool }
    }

    /// Run `future` as a tokio task, detached. Call from within a tokio
    /// runtime.
    pub fn spawn<Fut: Future<Output = ()> + Send + 'static>(&self, future: Fut) {
        drop(tokio::spawn(future));
    }

    /// Build a future with `make` on one of the pool's workers and run it
    /// there, detached. Only `make` crosses threads; the future it returns
    /// never does.
    pub fn spawn_pinned<F, Fut>(&self, make: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        drop(self.pinned.spawn_pinned(make));
    }
}

impl fmt::Debug for TokioSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokioSpawner")
            .field("pool_threads", &self.pinned.num_threads())
            .finish()
    }
}
