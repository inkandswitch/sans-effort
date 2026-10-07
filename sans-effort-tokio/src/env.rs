//! The environment, from the process.

use sans_effort_effects::env::Var;

/// `Var` as `std::env::var`. A value that is not UTF-8 reads as unset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokioEnv;

impl Var for TokioEnv {
    async fn var(&self, name: String) -> Option<String> {
        std::env::var(name).ok()
    }
}
