//! The demo's own capabilities. `Count` and `Lookup` are specific to the
//! greeter; `Sleep`, `ReadLine`, and `WriteLine` come from
//! `sans-effort-effects`, the standard library, because nearly every routine
//! wants them. One trait per verb, each one thing a routine can do to, or
//! wait on from, its environment.
//!
//! Laid out like a module of the standard library: each trait, its effect
//! (in [`effect`]), and its impl for the reifying
//! [`Ctx`] live together. They must: the orphan
//! rule allows `impl Count for Ctx<E>` only in the crate that defines `Count`
//! or the one that defines `Ctx`. The routines themselves (`greeter`,
//! `fanout`, `ticker`) use only the traits.
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

pub mod effect;

use alloc::string::String;
use sans_effort::request::Asked;
use sans_effort_effects::Ctx;

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

impl<E: From<Asked<effect::Count>>> Count for Ctx<E> {
    async fn count(&self) -> u64 {
        self.request(effect::Count).await
    }
}

impl<E: From<Asked<effect::Lookup>>> Lookup for Ctx<E> {
    async fn lookup(&self, name: String) -> String {
        self.request(effect::Lookup(name)).await
    }
}
