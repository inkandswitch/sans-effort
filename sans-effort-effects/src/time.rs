//! Time: waiting, and telling the time.
//!
//! Both are asks, so a host decides what time it is: the real clock, or —
//! for a test or a replay — a virtual one that makes a run repeatable.

use crate::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};
use core::{future::Future, time::Duration};
use sans_effort_core::{
    boundary::codec::{Decode, DecodeError, Encode, Reader, Writer},
    reply::Answer,
};

/// Wait for a duration.
///
/// Infallible: nothing a routine could act on makes a sleep fail. A host may
/// answer at once, after a real delay, or on a virtual clock.
///
/// # Cancellation
///
/// _Retractable:_ an abandoned sleep loses nothing. A host should stop its
/// timer, or it waits on a timer nobody needs.
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

/// Tell the wall-clock time.
///
/// Infallible, like [`Sleep`]. Wall-clock time can jump — a host's clock may
/// be adjusted — so measure a pause with [`Sleep`], not by subtracting two
/// readings.
///
/// # Cancellation
///
/// _Retractable:_ an abandoned reading is discarded, and loses nothing.
pub trait Now {
    /// The current time.
    fn now(&self) -> impl Future<Output = UnixTime> + Send;
}

impl<C: AsCtx + Sync> Now for C
where
    C::Vocabulary: From<Asked<NowEffect>> + Send,
{
    async fn now(&self) -> UnixTime {
        self.ctx().ask(NowEffect).await
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

/// The current time. Awaits a [`UnixTime`], which crosses as a `u64`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NowEffect;

impl Ask for NowEffect {
    type Reply = UnixTime;
}

/// No fields.
impl Encode for NowEffect {
    fn encode(&self, _: &mut Writer) {}
}

impl Decode for NowEffect {
    fn decode(_: &mut Reader<'_>) -> Result<Self, DecodeError> {
        Ok(NowEffect)
    }
}

/// A wall-clock time: how long after the Unix epoch (1970-01-01 UTC).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct UnixTime(Duration);

impl UnixTime {
    /// The time `since_epoch` after the Unix epoch.
    #[must_use]
    pub const fn from_since_epoch(since_epoch: Duration) -> Self {
        Self(since_epoch)
    }

    /// How long after the Unix epoch.
    #[must_use]
    pub const fn since_epoch(self) -> Duration {
        self.0
    }
}

/// Whole nanoseconds since the epoch, `u64`: enough until the year 2554, and
/// saturating after.
impl Answer for UnixTime {
    type Wire = u64;

    fn into_wire(self) -> u64 {
        u64::try_from(self.0.as_nanos()).unwrap_or(u64::MAX)
    }

    fn from_wire(nanos: u64) -> Result<Self, DecodeError> {
        Ok(Self(Duration::from_nanos(nanos)))
    }

    fn check(_: &u64) -> Result<(), DecodeError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_times_round_trip_in_whole_nanoseconds() {
        bolero::check!().with_type::<u64>().for_each(|nanos| {
            let time = UnixTime::from_since_epoch(Duration::from_nanos(*nanos));
            assert_eq!(UnixTime::from_wire(time.into_wire()), Ok(time));
        });
    }

    #[test]
    fn unix_times_past_u64_nanos_saturate() {
        assert_eq!(
            UnixTime::from_since_epoch(Duration::MAX).into_wire(),
            u64::MAX
        );
    }

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
