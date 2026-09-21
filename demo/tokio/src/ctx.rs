//! The five capability traits, served by tokio.
//!
//! Compare `greeter_wire::Ctx`: there each trait method records a request
//! and suspends until a host replies. Here each one is a real future —
//! `tokio::time::sleep`, a line from an async reader — and tokio's own
//! waker drives it. No effect is built, no driver polls, no host loop
//! interprets anything. The routine is the task.

use routines::traits::{Count, Lookup, ReadLine, Sleep, WriteLine};
use std::{
    io::{self, Write as _},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Duration,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt},
    sync::Mutex,
};

/// A context whose waits are tokio futures.
///
/// Generic over its line source `R`: stdin in `main`, a byte slice in tests,
/// statically dispatched either way. `WriteLine` writes to stdout; `Lookup` is a fixed table; `Count`
/// is an atomic; `Sleep` is `tokio::time::sleep`, so under a paused-clock
/// test it costs no wall time.
pub(crate) struct TokioCtx<R> {
    /// `read_line` needs `&mut R`; the trait takes `&self` so that a routine
    /// can `join` two waits on one context. Something must bridge the two,
    /// and it is the context's job, not the caller's. It must be tokio's mutex,
    /// not `std`'s: the guard is held across `.await`, and a `std` guard there
    /// would make the future `!Send` and could deadlock a single thread.
    /// Never contended — one routine holds one context.
    lines: Mutex<R>,
    greeted: AtomicU64,
    /// Set once stdout has failed, so the failure is reported once and the
    /// routine is left to finish on its own (it reads EOF and quits).
    stdout_failed: AtomicBool,
}

impl<R: AsyncBufRead + Send + Unpin> TokioCtx<R> {
    /// A context reading lines from `input`.
    pub(crate) const fn new(input: R) -> Self {
        Self {
            lines: Mutex::const_new(input),
            greeted: AtomicU64::new(0),
            stdout_failed: AtomicBool::new(false),
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

impl<R: AsyncBufRead + Send + Unpin> Sleep for TokioCtx<R> {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

impl<R: AsyncBufRead + Send + Unpin> Count for TokioCtx<R> {
    async fn count(&self) -> u64 {
        self.greeted.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl<R: AsyncBufRead + Send + Unpin> Lookup for TokioCtx<R> {
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

impl<R: AsyncBufRead + Send + Unpin> ReadLine for TokioCtx<R> {
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

impl<R: AsyncBufRead + Send + Unpin> WriteLine for TokioCtx<R> {
    /// `WriteLine::write` is fire-and-forget by design, so a stdout error has
    /// nowhere to go. A context must not end the process on the routine's
    /// behalf; it reports once and carries on.
    fn write(&self, line: String) {
        if self.stdout_failed.load(Ordering::Relaxed) {
            return;
        }

        if let Err(e) = writeln!(io::stdout().lock(), "{line}") {
            self.stdout_failed.store(true, Ordering::Relaxed);
            eprintln!("greeter_tokio: stdout: {e}");
        }
    }
}
