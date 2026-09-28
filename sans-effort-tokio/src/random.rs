//! Randomness, from the operating system.

use core::future::Future;
use sans_effort_effects::random::Random;

/// `Random` from the operating system's entropy source, via `getrandom`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioRandom;

impl Random for TokioRandom {
    #[expect(
        clippy::expect_used,
        reason = "the trait promises bytes, and a host with no entropy source has nothing a routine could act on"
    )]
    fn random_bytes(&self, len: u32) -> impl Future<Output = Vec<u8>> + Send {
        let mut bytes = vec![0; usize::try_from(len).unwrap_or(usize::MAX)];
        getrandom::fill(&mut bytes).expect("the operating system's entropy source");
        core::future::ready(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exactly_the_bytes_asked_for() {
        assert_eq!(TokioRandom.random_bytes(0).await.len(), 0);
        assert_eq!(TokioRandom.random_bytes(33).await.len(), 33);
    }
}
