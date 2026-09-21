//! The capabilities a context may offer. One trait per verb, each one thing
//! a routine can do to, or wait on from, its environment — and, under a
//! reifying context, each one request type on the wire.
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

/// Wait for a duration.
pub trait Sleep {
    /// Return after `duration` has passed.
    async fn sleep(&self, duration: Duration);
}

/// Count a greeting.
pub trait Count {
    /// One more greeting; how many so far, including this one.
    async fn count(&self) -> u64;
}

/// Look a name up.
pub trait Lookup {
    /// The greeting word for `name`.
    async fn lookup(&self, name: String) -> String;
}

/// Read a line.
pub trait ReadLine {
    /// The next line.
    async fn read_line(&self) -> String;
}

/// Write a line. Fire-and-forget, so not `async`.
pub trait WriteLine {
    /// Show `line`.
    fn write(&self, line: String);
}
