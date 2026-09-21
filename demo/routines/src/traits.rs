//! The capabilities a context may offer. Each is one thing the greeter can
//! do to, or wait on from, its environment.
//!
//! Written as `async fn`. rustc warns that this leaves auto traits unstated,
//! and it is right about what that costs: nothing generic over `C` can also
//! spawn the routine, because there is no way on stable Rust to write
//! `C::sleep(..): Send`. This library accepts that and never does it — a
//! routine is spawned where its context is concrete, and the compiler then
//! decides `Send` on the real state machine. In exchange, no context is
//! forced to be `Send`: the `Rc`-based test mock in `recording.rs` compiles,
//! and so would a context for a target without atomics. Both halves are
//! pinned in `tests/send.rs`.

#![allow(
    async_fn_in_trait,
    reason = "spawn sites are always concrete (tokio::spawn, Driver::new), so Send is inferred there; a bound here would forbid !Send contexts such as the Rc-based test mock"
)]

use alloc::string::String;
use core::time::Duration;

/// Something that can wait for a duration.
pub trait Clock {
    /// Return after `duration` has passed.
    async fn sleep(&self, duration: Duration);
}

/// Something that counts greetings.
pub trait Counter {
    /// One more greeting; how many so far, including this one.
    async fn count(&self) -> u64;
}

/// Something that knows which greeting suits a name.
pub trait Directory {
    /// The greeting word for `name`.
    async fn lookup(&self, name: String) -> String;
}

/// Something lines arrive from.
pub trait Input {
    /// The next line.
    async fn read_line(&self) -> String;
}

/// Somewhere to print. Fire-and-forget, so not `async`.
pub trait Output {
    /// Show `line`.
    fn write(&self, line: String);
}
