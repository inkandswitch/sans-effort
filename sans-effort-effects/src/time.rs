//! Time: waiting.

use crate::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};
use core::{future::Future, time::Duration};
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};

/// Wait for a duration.
///
/// Infallible: nothing a routine could act on makes a sleep fail. A host may
/// answer at once, after a real delay, or on a virtual clock.
pub trait Sleep {
    /// Return after `duration` has passed.
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send;
}

impl<C: AsCtx + Sync> Sleep for C
where
    C::Vocabulary: From<Asked<SleepEffect>> + Send,
{
    async fn sleep(&self, duration: Duration) {
        self.ctx().ask(SleepEffect(duration)).await;
    }
}

/// Wake after a duration. Awaits `unit`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SleepEffect(pub Duration);

impl SleepEffect {
    /// The duration in whole milliseconds, as it crosses the wire; saturates
    /// at `u64::MAX`.
    #[must_use]
    pub fn millis(&self) -> u64 {
        u64::try_from(self.0.as_millis()).unwrap_or(u64::MAX)
    }
}

impl Ask for SleepEffect {
    type Reply = ();
}

/// Whole milliseconds, `u64`. Sub-millisecond precision does not cross.
impl Encode for SleepEffect {
    fn encode(&self, w: &mut Writer) {
        w.u64(self.millis());
    }
}

impl Decode for SleepEffect {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.u64().map(|ms| SleepEffect(Duration::from_millis(ms)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_milliseconds_round_trip() {
        bolero::check!().with_type::<u64>().for_each(|ms| {
            let sleep = SleepEffect(Duration::from_millis(*ms));
            assert_eq!(SleepEffect::from_bytes(&sleep.to_bytes()), Ok(sleep));
        });
    }

    #[test]
    fn durations_past_u64_millis_saturate() {
        assert_eq!(SleepEffect(Duration::MAX).millis(), u64::MAX);
    }
}
