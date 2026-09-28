//! Counting greetings: the trait, its reifying impl, and its effect.

use core::future::Future;
use sans_effort::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};

/// Count a greeting.
pub trait Count {
    /// One more greeting; how many so far, including this one.
    fn count(&self) -> impl Future<Output = u64> + Send;
}

impl<C: AsCtx + Sync> Count for C
where
    C::Vocabulary: From<Asked<CountEffect>> + Send,
{
    async fn count(&self) -> u64 {
        self.ctx().ask(CountEffect).await
    }
}

/// What [`Count`] records under a reifying context: how many greetings so
/// far. Awaits a `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountEffect;

impl Ask for CountEffect {
    type Reply = u64;
}
