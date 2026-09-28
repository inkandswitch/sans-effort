//! The environment: named configuration values.

use crate::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};
use alloc::string::String;
use core::future::Future;
use sans_effort_core::boundary::codec::{Decode, DecodeError, Encode, Reader, Writer};

/// Read an environment variable.
///
/// Infallible: an unset variable is `None`, not an error. A host whose value
/// is not UTF-8 answers `None` too — a routine could do nothing else with it.
pub trait Var {
    /// The value of `name`, or `None` if it is unset.
    fn var(&self, name: String) -> impl Future<Output = Option<String>> + Send;
}

impl<C: AsCtx + Sync> Var for C
where
    C::Vocabulary: From<Asked<VarEffect>> + Send,
{
    async fn var(&self, name: String) -> Option<String> {
        self.ctx().ask(VarEffect(name)).await
    }
}

/// The value of a variable. Awaits an `Option<String>`, which crosses as
/// `bytes`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VarEffect(pub String);

impl Ask for VarEffect {
    type Reply = Option<String>;
}

/// The name, as a `str`.
impl Encode for VarEffect {
    fn encode(&self, w: &mut Writer) {
        w.str(&self.0);
    }
}

impl Decode for VarEffect {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.str().map(VarEffect)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        bolero::check!().with_type::<String>().for_each(|name| {
            let effect = VarEffect(name.clone());
            assert_eq!(VarEffect::from_bytes(&effect.to_bytes()), Ok(effect));
        });
    }
}
