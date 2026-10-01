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
    use sans_effort_core::testing::run_now;

    /// Exactly the bytes asked for, and filled: 16 zero bytes from a real
    /// entropy source would happen once in 2^128 draws.
    #[test]
    fn exactly_the_bytes_asked_for_and_filled() {
        bolero::check!().with_type::<u16>().for_each(|len| {
            let bytes = run_now(TokioRandom.random_bytes((*len).into()));
            assert_eq!(bytes.len(), usize::from(*len));
            if *len >= 16 {
                assert!(bytes.iter().any(|b| *b != 0), "the buffer was filled");
            }
        });
    }
}
