//! Counting greetings.

pub mod effect;

use core::future::Future;
use sans_effort::{ask::Asked, ctx::AsCtx};

/// Count a greeting.
pub trait Count {
    /// One more greeting; how many so far, including this one.
    fn count(&self) -> impl Future<Output = u64> + Send;
}

impl<C: AsCtx + Sync> Count for C
where
    C::Vocabulary: From<Asked<effect::Count>> + Send,
{
    async fn count(&self) -> u64 {
        self.ctx().ask(effect::Count).await
    }
}
