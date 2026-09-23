//! Time, from tokio's timer.

use core::time::Duration;
use sans_effort_effects::time::Sleep;

/// `Sleep` as `tokio::time::sleep`.
///
/// Under tokio's paused clock (`#[tokio::test(start_paused = true)]`), a
/// sleep advances virtual time and costs no wall time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioClock;

impl Sleep for TokioClock {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
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
}
