//! Time: waiting.

pub mod effect;

use crate::{ctx::Ctx, request::Asked};
use core::time::Duration;

/// Wait for a duration.
///
/// Infallible: nothing a routine could act on makes a sleep fail. A host may
/// answer at once, after a real delay, or on a virtual clock.
pub trait Sleep {
    /// Return after `duration` has passed.
    async fn sleep(&self, duration: Duration);
}

impl<E: From<Asked<effect::Sleep>>> Sleep for Ctx<E> {
    async fn sleep(&self, duration: Duration) {
        self.request(effect::Sleep(duration)).await;
    }
}
