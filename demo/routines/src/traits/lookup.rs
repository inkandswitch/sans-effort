//! Looking up the greeting word for a name.

pub mod effect;

use alloc::string::String;
use core::future::Future;
use sans_effort::{ask::Asked, ctx::AsCtx};

/// Look a name up.
pub trait Lookup {
    /// The greeting word for `name`.
    fn lookup(&self, name: String) -> impl Future<Output = String> + Send;
}

impl<C: AsCtx + Sync> Lookup for C
where
    C::Vocabulary: From<Asked<effect::Lookup>> + Send,
{
    async fn lookup(&self, name: String) -> String {
        self.ctx().ask(effect::Lookup(name)).await
    }
}
