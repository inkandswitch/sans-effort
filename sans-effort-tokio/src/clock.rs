//! Time: sleeping on tokio's timer, and the system's wall clock.

use core::{future::Future, time::Duration};
use sans_effort_effects::time::{Now, Sleep, UnixTime};
use std::time::SystemTime;

/// `Sleep` as `tokio::time::sleep`, and `Now` as the system clock.
///
/// Under tokio's paused clock (`#[tokio::test(start_paused = true)]`), a
/// sleep advances virtual time and costs no wall time. The wall clock is not
/// paused: `now` reads the system's time either way.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioClock;

impl Sleep for TokioClock {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
        tokio::time::sleep(duration)
    }
}

/// A clock set before 1970 reads as the epoch itself.
impl Now for TokioClock {
    fn now(&self) -> impl Future<Output = UnixTime> + Send {
        let since_epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        core::future::ready(UnixTime::from_since_epoch(since_epoch))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_sleep_advances_virtual_time() {
        let start = tokio::time::Instant::now();
        TokioClock.sleep(Duration::from_millis(50)).await;
        assert_eq!(start.elapsed(), Duration::from_millis(50));
    }

    #[tokio::test]
    async fn now_is_after_this_code_was_written() {
        let written = Duration::from_secs(1_790_000_000);
        assert!(TokioClock.now().await.since_epoch() > written);
    }
}
