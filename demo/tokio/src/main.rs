//! The greeter on tokio, with no driver.
//!
//! [`ctx::DemoCtx`] is `sans-effort-tokio`'s `TokioCtx` plus the demo's own
//! `Count` and `Lookup`, and `Greeter::new(ctx).run()` is then an ordinary
//! future: `tokio::spawn` polls it, `tokio::time::sleep` wakes it, stdin
//! delivers its lines. There is no effect enum, no reply loop, no host — the
//! routine _is_ the task.
//! Compare `../cdylib` and `../wasm`, where the identical `Greeter` runs
//! behind a `Driver` because the poller is not Rust.
//!
//! `tokio::spawn` needs the future to be `Send`. The capability traits
//! declare their futures `Send`, and the context is `Send`, so it is.
//!
//! ```sh
//! printf 'alice\nbob\nquit\n' | cargo run -p greeter_tokio
//! cargo run -p greeter_tokio -- --fanout    # two waits at a time
//! ```
//!
//! The tests at the bottom run the same routines under tokio's paused clock,
//! writing into a buffer: the 50 ms `PAUSE`s cost no wall time, and the
//! transcript is checked.

mod ctx;

use ctx::DemoCtx;
use routines::{fanout::Fanout, greeter::Greeter};
use sans_effort::run::Run;
use sans_effort_tokio::ctx::TokioCtx;
use tokio_util::task::LocalPoolHandle;

#[tokio::main]
async fn main() -> Result<(), tokio::task::JoinError> {
    // Workers for pinned children. Neither demo routine spawns one; the pool
    // is the application's to size, and one thread is plenty here.
    let ctx = DemoCtx::new(TokioCtx::stdio(LocalPoolHandle::new(1)));

    if std::env::args().any(|a| a == "--fanout") {
        tokio::spawn(Fanout::new(ctx).run()).await
    } else {
        tokio::spawn(Greeter::new(ctx).run()).await
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::expect_used, reason = "tests assert their preconditions")]

    use super::*;
    use std::{
        io,
        sync::{Arc, Mutex},
        time::Instant,
    };

    /// A writer the test keeps a handle to while the routine owns the context.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Shared {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("unpoisoned").extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Runs `routine` on a context over `input`, and returns what it wrote.
    async fn transcript<F, P>(input: &'static [u8], routine: F) -> String
    where
        F: FnOnce(DemoCtx<&'static [u8], Shared>) -> P,
        P: core::future::Future<Output = ()> + Send + 'static,
    {
        let out = Shared::default();
        let pool = LocalPoolHandle::new(1);
        tokio::spawn(routine(DemoCtx::new(TokioCtx::new(
            input,
            out.clone(),
            pool,
        ))))
        .await
        .expect("finished");
        let written = out.0.lock().expect("unpoisoned").clone();
        String::from_utf8(written).expect("UTF-8")
    }

    /// The greeter runs to completion under tokio alone — no `Driver`
    /// anywhere — and its sleeps are tokio's: under a paused clock, three
    /// 50 ms pauses take no wall time and advance virtual time by 150 ms.
    ///
    /// `tokio::spawn` accepts the future: the capability traits declare
    /// their futures `Send`, and the context is `Send`.
    #[tokio::test(start_paused = true)]
    async fn greeter_runs_natively_in_virtual_time() {
        let started = Instant::now();
        let virtual_start = tokio::time::Instant::now();

        let written = transcript(b"alice\nbob\ncarol\n", |ctx| Greeter::new(ctx).run()).await;

        assert_eq!(
            written,
            "Who are you?\nHello, alice!\n(greeted 1 so far)\n\
             Who are you?\nHi, bob!\n(greeted 2 so far)\n\
             Who are you?\nHey, carol!\n(greeted 3 so far)\n\
             Who are you?\nBye.\n",
            "the end of input ends the conversation"
        );
        assert!(
            started.elapsed().as_millis() < 100,
            "wall clock advanced: {:?}",
            started.elapsed()
        );
        assert_eq!(
            virtual_start.elapsed().as_millis(),
            150,
            "three PAUSEs of 50 ms each"
        );
    }

    /// Fan-out's `join` is two native tokio futures polled together: the
    /// sleep and the read overlap, so one 50 ms pause covers both.
    #[tokio::test(start_paused = true)]
    async fn fanout_joins_native_futures() {
        let virtual_start = tokio::time::Instant::now();
        let written = transcript(b"bob\ncarol\n", |ctx| Fanout::new(ctx).run()).await;

        assert_eq!(written, "Who are you?\nHi, bob! (#1)\nBye, carol.\n");
        assert_eq!(virtual_start.elapsed().as_millis(), 50);
    }

    /// Ping-pong's child is spawned with `spawn`, so under tokio it is an
    /// ordinary task that may run on any worker; the transcript alternates
    /// because each side waits for the other.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ping_pong_spawns_a_task() {
        let written = transcript(b"", |ctx| routines::ping_pong::PingPong::new(ctx, 3).run()).await;
        assert_eq!(
            written,
            "pong 1\nping 1, pong 1\npong 2\nping 2, pong 2\npong 3\nping 3, pong 3\n"
        );
    }
}
