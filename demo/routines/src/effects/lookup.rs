//! Looking up the greeting word for a name: the trait, its reifying impl,
//! and its effect.

use alloc::string::String;
use core::future::Future;
use sans_effort::{
    ask::{Ask, Asked},
    ctx::AsCtx,
};

/// Look a name up.
///
/// # Cancellation
///
/// _Retractable:_ an abandoned lookup is discarded, and loses nothing.
pub trait Lookup {
    /// The greeting word for `name`.
    fn lookup(&self, name: String) -> impl Future<Output = String> + Send;
}

impl<C: AsCtx + Sync> Lookup for C
where
    C::Vocabulary: From<Asked<LookupEffect>> + Send,
{
    async fn lookup(&self, name: String) -> String {
        self.ctx().ask(LookupEffect(name)).await
    }
}

/// What [`Lookup`] records under a reifying context: the greeting word for
/// a name. Awaits a `str`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupEffect(pub String);

impl Ask for LookupEffect {
    type Reply = String;
}
