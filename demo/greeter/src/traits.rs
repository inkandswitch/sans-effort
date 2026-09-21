//! The capabilities a context may offer. Each is one thing the greeter can
//! do to, or wait on from, its environment.
//!
//! Written as `async fn`. rustc warns that this leaves auto traits unstated;
//! that is the intent — `Send` is the poller's concern, not the routine's,
//! and the compiler infers it at each spawn site from the concrete context.

#![allow(
    async_fn_in_trait,
    reason = "Send is decided by the concrete context at the spawn site, not here"
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
