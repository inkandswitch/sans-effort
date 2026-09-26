//! What [`Count`](super::Count) records under a reifying context.

use sans_effort::ask::Ask;

/// How many greetings so far. Awaits a `u64`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Count;

impl Ask for Count {
    type Reply = u64;
}
