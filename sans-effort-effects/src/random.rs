//! Randomness, supplied by the host.
//!
//! Random bytes are an ask like any other: a host answers with real entropy,
//! or — for a test or a replay — with bytes it chooses, so a run can be
//! repeated exactly.

use crate::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};
use alloc::vec::Vec;
use core::future::Future;
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};

/// Draw random bytes.
///
/// Infallible: a host that has no entropy has nothing a routine could act
/// on. Whether the bytes are fit for keys is the host's promise, not this
/// trait's.
///
/// # Cancellation
///
/// _Consuming, but nothing a routine needed is lost:_ bytes drawn for an
/// abandoned request are discarded, and the next request draws fresh ones.
pub trait Random {
    /// `len` random bytes.
    fn random_bytes(&self, len: u32) -> impl Future<Output = Vec<u8>> + Send;
}

impl<C: AsCtx + Sync> Random for C
where
    C::Vocabulary: From<Asked<RandomEffect>> + Send,
{
    async fn random_bytes(&self, len: u32) -> Vec<u8> {
        self.ctx().ask(RandomEffect(len)).await
    }
}

/// `len` random bytes. Awaits `bytes`, exactly `len` of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RandomEffect(pub u32);

impl Ask for RandomEffect {
    type Reply = Vec<u8>;
}

/// The length, `u32`.
impl Encode for RandomEffect {
    fn encode(&self, w: &mut Writer) {
        w.u32(self.0);
    }
}

impl Decode for RandomEffect {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.u32().map(RandomEffect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        bolero::check!().with_type::<u32>().for_each(|len| {
            let effect = RandomEffect(*len);
            assert_eq!(RandomEffect::from_bytes(&effect.to_bytes()), Ok(effect));
        });
    }
}
