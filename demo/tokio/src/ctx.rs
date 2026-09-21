//! The five capability traits, served by tokio.
//!
//! Compare `greeter_wire::Ctx`: there each trait method records a request
//! and suspends until a host replies. Here each one is a real future —
//! `tokio::time::sleep`, a line from an async reader — and tokio's own
//! waker drives it. No effect is built, no driver polls, no host loop
//! interprets anything. The routine is the task.

use routines::traits::{Clock, Counter, Directory, Input, Output};
use std::{
    io::{self, Write as _},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt},
    sync::Mutex,
};

/// A context whose waits are tokio futures.
///
/// Generic over its line source `R`: stdin in `main`, a byte slice in tests,
/// statically dispatched either way. `Output` writes to stdout; `Directory` is a fixed table; `Counter`
/// is an atomic; `Clock` is `tokio::time::sleep`, so under a paused-clock
/// test it costs no wall time.
pub(crate) struct TokioCtx<R> {
    /// `read_line` needs `&mut`; the trait takes `&self`. An async mutex,
    /// never contended: one routine holds one context.
    lines: Mutex<R>,
    greeted: AtomicU64,
}

impl<R: AsyncBufRead + Send + Unpin> TokioCtx<R> {
    /// A context reading lines from `input`.
    pub(crate) const fn new(input: R) -> Self {
        Self {
            lines: Mutex::const_new(input),
            greeted: AtomicU64::new(0),
        }
    }
}

impl<R> std::fmt::Debug for TokioCtx<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokioCtx")
            .field("greeted", &self.greeted)
            .finish_non_exhaustive()
    }
}

impl<R: AsyncBufRead + Send + Unpin> Clock for TokioCtx<R> {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

impl<R: AsyncBufRead + Send + Unpin> Counter for TokioCtx<R> {
    async fn count(&self) -> u64 {
        self.greeted.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl<R: AsyncBufRead + Send + Unpin> Directory for TokioCtx<R> {
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

impl<R: AsyncBufRead + Send + Unpin> Input for TokioCtx<R> {
    /// The next line, or `quit` at end of input.
    async fn read_line(&self) -> String {
        let mut line = String::new();
        let read = self.lines.lock().await.read_line(&mut line).await;

        match read {
            Ok(0) | Err(_) => String::from("quit"),
            Ok(_) => line.trim_end().to_owned(),
        }
    }
}

impl<R: AsyncBufRead + Send + Unpin> Output for TokioCtx<R> {
    fn write(&self, line: String) {
        // A closed stdout is the host's problem, not the routine's.
        if writeln!(io::stdout().lock(), "{line}").is_err() {
            std::process::exit(0);
        }
    }
}
