//! The demo's own capabilities. `Count` and `Lookup` are specific to the
//! greeter; `Sleep`, `ReadLine`, and `WriteLine` come from
//! `sans-effort-effects`, the standard library, because nearly every routine
//! wants them. One trait per verb, each one thing a routine can do to, or
//! wait on from, its environment.
//!
//! Laid out like a module of the standard library: each trait, its effect
//! (in [`effect`]), and its reifying impl live together. The impl is written
//! over [`AsCtx`], so it holds for [`Ctx`](sans_effort_effects::ctx::Ctx) and for
//! any newtype wrapping one. The routines themselves (`greeter`, `fanout`,
//! `ticker`) use only the traits.
//!
//! Methods are spelled `fn … -> impl Future`, as `sans_effort::run::Run`
//! spells `step`; implementors write `async fn`. Neither says anything about
//! `Send`, and nothing generic over `C` can add it later — so nothing here
//! both takes any `C` and spawns the routine. A routine is spawned where its
//! context is concrete, and the compiler decides `Send` on the real state
//! machine. In exchange, no context is forced to be `Send`: the `Rc`-based
//! test mock in `recording.rs` compiles, and so would a context for a target
//! without atomics. Both halves are pinned in `tests/send.rs`.

pub mod effect;

use alloc::string::String;
use core::future::Future;
use sans_effort_effects::{ctx::AsCtx, request::Asked};

/// Count a greeting.
pub trait Count {
    /// One more greeting; how many so far, including this one.
    fn count(&self) -> impl Future<Output = u64> + Send;
}

/// Look a name up.
pub trait Lookup {
    /// The greeting word for `name`.
    fn lookup(&self, name: String) -> impl Future<Output = String> + Send;
}

impl<C: AsCtx + Sync> Count for C
where
    C::Vocabulary: From<Asked<effect::Count>> + Send,
{
    async fn count(&self) -> u64 {
        self.ctx().request(effect::Count).await
    }
}

impl<C: AsCtx + Sync> Lookup for C
where
    C::Vocabulary: From<Asked<effect::Lookup>> + Send,
{
    async fn lookup(&self, name: String) -> String {
        self.ctx().request(effect::Lookup(name)).await
    }
}
