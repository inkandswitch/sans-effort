//! Run a routine against a context whose every future is ready at once.
//!
//! The cheapest test of a routine needs no driver and no runtime: a test
//! double implements the routine's traits, each call returns a ready future,
//! and one poll runs the whole routine to completion. The transcript of
//! calls the double recorded is the assertion.
//!
//! ```
//! use core::{cell::RefCell, ops::ControlFlow};
//! use sans_effort::{run::Run, testing::run_now};
//!
//! trait Console {
//!     async fn read_line(&self) -> String;
//!     fn write_line(&self, line: String);
//! }
//!
//! struct Greeter<C: Console>(C);
//!
//! impl<C: Console> Run for Greeter<C> {
//!     async fn step(&mut self) -> ControlFlow<()> {
//!         let name = self.0.read_line().await;
//!         self.0.write_line(format!("Hello, {name}!"));
//!         ControlFlow::Break(())
//!     }
//! }
//!
//! // A mock: scripted input, recorded output, every future ready.
//! struct Mock(RefCell<Vec<String>>);
//!
//! impl Console for &Mock {
//!     async fn read_line(&self) -> String { "bob".into() }
//!     fn write_line(&self, line: String) { self.0.borrow_mut().push(line); }
//! }
//!
//! let mock = Mock(RefCell::new(Vec::new()));
//! run_now(Greeter(&mock).run());
//! assert_eq!(mock.0.borrow().as_slice(), ["Hello, bob!"]);
//! ```

use core::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

/// Poll `future` once with a no-op waker and return its output.
///
/// This is not an executor. It is for futures that are ready on the first
/// poll — a routine against a mock whose every method returns immediately —
/// and it says so loudly when that assumption fails.
///
/// # Panics
///
/// If the future returns `Pending`. With no waker, nothing could ever wake
/// it; the cause is a wait that needs a real runtime (a `tokio::time::sleep`,
/// a channel) reaching a test that meant to mock it.
#[expect(
    clippy::panic,
    reason = "a Pending future here is a test bug, and a bare panic is the clearest report"
)]
pub fn run_now<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);

    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!(
            "run_now: the future returned Pending. Every wait in a routine under test \
             must resolve at once; a native future (a sleep, a channel) is leaking into \
             a test context, or the routine awaited something no context provides."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_futures_complete() {
        assert_eq!(run_now(async { 1 + 1 }), 2);
    }

    #[test]
    #[should_panic(expected = "returned Pending")]
    fn pending_futures_are_reported() {
        run_now(core::future::pending::<()>());
    }
}
