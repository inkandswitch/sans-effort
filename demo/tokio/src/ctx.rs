//! The demo's native context: `sans-effort-tokio`'s `TokioCtx`, plus the
//! demo's own capabilities.
//!
//! `TokioCtx` already serves `Sleep`, `ReadLine`, `WriteLine`, and `Spawn`
//! with tokio futures and tasks. `Count` and `Lookup` are the demo's, and the
//! orphan rule allows `impl Count for TokioCtx` in neither this crate nor any
//! other that owns only one side — so they go on a type this crate owns,
//! which forwards the stdlib capabilities to the `TokioCtx` inside it, one
//! line each.
//!
//! Compare `greeter_boundary`'s vocabularies: there each capability records
//! an effect and suspends until a host replies. Here each one is a real
//! future, and the routine is the task.

use core::{future::Future, time::Duration};
use routines::traits::{Count, Lookup};
use sans_effort_effects::{
    console::{ReadLine, ReadLineError, WriteLine},
    spawn::Spawn,
    time::Sleep,
};
use sans_effort_tokio::ctx::TokioCtx;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::io::AsyncBufRead;

/// `TokioCtx` with a greeting counter and a fixed directory. Cloning shares
/// both, as a spawned child's context does.
#[derive(Debug)]
pub(crate) struct DemoCtx<R, W> {
    tokio: TokioCtx<R, W>,
    greeted: Arc<AtomicU64>,
}

impl<R, W> DemoCtx<R, W> {
    /// The demo's capabilities on top of `tokio`.
    pub(crate) fn new(tokio: TokioCtx<R, W>) -> Self {
        Self {
            tokio,
            greeted: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl<R, W> Clone for DemoCtx<R, W> {
    fn clone(&self) -> Self {
        Self {
            tokio: self.tokio.clone(),
            greeted: Arc::clone(&self.greeted),
        }
    }
}

impl<R, W> Count for DemoCtx<R, W> {
    fn count(&self) -> impl Future<Output = u64> + Send {
        let greeted = Arc::clone(&self.greeted);
        async move { greeted.fetch_add(1, Ordering::Relaxed) + 1 }
    }
}

impl<R, W> Lookup for DemoCtx<R, W> {
    /// A fixed directory, so the answer is ready at once.
    fn lookup(&self, name: String) -> impl Future<Output = String> + Send {
        let greeting = match name.as_str() {
            "alice" => "Hello",
            "bob" => "Hi",
            "carol" => "Hey",
            _ => "Greetings",
        };
        core::future::ready(greeting.to_owned())
    }
}

impl<R, W> Sleep for DemoCtx<R, W> {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        self.tokio.sleep(duration)
    }
}

impl<R: AsyncBufRead + Unpin + Send, W> ReadLine for DemoCtx<R, W> {
    fn read_line(&self) -> impl Future<Output = Result<String, ReadLineError>> + Send {
        self.tokio.read_line()
    }
}

impl<R, W: io::Write> WriteLine for DemoCtx<R, W> {
    fn write_line(&self, line: String) {
        self.tokio.write_line(line);
    }
}

/// A child's context is a clone: it shares the counter, the directory, and
/// the console.
impl<R: Send + 'static, W: Send + 'static> Spawn for DemoCtx<R, W> {
    type Child = Self;

    fn spawn<F, Fut>(&self, f: F)
    where
        F: FnOnce(Self) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.tokio.spawner().spawn(f(self.clone()));
    }

    fn spawn_pinned<F, Fut>(&self, f: F)
    where
        F: FnOnce(Self) -> Fut + Send + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        let child = self.clone();
        self.tokio.spawner().spawn_pinned(move || f(child));
    }
}
