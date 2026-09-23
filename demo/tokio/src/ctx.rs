//! The demo's native context: `sans-effort-tokio`'s `TokioCtx`, plus the
//! demo's own capabilities.
//!
//! `TokioCtx` already serves `Sleep`, `ReadLine`, and `WriteLine` with tokio
//! futures. `Count` and `Lookup` are the demo's, and the orphan rule allows
//! `impl Count for TokioCtx` in neither this crate nor any other that owns
//! only one side — so they go on a type this crate owns, which forwards the
//! stdlib capabilities to the `TokioCtx` inside it, one line each.
//!
//! Compare `greeter_boundary`'s vocabularies: there each capability records
//! an effect and suspends until a host replies. Here each one is a real
//! future, and the routine is the task.

use core::time::Duration;
use routines::traits::{Count, Lookup};
use sans_effort_effects::{
    console::{ReadLine, ReadLineError, WriteLine},
    time::Sleep,
};
use sans_effort_tokio::ctx::TokioCtx;
use std::{
    io,
    sync::atomic::{AtomicU64, Ordering},
};
use tokio::io::AsyncBufRead;

/// `TokioCtx` with a greeting counter and a fixed directory.
#[derive(Debug)]
pub(crate) struct DemoCtx<R, W> {
    tokio: TokioCtx<R, W>,
    greeted: AtomicU64,
}

impl<R, W> DemoCtx<R, W> {
    /// The demo's capabilities on top of `tokio`.
    pub(crate) const fn new(tokio: TokioCtx<R, W>) -> Self {
        Self {
            tokio,
            greeted: AtomicU64::new(0),
        }
    }
}

impl<R, W> Count for DemoCtx<R, W> {
    async fn count(&self) -> u64 {
        self.greeted.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl<R, W> Lookup for DemoCtx<R, W> {
    async fn lookup(&self, name: String) -> String {
        match name.as_str() {
            "alice" => "Hello",
            "bob" => "Hi",
            "carol" => "Hey",
            _ => "Greetings",
        }
        .to_owned()
    }
}

impl<R, W> Sleep for DemoCtx<R, W> {
    async fn sleep(&self, duration: Duration) {
        self.tokio.sleep(duration).await;
    }
}

impl<R: AsyncBufRead + Unpin, W> ReadLine for DemoCtx<R, W> {
    async fn read_line(&self) -> Result<String, ReadLineError> {
        self.tokio.read_line().await
    }
}

impl<R, W: io::Write> WriteLine for DemoCtx<R, W> {
    fn write_line(&self, line: String) {
        self.tokio.write_line(line);
    }
}
