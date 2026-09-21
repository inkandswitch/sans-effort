//! The greeter on tokio, with no driver.
//!
//! [`ctx::TokioCtx`] implements the five traits with tokio primitives, and
//! `Greeter::new(ctx).run()` is then an ordinary future: `tokio::spawn`
//! polls it, `tokio::time::sleep` wakes it, stdin delivers its lines. There
//! is no effect enum, no reply loop, no host — the routine _is_ the task.
//! Compare `../cdylib` and `../wasm`, where the identical `Greeter` runs
//! behind a `Driver` because the poller is not Rust.
//!
//! `tokio::spawn` needs the future to be `Send`. Nothing in the traits says
//! so; the compiler infers it from `TokioCtx`'s fields, because at this call
//! site the context is concrete.
//!
//! ```sh
//! printf 'alice\nbob\nquit\n' | cargo run -p greeter_tokio
//! cargo run -p greeter_tokio -- --fanout    # two waits at a time
//! ```
//!
//! The test at the bottom runs the same routines under tokio's paused clock:
//! the 50 ms `PAUSE`s cost no wall time, and the transcript is checked.

mod ctx;

use ctx::TokioCtx;
use effect_routine::run::Run;
use routines::{fanout::Fanout, greeter::Greeter};
use tokio::io::BufReader;

#[tokio::main]
async fn main() -> Result<(), tokio::task::JoinError> {
    let ctx = TokioCtx::new(BufReader::new(tokio::io::stdin()));

    if std::env::args().any(|a| a == "--fanout") {
        tokio::spawn(Fanout::new(ctx).run()).await
    } else {
        tokio::spawn(Greeter::new(ctx).run()).await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use std::time::Instant;

    /// The greeter runs to completion under tokio alone — no `Driver`
    /// anywhere — and its sleeps are tokio's: under a paused clock, three
    /// 50 ms pauses take no wall time and advance virtual time by 150 ms.
    ///
    /// The traits say nothing about `Send`; the future is `Send` because
    /// `TokioCtx` is, and the compiler works that out here, where the
    /// context is concrete. `assert_send` makes the inference visible.
    #[tokio::test(start_paused = true)]
    async fn greeter_runs_natively_in_virtual_time() {
        fn assert_send<T: Send>(value: T) -> T {
            value
        }

        let started = Instant::now();
        let virtual_start = tokio::time::Instant::now();

        let ctx = TokioCtx::new(BufReader::new(&b"alice\nbob\ncarol\nquit\n"[..]));
        tokio::spawn(assert_send(Greeter::new(ctx).run()))
            .await
            .expect("finished");

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

        let ctx = TokioCtx::new(BufReader::new(&b"bob\ncarol\n"[..]));
        tokio::spawn(Fanout::new(ctx).run())
            .await
            .expect("finished");

        assert_eq!(virtual_start.elapsed().as_millis(), 50);
    }
}
