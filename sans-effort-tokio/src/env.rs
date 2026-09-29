//! The environment, from the process.

use core::future::Future;
use sans_effort_effects::env::Var;

/// `Var` as `std::env::var`. A value that is not UTF-8 reads as unset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioEnv;

impl Var for TokioEnv {
    fn var(&self, name: String) -> impl Future<Output = Option<String>> + Send {
        core::future::ready(std::env::var(name).ok())
    }
}
