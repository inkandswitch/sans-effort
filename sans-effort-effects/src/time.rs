//! Time: waiting.

pub mod effect;

use crate::{ctx::AsCtx, request::Asked};
use core::{future::Future, time::Duration};

/// Wait for a duration.
///
/// Infallible: nothing a routine could act on makes a sleep fail. A host may
/// answer at once, after a real delay, or on a virtual clock.
pub trait Sleep {
    /// Return after `duration` has passed.
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()>;
}

impl<C: AsCtx> Sleep for C
where
    C::Vocabulary: From<Asked<effect::Sleep>>,
{
    async fn sleep(&self, duration: Duration) {
        self.ctx().request(effect::Sleep(duration)).await;
    }
}
