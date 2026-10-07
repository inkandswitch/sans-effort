//! Time: sleeping on tokio's timer, and the system's wall clock.

use core::time::Duration;
use sans_effort_effects::time::{Now, Sleep, UnixTime};
use std::time::SystemTime;

/// `Sleep` as `tokio::time::sleep`, and `Now` as the system clock.
///
/// Like every component here, nothing happens until the future is polled: a
/// sleep starts when it is first awaited, and `now` reads the clock then, as
/// they would under a reifying context.
///
/// Under tokio's paused clock (`#[tokio::test(start_paused = true)]`), a
/// sleep advances virtual time and costs no wall time. The wall clock is not
/// paused: `now` reads the system's time either way.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioClock;

impl Sleep for TokioClock {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// A clock set before 1970 reads as the epoch itself.
impl Now for TokioClock {
    async fn now(&self) -> UnixTime {
        let since_epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        UnixTime::from_since_epoch(since_epoch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testresult::TestResult;

    /// A sleep starts when it is awaited, as a reifying context's would — not
    /// when `sleep` is called.
    #[tokio::test(start_paused = true)]
    async fn a_sleep_starts_when_awaited() {
        let sleep = TokioClock.sleep(Duration::from_millis(50));
        tokio::time::advance(Duration::from_millis(30)).await;
        let start = tokio::time::Instant::now();
        sleep.await;
        assert_eq!(start.elapsed(), Duration::from_millis(50));
    }

    /// Making a sleep needs no runtime; only awaiting it does.
    #[test]
    fn a_sleep_can_be_made_outside_a_runtime() {
        drop(TokioClock.sleep(Duration::from_millis(50)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_sleep_advances_virtual_time() {
        let start = tokio::time::Instant::now();
        TokioClock.sleep(Duration::from_millis(50)).await;
        assert_eq!(start.elapsed(), Duration::from_millis(50));
    }

    #[tokio::test]
    async fn now_is_the_system_time() -> TestResult {
        let since_epoch = || SystemTime::now().duration_since(SystemTime::UNIX_EPOCH);
        let before = since_epoch()?;
        let now = TokioClock.now().await.since_epoch();
        let after = since_epoch()?;
        assert!(
            before <= now && now <= after,
            "{before:?} <= {now:?} <= {after:?}"
        );
        Ok(())
    }
}
